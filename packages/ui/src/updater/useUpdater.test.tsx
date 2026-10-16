// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { useUpdater } from "./useUpdater";

const m = vi.hoisted(() => ({
  isTauri: vi.fn(),
  check: vi.fn(),
  relaunch: vi.fn(),
  getVersion: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: m.isTauri }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: m.getVersion }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: m.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: m.relaunch }));

const FIRST = 15_000;
const SIX_HOURS = 6 * 60 * 60 * 1000;

type Evento =
  | { event: "Started"; data: { contentLength?: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };

function fakeUpdate(download?: (cb: (e: Evento) => void) => Promise<void>) {
  return { version: "0.2.0", body: "notas", downloadAndInstall: vi.fn(download ?? (() => Promise.resolve())) };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubEnv("PROD", true);
  m.isTauri.mockReturnValue(true);
  m.getVersion.mockResolvedValue("0.1.0");
  m.check.mockReset().mockResolvedValue(null);
  m.relaunch.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllEnvs();
});

const advance = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms));

test("fora do Tauri fica nao-suportado e nunca procura", async () => {
  m.isTauri.mockReturnValue(false);
  const { result } = renderHook(() => useUpdater());
  await advance(SIX_HOURS * 2);
  expect(result.current.state).toEqual({ status: "nao-suportado" });
  expect(await result.current.verify(true)).toEqual({ resultado: "ignorado" });
  expect(m.check).not.toHaveBeenCalled();
});

test("em desenvolvimento (tauri dev) não procura", async () => {
  vi.stubEnv("PROD", false);
  const { result } = renderHook(() => useUpdater());
  await advance(FIRST * 2);
  expect(result.current.state.status).toBe("nao-suportado");
  expect(m.check).not.toHaveBeenCalled();
});

test("primeira checagem 15 s após abrir e depois a cada 6 h", async () => {
  renderHook(() => useUpdater());
  await advance(FIRST - 1);
  expect(m.check).not.toHaveBeenCalled();
  await advance(1);
  expect(m.check).toHaveBeenCalledTimes(1);
  await advance(SIX_HOURS);
  expect(m.check).toHaveBeenCalledTimes(2);
});

test("sem agendamento: uma checagem ao montar e nenhum timer", async () => {
  renderHook(() => useUpdater({ schedule: false }));
  await advance(0);
  expect(m.check).toHaveBeenCalledTimes(1);
  await advance(SIX_HOURS * 2);
  expect(m.check).toHaveBeenCalledTimes(1);
});

test("lê a versão instalada", async () => {
  const { result } = renderHook(() => useUpdater());
  await advance(0);
  expect(result.current.version).toBe("0.1.0");
});

test("checagem automática com update fica disponivel", async () => {
  const update = fakeUpdate();
  m.check.mockResolvedValue(update);
  const { result } = renderHook(() => useUpdater());
  await advance(FIRST);
  expect(result.current.state).toEqual({ status: "disponivel", update });
  expect(result.current.lastCheck).not.toBeNull();
});

test("checagem automática que falha fica em silêncio", async () => {
  m.check.mockRejectedValue(new Error("offline"));
  const { result } = renderHook(() => useUpdater());
  await advance(FIRST);
  expect(m.check).toHaveBeenCalledTimes(1);
  expect(result.current.state).toEqual({ status: "ocioso" });
});

test("checagem manual mostra atualizado e mostra o erro", async () => {
  const { result } = renderHook(() => useUpdater());
  await act(() => result.current.verify(true));
  expect(result.current.state).toEqual({ status: "atualizado" });

  m.check.mockRejectedValue(new Error("offline"));
  await act(() => result.current.verify(true));
  expect(result.current.state).toEqual({
    status: "erro",
    mensagem: "Não foi possível procurar atualizações: offline",
  });
});

test("instalar baixa com progresso e reinicia", async () => {
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  const update = fakeUpdate(async (cb) => {
    cb({ event: "Started", data: { contentLength: 100 } });
    cb({ event: "Progress", data: { chunkLength: 40 } });
    cb({ event: "Progress", data: { chunkLength: 20 } });
    await gate;
    cb({ event: "Finished" });
  });
  const { result } = renderHook(() => useUpdater());
  let installing!: Promise<void>;
  await act(async () => {
    installing = result.current.install(update as never);
  });
  expect(result.current.state).toMatchObject({ status: "baixando", baixado: 60, total: 100 });
  expect(m.relaunch).not.toHaveBeenCalled();

  await act(async () => {
    release();
    await installing;
  });
  expect(m.relaunch).toHaveBeenCalledTimes(1);
  expect(result.current.state.status).toBe("reiniciando");
});

test("falha na instalação vira erro e guarda a atualização para tentar de novo", async () => {
  const update = fakeUpdate(() => Promise.reject(new Error("assinatura inválida")));
  const { result } = renderHook(() => useUpdater());
  await act(() => result.current.install(update as never));
  expect(m.relaunch).not.toHaveBeenCalled();
  expect(result.current.state).toEqual({ status: "erro", mensagem: "A atualização falhou: assinatura inválida", update });
});
