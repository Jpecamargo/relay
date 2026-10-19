// @vitest-environment jsdom
import type { Config } from "@relay/protocol-types";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { Main } from "./index";

const bridge = vi.hoisted(() => ({
  listeners: new Map<string, (p: unknown) => void>(),
  invoke: vi.fn(),
  navigate: undefined as undefined | ((e: { payload: string }) => void),
}));

vi.mock("@relay/protocol-types", () => ({
  listenEvent: async (name: string, cb: (p: unknown) => void) => {
    bridge.listeners.set(name, cb);
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
  network: { ports: { control: 1, input: 2, audio_pc_to_mac: 3, audio_mac_to_pc: 4, transfer: 5 }, manual_address: null },
  transition: { dwell_ms: 150, edge_resistance_pt: 8, require_modifier: null, corner_dead_zone_mm: 5 },
  keymap: { cmd_tap_timeout_ms: 300, rules: [], app_exceptions: [], reverse_modifiers: { ctrl_cmd: false } },
  hotkeys: [],
  clipboard: { enabled: true, mode: "on_switch", max_bytes: 10 * 1024 * 1024, formats: ["Text"] },
  audio: { pc_mode: "system", process_names: [], virtual_output_name: null, jitter_target_ms: 40, mac_output_id: null, mac_input_id: null, stream_volume: 1 },
  layout: {},
  general: { launch_at_login: false },
} as unknown as Config;

beforeEach(() => {
  bridge.listeners.clear();
  bridge.invoke.mockReset();
  bridge.invoke.mockImplementation(async (name: string) => {
    if (name === "get_status")
      return { status: "Connected", peer_name: "Desktop", focus: "Mac", locked: false, paused: false, secure_input: false, pc_unreachable: false, clipboard_sync: false, audio: { pc_to_mac: false, mac_to_pc: false } };
    if (name === "get_displays") return { mac: [], windows: [] };
    if (name === "get_config") return structuredClone(CONFIG);
    return undefined;
  });
});
afterEach(cleanup);

test("abre na visão geral e a sidebar troca de seção", async () => {
  render(<Main />);
  expect(screen.getByRole("heading", { name: "Visão geral" })).toBeTruthy();
  expect(await screen.findByText(/Sem métricas/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Telas" }));
  expect(await screen.findByText("editor de telas")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Telas" }).getAttribute("aria-current")).toBe("page");
});

test("relay:navigate leva à seção pedida e ignora id desconhecido", async () => {
  render(<Main />);
  await waitFor(() => expect(bridge.navigate).toBeDefined());
  bridge.navigate?.({ payload: "inexistente" });
  expect(screen.getByRole("heading", { name: "Visão geral" })).toBeTruthy();
  bridge.navigate?.({ payload: "general" });
  expect(await screen.findByRole("heading", { name: "Geral" })).toBeTruthy();
  expect(await screen.findByRole("heading", { name: "Início" })).toBeTruthy();
});
