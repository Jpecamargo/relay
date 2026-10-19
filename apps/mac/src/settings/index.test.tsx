// @vitest-environment jsdom
import type { Config } from "@relay/protocol-types";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SaveBar, SettingsPane, SettingsProvider, type SettingsView } from "./index";

const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@relay/protocol-types", () => ({
  invokeCommand: bridge.invoke,
  listenEvent: async () => () => undefined,
}));

const CONFIG: Config = {
  schema_version: 1,
  network: { ports: { control: 1, input: 2, audio_pc_to_mac: 3, audio_mac_to_pc: 4, transfer: 5 }, manual_address: null },
  transition: { dwell_ms: 150, edge_resistance_pt: 8, require_modifier: null, corner_dead_zone_mm: 5 },
  keymap: { cmd_tap_timeout_ms: 300, rules: [], app_exceptions: [], reverse_modifiers: { ctrl_cmd: false } } as unknown as Config["keymap"],
  hotkeys: [{ combo: { modifiers: ["Ctrl", "Option"], key: "Digit1" }, action: { JumpTo: 1 } }],
  clipboard: { enabled: true, mode: "on_switch", max_bytes: 10 * 1024 * 1024, formats: ["Text"] },
  audio: { pc_mode: "system", process_names: [], virtual_output_name: null, jitter_target_ms: 40, mac_output_id: null, mac_input_id: null, stream_volume: 1 },
  layout: {} as Config["layout"],
  general: { launch_at_login: false },
};

beforeEach(() => {
  bridge.invoke.mockImplementation(async (name: string) => {
    if (name === "get_config") return structuredClone(CONFIG);
    if (name === "list_discovered") return [{ name: "DESKTOP-PC", addrs: ["192.168.0.20"], port: 7000, device_id: "ab", protocol_version: 1 }];
    if (name === "get_status") return { peer_name: null };
    return undefined;
  });
});
afterEach(cleanup);

const open = async (...views: SettingsView[]) => {
  render(
    <SettingsProvider>
      <SettingsPane views={views} />
      <SaveBar />
    </SettingsProvider>,
  );
  await screen.findByRole("heading", { name: views[0] === "connection" ? "PC pareado" : views[0] === "hotkeys" ? "Atalhos globais" : "Troca de máquina" });
};

test("em Ajustes, colar os 6 dígitos envia o pareamento sem o botão Parear", async () => {
  await open("connection");
  expect(screen.queryByRole("button", { name: "Parear" })).toBeNull();
  await screen.findByRole("button", { name: /DESKTOP-PC/ });
  fireEvent.paste(screen.getByLabelText("Dígito 1"), { clipboardData: { getData: () => "123456" } });
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("pair_with_code", { code: "123456", address: "192.168.0.20:7000" }));
});

test("revogar pede confirmação antes de unpair", async () => {
  bridge.invoke.mockImplementation(async (name: string) => (name === "get_config" ? structuredClone(CONFIG) : name === "get_status" ? { peer_name: "DESKTOP-PC" } : name === "list_discovered" ? [] : undefined));
  await open("connection");
  fireEvent.click(await screen.findByRole("button", { name: "Revogar pareamento" }));
  expect(bridge.invoke).not.toHaveBeenCalledWith("unpair");
  fireEvent.click(within(screen.getByRole("alertdialog")).getByRole("button", { name: "Revogar" }));
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("unpair"));
});

test("o gravador captura Ctrl+Option+L e o conflito bloqueia o salvamento", async () => {
  await open("hotkeys");
  const lock = screen.getByRole("button", { name: "Gravar atalho: Travar cursor" });
  fireEvent.click(lock);
  fireEvent.keyDown(lock, { code: "KeyL", key: "¬", ctrlKey: true, altKey: true });
  expect(within(lock).getByLabelText("Ctrl Option L")).toBeTruthy();

  // mesma combinação do "Ir para a tela 1" (Ctrl+Option+1)
  const pause = screen.getByRole("button", { name: "Gravar atalho: Pausar compartilhamento" });
  fireEvent.click(pause);
  fireEvent.keyDown(pause, { code: "Digit1", key: "1", ctrlKey: true, altKey: true });
  expect((await screen.findAllByText(/Conflita com/)).length).toBe(2);
  fireEvent.click(screen.getByRole("button", { name: "Salvar" }));
  expect(bridge.invoke).not.toHaveBeenCalledWith("save_config", expect.anything());
});

test("salvar envia o Config completo com a resistência da borda alterada", async () => {
  await open("transition");
  fireEvent.change(screen.getByLabelText("Resistência da borda"), { target: { value: "medium" } });
  fireEvent.click(screen.getByRole("button", { name: "Salvar" }));
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("save_config", { config: { ...CONFIG, transition: { ...CONFIG.transition, edge_resistance_pt: 20, dwell_ms: 0 } } }));
});

test("erro de validação do backend aparece inline", async () => {
  bridge.invoke.mockImplementation(async (name: string) => {
    if (name === "get_config") return structuredClone(CONFIG);
    if (name === "save_config") throw "dwell_ms fora do intervalo";
    if (name === "list_discovered") return [];
    if (name === "get_status") return { peer_name: null };
    return undefined;
  });
  await open("transition");
  fireEvent.change(screen.getByLabelText("Resistência da borda"), { target: { value: "none" } });
  fireEvent.click(screen.getByRole("button", { name: "Salvar" }));
  expect(await screen.findByText("dwell_ms fora do intervalo")).toBeTruthy();
});
