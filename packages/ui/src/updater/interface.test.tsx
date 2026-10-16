// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { UpdateAbout } from "./UpdateAbout";
import { UpdaterProvider } from "./UpdaterProvider";

const m = vi.hoisted(() => ({
  check: vi.fn(),
  relaunch: vi.fn(),
  listeners: new Map<string, () => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "0.1.0" }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, cb: () => void) => {
    m.listeners.set(name, cb);
    return () => m.listeners.delete(name);
  },
}));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: m.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: m.relaunch }));

function fakeUpdate(download: () => Promise<void> = () => Promise.resolve()) {
  return {
    version: "0.2.0",
    date: "2026-09-15 19:00:00.0 +00:00:00",
    body: "- corrige o áudio",
    downloadAndInstall: vi.fn(download),
  };
}

beforeEach(() => {
  vi.stubEnv("PROD", true);
  m.check.mockReset().mockResolvedValue(null);
  m.relaunch.mockReset().mockResolvedValue(undefined);
  m.listeners.clear();
});
afterEach(() => {
  cleanup();
  vi.unstubAllEnvs();
});

const setup = () =>
  render(
    <UpdaterProvider schedule={false}>
      <UpdateAbout />
    </UpdaterProvider>,
  );

test("Sobre mostra a versão e a atualização manual", async () => {
  setup();
  await act(async () => {});
  expect(screen.getByText("0.1.0")).toBeTruthy();
  m.check.mockResolvedValue(null);
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Procurar atualizações" })));
  expect(screen.getByText("Você está na versão mais recente.")).toBeTruthy();
});

test("com atualização: modal com notas; só baixa ao clicar e reinicia", async () => {
  const update = fakeUpdate();
  m.check.mockResolvedValue(update);
  setup();
  await act(async () => {});
  fireEvent.click(screen.getByRole("button", { name: "Ver atualização" }));
  const dialog = screen.getByRole("dialog");
  expect(dialog.textContent).toContain("0.2.0");
  expect(dialog.textContent).toContain("15/09/2026");
  expect(dialog.textContent).toContain("- corrige o áudio");
  expect(update.downloadAndInstall).not.toHaveBeenCalled();

  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Atualizar e reiniciar" })));
  expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
  expect(m.relaunch).toHaveBeenCalledTimes(1);
});

test("Depois fecha o modal; erro na instalação oferece tentar de novo", async () => {
  m.check.mockResolvedValue(fakeUpdate(() => Promise.reject(new Error("falhou"))));
  setup();
  await act(async () => {});
  fireEvent.click(screen.getByRole("button", { name: "Ver atualização" }));
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Atualizar e reiniciar" })));
  expect(within(screen.getByRole("dialog")).getByRole("alert").textContent).toContain("A atualização falhou: falhou");
  expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Depois" }));
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("evento relay:open-update abre o modal quando há atualização", async () => {
  m.check.mockResolvedValue(fakeUpdate());
  setup();
  await act(async () => {});
  await act(async () => m.listeners.get("relay:open-update")!());
  expect(screen.getByRole("dialog")).toBeTruthy();
});
