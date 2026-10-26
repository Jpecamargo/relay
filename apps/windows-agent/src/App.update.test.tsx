// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { UpdaterProvider } from "@relay/ui";
import { App } from "./App";

const m = vi.hoisted(() => ({
  invoke: vi.fn(),
  check: vi.fn(),
  relaunch: vi.fn(),
  tauriListeners: new Map<string, (e: { payload?: unknown }) => void>(),
}));

vi.mock("@relay/protocol-types", () => ({
  invokeCommand: m.invoke,
  listenEvent: async () => () => undefined,
}));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "0.1.8" }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, cb: (e: { payload?: unknown }) => void) => {
    m.tauriListeners.set(name, cb);
    return () => m.tauriListeners.delete(name);
  },
}));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: m.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: m.relaunch }));

const update = { version: "0.2.0", date: "2026-10-20 12:00:00.0 +00:00:00", body: "- novidades", downloadAndInstall: vi.fn(async () => undefined) };

beforeEach(() => {
  vi.stubEnv("PROD", true);
  m.tauriListeners.clear();
  m.check.mockReset().mockResolvedValue(null);
  m.relaunch.mockReset().mockResolvedValue(undefined);
  update.downloadAndInstall.mockClear();
  m.invoke.mockReset().mockImplementation(async (name: string) => {
    switch (name) {
      case "agent_get_status":
        return { status: "Connected", peer_name: "MacBook", focus: "Peer", peer_version: "0.2.0", version_sync: "LocalBehind" };
      case "agent_get_service_status":
        return { installed: true, running: true, helper_active: true, vbcable_detected: false };
      case "agent_get_config":
        return { audio: { jitter_target_ms: 20, mac_input_id: null }, clipboard: { enabled: true }, general: { launch_at_login: false } };
      case "agent_new_pairing_code":
        return { code: "123456", expires_at: Date.now() + 120_000 };
      default:
        return undefined;
    }
  });
});
afterEach(() => {
  cleanup();
  vi.unstubAllEnvs();
});

const open = () =>
  render(
    <UpdaterProvider schedule={false}>
      <App />
    </UpdaterProvider>,
  );

test("pedido de atualização vindo do Mac procura na hora e abre o modal Atualizar e reiniciar", async () => {
  m.check.mockResolvedValue(update);
  open();
  await act(async () => {});
  expect(screen.queryByRole("dialog")).toBeNull();

  await act(async () => m.tauriListeners.get("relay:open-update")!({ payload: null }));
  expect(screen.getByRole("dialog").textContent).toContain("0.2.0");

  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Atualizar e reiniciar" })));
  expect(update.downloadAndInstall).toHaveBeenCalledTimes(1);
  expect(m.relaunch).toHaveBeenCalledTimes(1);
});

test("pedido sem versão nova disponível não abre modal", async () => {
  open();
  await act(async () => {});
  await act(async () => m.tauriListeners.get("relay:open-update")!({ payload: null }));
  expect(screen.queryByRole("dialog")).toBeNull();
});
