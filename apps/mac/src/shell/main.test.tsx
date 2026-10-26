// @vitest-environment jsdom
import type { Config } from "@relay/protocol-types";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { Main } from "./index";

const bridge = vi.hoisted(() => ({
  listeners: new Map<string, (p: unknown) => void>(),
  invoke: vi.fn(),
  navigate: undefined as undefined | ((e: { payload: string }) => void),
  peer: "Desktop" as string | null,
}));

vi.mock("@relay/protocol-types", () => ({
  listenEvent: async (name: string, cb: (p: unknown) => void) => {
    const prev = bridge.listeners.get(name);
    bridge.listeners.set(name, (p) => {
      prev?.(p);
      cb(p);
    });
    return () => bridge.listeners.delete(name);
  },
  invokeCommand: bridge.invoke,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, cb: (e: { payload: string }) => void) => {
    if (name === "relay:navigate") bridge.navigate = cb;
    return () => undefined;
  },
}));
// O editor tem testes próprios; aqui só importa que a seção "Telas" o monta.
vi.mock("../editor", () => ({ Editor: () => <p>editor de telas</p> }));

const CONFIG = {
  schema_version: 1,
  network: { ports: { control: 1, input: 2, audio_mac_to_pc: 4, transfer: 5 }, manual_address: null },
  transition: { dwell_ms: 150, edge_resistance_pt: 8, require_modifier: null, corner_dead_zone_mm: 5 },
  keymap: { cmd_tap_timeout_ms: 300, rules: [], app_exceptions: [], reverse_modifiers: { ctrl_cmd: false } },
  hotkeys: [],
  clipboard: { enabled: true, mode: "on_switch", max_bytes: 10 * 1024 * 1024, formats: ["Text"] },
  audio: { jitter_target_ms: 40, mac_input_id: null },
  layout: {},
  general: { launch_at_login: false },
} as unknown as Config;

const PC = { name: "DESKTOP-PC", addrs: ["192.168.0.20"], port: 7000, device_id: "ab", protocol_version: 1 };

beforeEach(() => {
  bridge.peer = "Desktop";
  bridge.listeners.clear();
  bridge.invoke.mockReset();
  bridge.invoke.mockImplementation(async (name: string) => {
    if (name === "get_status")
      return { status: "Connected", peer_name: bridge.peer, focus: "Mac", locked: false, paused: false, secure_input: false, pc_unreachable: false, clipboard_sync: false, audio: { mac_to_pc: false }, peer_version: null, version_sync: "InSync" };
    if (name === "get_displays") return { mac: [], windows: [] };
    if (name === "get_config") return structuredClone(CONFIG);
    if (name === "list_discovered") return [PC];
    return undefined;
  });
});
afterEach(cleanup);

test("abre na visão geral e a sidebar troca de seção", async () => {
  render(<Main />);
  expect(await screen.findByRole("heading", { name: "Visão geral" })).toBeTruthy();
  expect(await screen.findByText(/Sem métricas/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Telas" }));
  expect(await screen.findByText("editor de telas")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Telas" }).getAttribute("aria-current")).toBe("page");
});

test("relay:navigate leva à seção pedida e ignora id desconhecido", async () => {
  render(<Main />);
  await screen.findByRole("heading", { name: "Visão geral" });
  await waitFor(() => expect(bridge.navigate).toBeDefined());
  bridge.navigate?.({ payload: "inexistente" });
  expect(screen.getByRole("heading", { name: "Visão geral" })).toBeTruthy();
  bridge.navigate?.({ payload: "general" });
  expect(await screen.findByRole("heading", { name: "Geral" })).toBeTruthy();
  expect(await screen.findByRole("heading", { name: "Início" })).toBeTruthy();
});

const box = (n: number) => screen.getByLabelText(`Dígito ${n}`) as HTMLInputElement;
const typeCode = (code: string) => [...code].forEach((d, i) => fireEvent.change(box(i + 1), { target: { value: d } }));
const pasteCode = (text: string) => fireEvent.paste(box(1), { clipboardData: { getData: () => text } });
const pairCalls = () => bridge.invoke.mock.calls.filter(([n]) => n === "pair_with_code");

async function openFirstUse() {
  bridge.peer = null;
  render(<Main />);
  await screen.findByRole("heading", { name: "Conecte seu PC" });
  await screen.findByRole("button", { name: /DESKTOP-PC/ });
}

test("sem PC pareado a janela abre na tela de pareamento, com o único PC já selecionado", async () => {
  await openFirstUse();
  expect(screen.queryByRole("heading", { name: "Visão geral" })).toBeNull();
  expect(screen.queryByRole("navigation", { name: "Seções" })).toBeNull();
  expect(screen.getByRole("button", { name: /DESKTOP-PC/ }).getAttribute("aria-pressed")).toBe("true");
  expect(box(1)).toBe(document.activeElement);
});

test("digitar os 6 dígitos envia o pareamento sozinho, com o endereço do PC", async () => {
  await openFirstUse();
  typeCode("12345");
  expect(pairCalls()).toHaveLength(0);
  expect(box(6)).toBe(document.activeElement);
  fireEvent.change(box(6), { target: { value: "6" } });
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("pair_with_code", { code: "123456", address: "192.168.0.20:7000" }));
});

test("colar o código preenche as caixas, ignora o que não é dígito e envia", async () => {
  await openFirstUse();
  pasteCode("12a-3 456");
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("pair_with_code", { code: "123456", address: "192.168.0.20:7000" }));
  fireEvent.change(box(1), { target: { value: "x" } });
  expect(pairCalls()).toHaveLength(1);
});

test("backspace volta uma caixa e apaga o dígito anterior", async () => {
  await openFirstUse();
  typeCode("12");
  fireEvent.keyDown(box(3), { key: "Backspace" });
  expect(box(2).value).toBe("");
  expect(box(1).value).toBe("1");
  expect(box(2)).toBe(document.activeElement);
});

test("código errado mostra o erro e limpa as caixas", async () => {
  await openFirstUse();
  const base = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === "pair_with_code") throw "recusado";
    return base(name, args);
  });
  pasteCode("999999");
  expect((await screen.findByRole("alert")).textContent).toMatch(/incorreto ou expirado/);
  expect(box(1).value).toBe("");
  expect(box(1)).toBe(document.activeElement);
  expect(box(1).getAttribute("aria-invalid")).toBe("true");
});

test("ao parear mostra o sucesso e segue para a visão geral", async () => {
  await openFirstUse();
  const base = bridge.invoke.getMockImplementation()!;
  bridge.invoke.mockImplementation(async (name: string, args?: unknown) => {
    if (name === "pair_with_code") {
      bridge.peer = "DESKTOP-PC";
      bridge.listeners.get("relay:status")?.(await base("get_status"));
    }
    return base(name, args);
  });
  pasteCode("123456");
  expect(await screen.findByRole("heading", { name: "Conectado" })).toBeTruthy();
  await act(async () => void fireEvent.click(screen.getByRole("button", { name: "Começar" })));
  expect(await screen.findByRole("heading", { name: "Visão geral" })).toBeTruthy();
});

test("depois de revogar, o app volta à tela de primeiro uso", async () => {
  render(<Main />);
  await screen.findByRole("heading", { name: "Visão geral" });
  bridge.peer = null;
  await act(async () => bridge.listeners.get("relay:status")?.({ status: "Disconnected", peer_name: null, focus: "Mac", audio: { mac_to_pc: false } }));
  expect(await screen.findByRole("heading", { name: "Conecte seu PC" })).toBeTruthy();
});
