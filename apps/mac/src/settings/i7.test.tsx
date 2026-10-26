// @vitest-environment jsdom
// I7: aba "Este computador" por SO, os dois modos de pareamento e a troca do host dos layouts.
import type { Config, StatusSnapshot } from "@relay/protocol-types";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SettingsPane, SettingsProvider, type SettingsView } from "./index";

const bridge = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, (p: unknown) => void>() }));
vi.mock("@relay/protocol-types", () => ({
  invokeCommand: bridge.invoke,
  listenEvent: async (name: string, cb: (p: unknown) => void) => {
    bridge.listeners.set(name, cb);
    return () => bridge.listeners.delete(name);
  },
}));

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
const STATUS = {
  status: "Connected", peer_name: "DESKTOP-PC", focus: "Local", local_os: "Mac", peer_os: "Windows",
  layout_host: "Local", layout_editable: true,
} as unknown as StatusSnapshot;
const PEER = { name: "DESKTOP-PC", addrs: ["192.168.0.20"], port: 7000, device_id: "ab", protocol_version: 1, os: "Windows", app_version: "0.3.0" };

let status: StatusSnapshot;
beforeEach(() => {
  status = STATUS;
  bridge.listeners.clear();
  bridge.invoke.mockReset();
  bridge.invoke.mockImplementation(async (name: string) => {
    if (name === "get_status") return status;
    if (name === "get_config") return structuredClone(CONFIG);
    if (name === "list_discovered") return [PEER];
    if (name === "check_permissions") return { accessibility: "Granted", input_monitoring: "Denied" };
    if (name === "get_service_status") return { installed: true, running: true, helper_active: false, vbcable_detected: true };
    if (name === "new_pairing_code") return { code: "123456", expires_at: Date.now() + 120_000 };
    return undefined;
  });
});
afterEach(cleanup);

const open = (...views: SettingsView[]) =>
  render(
    <SettingsProvider>
      <SettingsPane views={views} />
    </SettingsProvider>,
  );

test("Este computador mostra permissões no Mac e serviço e VB-CABLE no Windows", async () => {
  open("computer");
  expect(await screen.findByText(/Monitoramento de entrada/)).toBeTruthy();
  expect(screen.getByText("Iniciar no login")).toBeTruthy();
  expect(screen.queryByText("VB-CABLE (microfone)")).toBeNull();
  cleanup();

  status = { ...STATUS, local_os: "Windows" };
  open("computer");
  expect(await screen.findByText("VB-CABLE (microfone)")).toBeTruthy();
  expect(screen.getByText("Iniciar com o Windows")).toBeTruthy();
  expect(screen.getByText("Helper de entrada")).toBeTruthy();
  expect(screen.queryByText(/Monitoramento de entrada/)).toBeNull();
});

test("Mostrar código gera o código, conta 120 s e reage ao relay:pairing", async () => {
  open("connection");
  fireEvent.click(await screen.findByRole("tab", { name: "Mostrar código" }));
  const code = await screen.findByLabelText("Código de pareamento");
  expect(code.textContent).toBe("123456");
  expect(screen.getByText("2:00")).toBeTruthy();
  expect(bridge.invoke).toHaveBeenCalledWith("new_pairing_code");

  await act(async () => bridge.listeners.get("relay:pairing")?.({ state: "failed", code: null, expires_at: null }));
  expect(screen.queryByLabelText("Código de pareamento")).toBeNull();
  expect(screen.getByText(/pareamento falhou/)).toBeTruthy();
});

test("Digitar código envia o layout_host escolhido (padrão: este computador)", async () => {
  open("connection");
  await screen.findByRole("button", { name: /DESKTOP-PC/ });
  const host = screen.getByLabelText("Guardar os layouts em") as HTMLSelectElement;
  expect(host.value).toBe("Local");
  fireEvent.change(host, { target: { value: "Peer" } });
  fireEvent.paste(screen.getByLabelText("Dígito 1"), { clipboardData: { getData: () => "654321" } });
  await waitFor(() =>
    expect(bridge.invoke).toHaveBeenCalledWith("pair_with_code", { code: "654321", address: "192.168.0.20:7000", layout_host: "Peer" }),
  );
});

test("Layout troca o host por set_layout_host e fica desabilitado sem permissão de edição", async () => {
  open("layout");
  const select = (await screen.findByLabelText("Guardar os layouts em")) as HTMLSelectElement;
  fireEvent.change(select, { target: { value: "Peer" } });
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("set_layout_host", { host: "Peer" }));
  cleanup();

  status = { ...STATUS, layout_editable: false };
  open("layout");
  expect(((await screen.findByLabelText("Guardar os layouts em")) as HTMLSelectElement).disabled).toBe(true);
  expect(screen.getByText(/Só leitura agora/)).toBeTruthy();
});
