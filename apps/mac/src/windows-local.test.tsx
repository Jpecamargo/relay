// @vitest-environment jsdom
// Onda 2 (I5, I6, I7): painel, editor e ajustes juntos com status de Windows (local Windows,
// peer Mac com capacidades baseline) e o bridge mockado. Nada que o Mac não suporta aparece e
// nenhum texto fixo "Mac"/"PC" nomeia os lados.
import type { StatusSnapshot } from "@relay/protocol-types";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { Editor } from "./editor";
import { Panel } from "./panel";
import { SettingsPane, SettingsProvider, type SettingsView } from "./settings";
import { Main } from "./shell";

class TestPointerEvent extends MouseEvent {
  pointerId: number;
  constructor(type: string, init: MouseEventInit & { pointerId?: number } = {}) {
    super(type, init);
    this.pointerId = init.pointerId ?? 1;
  }
}
vi.stubGlobal("PointerEvent", TestPointerEvent);

const bridge = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, (p: unknown) => void>() }));
vi.mock("@relay/protocol-types", async (importActual) => ({
  ...(await importActual<typeof import("@relay/protocol-types")>()),
  invokeCommand: bridge.invoke,
  listenEvent: async (name: string, cb: (p: unknown) => void) => {
    bridge.listeners.set(name, cb);
    return () => bridge.listeners.delete(name);
  },
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "0.3.0" }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => undefined }));

const PEER = "Desktop-Joao";
const MAC_BASELINE = { os: "Mac", features: (1 << 5) | (1 << 9) }; // MIC_SOURCE | FILE_PULL
const WIN_BASELINE = { os: "Windows", features: (1 << 10) - 1 - ((1 << 5) | (1 << 9)) };
const WIN_LOCAL = { os: "Windows", features: (1 << 10) - 1 - ((1 << 5) | (1 << 9)) - (1 << 3) };

const base = {
  status: "Connected", peer_name: PEER, focus: "Local", locked: false, paused: false, secure_input: false,
  peer_unreachable: false, clipboard_sync: false, audio: { mic_to_peer: false, mic_from_peer: false },
  peer_version: "0.3.0", version_sync: "InSync", layout_host: "Local", layout_editable: true,
};
const WINDOWS_STATUS = { ...base, local_os: "Windows", peer_os: "Mac", local_caps: WIN_LOCAL, peer_caps: MAC_BASELINE } as unknown as StatusSnapshot;
const MAC_STATUS = { ...base, local_os: "Mac", peer_os: "Windows", local_caps: MAC_BASELINE, peer_caps: WIN_BASELINE } as unknown as StatusSnapshot;

const info = (id: string, name: string, machine: string) => ({
  id, name, machine, origin_px: [0, 0], size_px: [1000, 1000], scale: 1, size_mm: [300, 200], is_primary: true, is_builtin: false,
});
const placed = (id: string, machine: string, x: number) => ({
  display_id: id, machine, rect_mm: { x, y: 0, w: 300, h: 200 }, sensitivity: 1, rotation: null,
});
const PROFILE = { key: ["m", "w"], name: "Mesa", displays: [placed("m1", "Mac", 0), placed("w1", "Windows", 300)], windows_detached: false };
const CONFIG = {
  schema_version: 1,
  network: { ports: { control: 1, input: 2, audio_mac_to_pc: 4, transfer: 5 }, manual_address: null },
  transition: { dwell_ms: 150, edge_resistance_pt: 8, require_modifier: null, corner_dead_zone_mm: 5 },
  keymap: { cmd_tap_timeout_ms: 300, rules: [], app_exceptions: [], reverse_modifiers: { ctrl_cmd: false } },
  hotkeys: [],
  clipboard: { enabled: true, mode: "on_switch", max_bytes: 10485760, formats: ["Text"] },
  audio: { jitter_target_ms: 40, mac_input_id: null },
  layout: { profiles: [PROFILE], active: "Mesa" },
  general: { launch_at_login: false },
};

