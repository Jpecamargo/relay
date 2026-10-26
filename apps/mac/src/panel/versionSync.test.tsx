// @vitest-environment jsdom
import type { StatusSnapshot } from "@relay/protocol-types";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { Panel } from "./index";

const m = vi.hoisted(() => ({
  invoke: vi.fn(),
  emitTo: vi.fn(),
  check: vi.fn(),
  status: undefined as unknown as StatusSnapshot,
}));

vi.mock("@relay/protocol-types", () => ({
  listenEvent: async () => () => undefined,
  invokeCommand: m.invoke,
}));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "0.1.8" }));
vi.mock("@tauri-apps/api/event", () => ({ emit: async () => undefined, emitTo: m.emitTo, listen: async () => () => undefined }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ hide: async () => undefined }) }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check: m.check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: async () => undefined }));

const STATUS: StatusSnapshot = {
  status: "Connected", peer_name: "Desktop", focus: "Local", locked: false, paused: false,
  secure_input: false, peer_unreachable: false, clipboard_sync: false, audio: { mic_to_peer: false, mic_from_peer: false },
  peer_version: "0.2.0", version_sync: "LocalBehind",
  local_os: "Mac", peer_os: null, local_caps: { os: "Mac", features: 544 }, peer_caps: null, layout_host: "Local", layout_editable: true,
};

beforeEach(() => {
  vi.stubEnv("PROD", true);
  m.emitTo.mockReset().mockResolvedValue(undefined);
  m.check.mockReset().mockResolvedValue({ version: "0.2.0", body: "notas", downloadAndInstall: vi.fn() });
  m.status = STATUS;
  m.invoke.mockReset().mockImplementation(async (name: string) => {
    if (name === "get_status") return m.status;
    if (name === "check_permissions") return { accessibility: "Granted", input_monitoring: "Granted" };
    return undefined;
  });
});
afterEach(() => {
  cleanup();
  vi.unstubAllEnvs();
});

test("Mac mais antigo: confere a atualização na hora e abre o modal da janela principal explicando que é para igualar ao PC", async () => {
  render(<Panel />);
  await waitFor(() =>
    expect(m.emitTo).toHaveBeenCalledWith("main", "relay:open-update", { name: "PC", version: "0.2.0" }),
  );
  expect(m.check).toHaveBeenCalledTimes(1);
});

test("versões iguais: não procura nem abre nada", async () => {
  m.status = { ...STATUS, version_sync: "InSync" };
  render(<Panel />);
  await act(async () => {});
  expect(m.check).not.toHaveBeenCalled();
  expect(m.emitTo).not.toHaveBeenCalled();
});