let status: StatusSnapshot;
beforeEach(() => {
  status = WINDOWS_STATUS;
  bridge.listeners.clear();
  bridge.invoke.mockReset();
  bridge.invoke.mockImplementation(async (name: string) => {
    switch (name) {
      case "get_status": return status;
      case "get_config": return structuredClone(CONFIG);
      case "get_displays":
        // Local = esta máquina. No PC, o local é Windows e o peer é o Mac.
        return status.local_os === "Windows"
          ? { local: [info("w1", "Monitor A", "Windows")], peer: [info("m1", "Tela B", "Mac")] }
          : { local: [info("m1", "Tela B", "Mac")], peer: [info("w1", "Monitor A", "Windows")] };
      case "get_layout": return PROFILE;
      case "get_layout_book": return { revision: 1, host: "Mac", profiles: [PROFILE], active: "Mesa" };
      case "list_discovered": return [];
      case "check_permissions": return { accessibility: "Granted", input_monitoring: "Granted" };
      case "get_service_status": return { installed: true, running: true, helper_active: true, vbcable_detected: true };
      case "send_command": return { id: 1, ok: true, output: null, error: null };
      default: return undefined;
    }
  });
});
afterEach(cleanup);

const text = () => document.body.textContent ?? "";
const labels = () =>
  [...document.querySelectorAll("[aria-label]")].map((e) => e.getAttribute("aria-label") ?? "").join(" | ");
const visible = () => `${text()} | ${labels()}`;

/** Nomes fixos de lado (Mac/PC) — "Windows" pode aparecer como recurso do SO ("Iniciar com o Windows"). */
const FIXED_SIDE = /\b(Mac|PC|MacBook)\b/;

async function openPanelOnWindows() {
  render(<Panel />);
  await screen.findByText(/Foco: Este computador/);
  // Deixa o carregamento assíncrono (config, displays) terminar.
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("get_status"));
  await new Promise((r) => setTimeout(r, 30));
}

test("painel no Windows com peer Mac: nada de comandos, volume, captura, mic, acordar ou arquivos", async () => {
  await openPanelOnWindows();
  const all = visible();
  for (const absent of [/Bloquear/, /Reiniciar/, /Suspender/, /Desligar/, /Saída de som/, /Mudo em/, /Capturar tela/, /Acordar/i, /Receber arquivos/i, /microfone/i, /Volume/i]) {
    expect(all, String(absent)).not.toMatch(absent);
  }
  expect(screen.queryAllByRole("slider")).toHaveLength(0);
  expect(all).toContain(PEER);
  expect(all).not.toMatch(FIXED_SIDE);
});

test("controle: o mesmo painel com status de Mac mostra os itens da 0.2", async () => {
  status = MAC_STATUS;
  render(<Panel />);
  await screen.findByRole("button", { name: `Mudo em ${PEER}` });
  fireEvent.click(screen.getByRole("button", { name: PEER }));
  expect(await screen.findByRole("button", { name: "Bloquear" })).toBeTruthy();
  expect(screen.getByRole("button", { name: `Capturar tela de ${PEER}` })).toBeTruthy();
});

test("o foco é dito por Host: o nome do peer, sem Mac/PC", async () => {
  status = { ...WINDOWS_STATUS, focus: "Peer" } as StatusSnapshot;
  render(<Panel />);
  expect(await screen.findByText(`Foco: ${PEER}`)).toBeTruthy();
  expect(visible()).not.toMatch(FIXED_SIDE);
});

test("editor aberto no PC: telas do Mac fixas, do Windows arrastáveis, selo na tela local", async () => {
  render(<Editor />);
  await screen.findByTestId("display-m1");
  const y = (id: string) => screen.getByTestId(`display-${id}`).querySelector("rect")!.getAttribute("x");
  const [mac0, win0] = [y("m1"), y("w1")];

  const drag = (id: string) => {
    const g = screen.getByTestId(`display-${id}`);
    fireEvent.pointerDown(g, { clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(g, { clientX: 140, clientY: 100, pointerId: 1 });
    fireEvent.pointerUp(g, { pointerId: 1 });
  };
  drag("m1");
  expect(y("m1")).toBe(mac0);
  drag("w1");
  expect(y("w1")).not.toBe(win0);

  expect(screen.getByTestId("display-w1").textContent).toContain("Este computador");
  expect(screen.getByTestId("display-m1").textContent).not.toContain("Este computador");
});

test("editor no PC com host offline: só-leitura que nomeia o host pelo peer_name", async () => {
  status = { ...WINDOWS_STATUS, status: "Disconnected", layout_host: "Peer", layout_editable: false } as StatusSnapshot;
  render(<Editor />);
  await screen.findByTestId("display-m1");
  expect(text()).toContain(PEER);
  expect(screen.getByRole("button", { name: "Salvar" })).toHaveProperty("disabled", true);
  expect(screen.getByRole("button", { name: "Novo perfil" })).toHaveProperty("disabled", true);
});

test("editor no PC com peer 0.2: pede para atualizar o Relay no peer", async () => {
  status = { ...WINDOWS_STATUS, peer_version: "0.2.9", layout_host: "Peer", layout_editable: false } as unknown as StatusSnapshot;
  render(<Editor />);
  await screen.findByTestId("display-m1");
  expect(text()).toMatch(new RegExp(`atualiz\\w+ o Relay em ${PEER}`, "i"));
});

// "keyboard"/"hotkeys" ficam de fora: a tabela de regras de atalho tem colunas Mac/Windows de teclas (keymap, D39).
const ALL_VIEWS: SettingsView[] = ["connection", "computer", "layout", "transition", "clipboard", "audio"];

test("ajustes no Windows: aba do PC mostra serviço e VB-CABLE, sem permissões do Mac, e nada de Mac/PC", async () => {
  render(
    <SettingsProvider>
      <SettingsPane views={ALL_VIEWS} />
    </SettingsProvider>,
  );
  expect(await screen.findByText("VB-CABLE (microfone)")).toBeTruthy();
  expect(text()).not.toMatch(/Monitoramento de entrada|Acessibilidade/);
  expect(bridge.invoke).not.toHaveBeenCalledWith("open_permission_settings");
  expect(visible()).not.toMatch(FIXED_SIDE);
});

test("janela principal no Windows: nenhuma seção nomeia os lados por Mac/PC", async () => {
  render(<Main />);
  await screen.findByRole("heading", { name: "Visão geral" });
  await new Promise((r) => setTimeout(r, 30));
  expect(visible()).not.toMatch(FIXED_SIDE);
  for (const section of ["Áudio", "Clipboard e arquivos", "Conexão e pareamento", "Este computador", "Geral"]) {
    fireEvent.click(screen.getByRole("button", { name: section }));
    await screen.findByRole("heading", { name: section });
    await new Promise((r) => setTimeout(r, 20));
    expect(visible(), section).not.toMatch(FIXED_SIDE);
    expect(text(), section).not.toMatch(/Bloquear|Reiniciar|Saída de som|Capturar tela/);
  }
});

test("janela principal: Windows local não mostra ferramentas, métricas nem receber arquivos; Mac local mostra", async () => {
  const hidden = /Ferramentas do outro computador|Receber arquivos|Sem métricas|Capturar tela/;
  render(<Main />);
  await screen.findByRole("heading", { name: "Visão geral" });
  await new Promise((r) => setTimeout(r, 30));
  expect(text()).not.toMatch(hidden);
  fireEvent.click(screen.getByRole("button", { name: "Clipboard e arquivos" }));
  await screen.findByRole("heading", { name: "Clipboard e arquivos" });
  expect(text()).not.toMatch(/Receber arquivos/);
  cleanup();

  status = { ...MAC_STATUS, peer_caps: { os: "Windows", features: (1 << 10) - 1 } } as unknown as StatusSnapshot;
  render(<Main />);
  await screen.findByRole("heading", { name: "Visão geral" });
  expect(await screen.findByText(/Ferramentas do outro computador/)).toBeTruthy();
  expect(text()).toMatch(/Sem métricas/);
  expect(text()).toMatch(/Receber arquivos/);
});

test("painel num PC ligado a Mac 0.2 atrasado pede atualização manual, sem dizer que atualiza", async () => {
  status = { ...WINDOWS_STATUS, version_sync: "PeerBehind" } as unknown as StatusSnapshot;
  render(<Panel />);
  expect(await screen.findByText(new RegExp(`Atualize o Relay no ${PEER}`))).toBeTruthy();
  expect(text()).not.toMatch(/Atualizando/);
});

test("editor no Windows manda segurar Alt", async () => {
  render(<Editor />);
  await screen.findByTestId("display-m1");
  expect(text()).toMatch(/segure Alt para/);
  expect(text()).not.toMatch(/Option para/);
});
