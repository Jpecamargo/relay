// @vitest-environment jsdom
import type { StatusSnapshot } from "@relay/protocol-types";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { Panel } from "./index";

const bridge = vi.hoisted(() => ({
  listeners: new Map<string, (p: unknown) => void>(),
  invoke: vi.fn(),
}));

vi.mock("@relay/protocol-types", async (importActual) => ({
  ...(await importActual<typeof import("@relay/protocol-types")>()),
  listenEvent: async (name: string, cb: (p: unknown) => void) => {
    bridge.listeners.set(name, cb);
    return () => bridge.listeners.delete(name);
  },
  invokeCommand: bridge.invoke,
}));

vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "0.2.0" }));

const STATUS: StatusSnapshot = {
  status: "Connected", peer_name: "Desktop", focus: "Local", locked: false, paused: false,
  secure_input: false, peer_unreachable: false, clipboard_sync: false, audio: { mic_to_peer: false, mic_from_peer: false },
  peer_version: null, version_sync: "InSync",
  local_os: "Mac", peer_os: "Windows", local_caps: { os: "Mac", features: 544 }, peer_caps: { os: "Windows", features: 479 }, layout_host: "Local", layout_editable: true,
};

let sendResult: unknown;
let hotkeys: unknown[];
const DEVICES = [
  { id: "spk", name: "Alto-falantes", is_default: true },
  { id: "hp", name: "Fone USB", is_default: false },
];
const PROFILES = [{ name: "Casa", key: "k", displays: [], windows_detached: false }, { name: "Jogo", key: "k", displays: [], windows_detached: false }];
const info = (id: string, name: string, machine: string) => ({ id, name, machine, origin_px: [0, 0], size_px: [1, 1], scale: 1, size_mm: null, is_primary: false, is_builtin: false });
const placed = (id: string, machine: string, x: number) => ({ display_id: id, machine, rect_mm: { x, y: 0, w: 100, h: 100 }, sensitivity: 1, rotation: null });

beforeEach(() => {
  bridge.listeners.clear();
  bridge.invoke.mockReset();
  sendResult = { id: 1, ok: true, output: null, error: null };
  hotkeys = [
    { combo: { modifiers: ["Ctrl", "Option"], key: "KeyL" }, action: "ToggleLock" },
    { combo: { modifiers: ["Ctrl", "Option"], key: "Digit2" }, action: { JumpTo: 2 } },
  ];
  bridge.invoke.mockImplementation(async (name: string, args?: { command?: unknown }) => {
    switch (name) {
      case "get_status": return STATUS;
      case "check_permissions": return { accessibility: "Granted", input_monitoring: "Granted" };
      case "get_config": return { audio: { jitter_target_ms: 20, mac_input_id: null }, hotkeys, layout: { profiles: PROFILES, active: "Casa" } };
      case "get_displays": return { local: [info("mac", "MacBook", "Mac")], peer: [info("d1", "Dell 1", "Windows"), info("d2", "Dell 2", "Windows")] };
      case "get_layout": return { name: "Casa", displays: [placed("mac", "Mac", 0), placed("d1", "Windows", 100), placed("d2", "Windows", 200)] };
      case "send_command":
        if (args?.command === "ListOutputDevices") return { id: 5, ok: true, output: { Devices: DEVICES }, error: null };
        return sendResult;
      default: return undefined;
    }
  });
});
afterEach(cleanup);

const emit = (name: string, payload: unknown) => act(() => void bridge.listeners.get(name)?.(payload));
const sentWith = (text: string) =>
  bridge.invoke.mock.calls.filter(([n, a]) => n === "send_command" && JSON.stringify(a).includes(text));

async function ready() {
  render(<Panel />);
  await waitFor(() => expect(screen.getByRole("button", { name: "Mudo em Desktop" }).hasAttribute("disabled")).toBe(false));
}

/** Abre o submenu "PC ›" (energia). */
const openPc = async () => {
  await ready();
  fireEvent.click(screen.getByRole("button", { name: "Desktop" }));
};

test("Bloquear envia Power Lock sem confirmação e sucesso não mostra toast", async () => {
  await openPc();
  fireEvent.click(screen.getByRole("button", { name: "Bloquear" }));
  await waitFor(() =>
    expect(bridge.invoke).toHaveBeenCalledWith("send_command", { command: { Power: { action: "Lock", confirm: false } } }),
  );
  await waitFor(() => expect(sentWith("Lock")).toHaveLength(1));
  expect(screen.queryByText(/: ok/)).toBeNull();
  expect(screen.queryByRole("button", { name: "Fechar" })).toBeNull(); // nenhum toast
});

test("Reiniciar só envia depois de confirmar, com confirm:true", async () => {
  await openPc();
  fireEvent.click(screen.getByRole("button", { name: "Reiniciar" }));
  expect(sentWith("Power")).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: "Cancelar" }));
  expect(sentWith("Power")).toHaveLength(0);

  fireEvent.click(screen.getByRole("button", { name: "Reiniciar" }));
  const dialog = screen.getByRole("alertdialog");
  fireEvent.click(Array.from(dialog.querySelectorAll("button")).find((b) => b.textContent === "Reiniciar")!);
  await waitFor(() =>
    expect(bridge.invoke).toHaveBeenCalledWith("send_command", { command: { Power: { action: "Restart", confirm: true } } }),
  );
});

test("CommandResult com erro vira toast de erro", async () => {
  await openPc();
  sendResult = { id: 2, ok: false, output: null, error: "timeout" };
  fireEvent.click(screen.getByRole("button", { name: "Suspender" }));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("O PC não respondeu a tempo.");
});

test("evento de latência atualiza o texto do cabeçalho", async () => {
  await ready();
  await emit("relay:latency", { rtt_ms: 4.2, jitter_ms: 0.8, loss_pct: 0 });
  expect(screen.getByText("Conectado · 4,2 ms")).toBeTruthy();
});

test("cabeçalho mostra o PC pareado e onde está o foco", async () => {
  await ready();
  expect(screen.getAllByText("Desktop").length).toBeGreaterThan(0);
  expect(screen.getByText("Foco: Este computador")).toBeTruthy();
  await emit("relay:status", { ...STATUS, focus: "Peer" });
  expect(screen.getByText("Foco: Desktop")).toBeTruthy();
});

test("toggle de cursor chama set_cursor_lock sem toast de sucesso", async () => {
  await ready();
  fireEvent.click(screen.getByRole("switch", { name: /Travar cursor/ }));
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("set_cursor_lock", { locked: true }));
  expect(screen.queryByText(/Travar cursor: ok/)).toBeNull();
});

test("linhas de menu do rodapé: Ajustes e Encerrar", async () => {
  await ready();
  fireEvent.click(screen.getByRole("button", { name: /Encerrar/ }));
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("quit_app"));
  expect(screen.getByRole("button", { name: /Ajustes/ }).textContent).toContain("⌘,");
});

test("banner de Secure Input aparece com active:true", async () => {
  render(<Panel />);
  await waitFor(() => expect(bridge.listeners.has("relay:secure-input")).toBe(true));
  expect(screen.queryByText(/Secure Input ativo/)).toBeNull();
  await emit("relay:secure-input", { active: true });
  expect(screen.getByText(/Secure Input ativo/)).toBeTruthy();
});

test("slider de volume do Windows envia SetVolume depois de 50 ms", async () => {
  await ready();
  vi.useFakeTimers();
  try {
    fireEvent.change(screen.getByLabelText("Volume de Desktop"), { target: { value: "0.3" } });
    expect(sentWith("SetVolume")).toHaveLength(0);
    await act(async () => void vi.advanceTimersByTime(50));
    expect(bridge.invoke).toHaveBeenCalledWith("send_command", { command: { SetVolume: { level: 0.3 } } });
  } finally {
    vi.useRealTimers();
  }
});

test("volume exibido acompanha cada troca de saída do PC, não só a primeira", async () => {
  await ready();
  const metrics = (volume: number) => ({ cpu_pct: 0, mem_pct: 0, volume, muted: false });
  const shown = () => Number((screen.getByLabelText("Volume de Desktop") as HTMLInputElement).value);
  for (const v of [0.8, 0.2]) {
    await emit("relay:metrics", metrics(v));
    expect(shown()).toBeCloseTo(v);
  }
});

test("banner de reconexão tem 'Tentar de novo' que chama reconnect", async () => {
  await ready();
  emit("relay:status", { ...STATUS, status: "Reconnecting" });
  fireEvent.click(await screen.findByRole("button", { name: "Tentar de novo" }));
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("reconnect"));
});

test("PC visto mas sem resposta explica o firewall; sem o flag, nada", async () => {
  await ready();
  emit("relay:status", { ...STATUS, status: "Connecting" });
  expect(screen.queryByText(/firewall do Windows/)).toBeNull();
  emit("relay:status", { ...STATUS, status: "Connecting", peer_unreachable: true });
  expect((await screen.findByText(/Desktop está na rede mas não responde/)).textContent).toContain("isolando o Wi-Fi do cabo");
});

test("PC mais antigo: o painel diz que o PC está atualizando, ou pede a atualização manual se ele é velho demais", async () => {
  await ready();
  expect(screen.queryByText(/Atualizando Desktop/)).toBeNull();
  emit("relay:status", { ...STATUS, peer_version: "0.1.8", version_sync: "PeerBehind" });
  expect(await screen.findByText("Atualizando Desktop para a versão 0.2.0…")).toBeTruthy();
  emit("relay:status", { ...STATUS, peer_version: "0.1.0", version_sync: "PeerBehindManual" });
  expect(await screen.findByText("Atualize o Relay no Desktop para 0.2.0.")).toBeTruthy();
  expect(screen.queryByText(/Atualizando Desktop/)).toBeNull();
  emit("relay:status", { ...STATUS, peer_version: "0.2.0", version_sync: "InSync" });
  await waitFor(() => expect(screen.queryByText(/Relay no Desktop/)).toBeNull());
});

test("relay:command-result de áudio mostra toast e desliga o toggle", async () => {
  await ready();
  await emit("relay:command-result", { id: 0, ok: false, output: null, error: "vbcable_missing" });
  expect((await screen.findByRole("alert")).textContent).toContain("VB-CABLE");
  await waitFor(() =>
    expect(bridge.invoke).toHaveBeenCalledWith("set_audio_streams", { mic_to_peer: false }),
  );
});

test("resultado igual por invoke e por evento gera um toast só", async () => {
  await openPc();
  sendResult = { id: 7, ok: false, output: null, error: "timeout" };
  fireEvent.click(screen.getByRole("button", { name: "Suspender" }));
  await screen.findByRole("alert");
  await emit("relay:command-result", sendResult);
  expect(screen.getAllByRole("alert")).toHaveLength(1);
  expect(screen.getByRole("alert").textContent).toContain("Suspender");
});

test("resultado de hotkey (só evento) aparece como toast", async () => {
  await ready();
  await emit("relay:command-result", { id: 9, ok: false, output: null, error: "unsupported" });
  expect((await screen.findByRole("alert")).textContent).toContain("Não suportado");
});

const menu = async (name: string | RegExp) => {
  await ready();
  fireEvent.click(screen.getByRole("button", { name }));
};

test("Saída de som lista as do PC com check na atual e troca na hora", async () => {
  await menu("Saída de som");
  const hp = await screen.findByRole("menuitemradio", { name: "Fone USB" });
  expect(screen.getByRole("menuitemradio", { name: "Alto-falantes" }).getAttribute("aria-checked")).toBe("true");
  expect(hp.getAttribute("aria-checked")).toBe("false");
  fireEvent.click(hp);
  await waitFor(() =>
    expect(bridge.invoke).toHaveBeenCalledWith("send_command", { command: { SetOutputDevice: { id: "hp" } } }),
  );
  expect(hp.getAttribute("aria-checked")).toBe("true");
  expect(screen.getByRole("menuitemradio", { name: "Alto-falantes" }).getAttribute("aria-checked")).toBe("false");
  expect(screen.queryByRole("button", { name: "Fechar" })).toBeNull(); // sem toast de sucesso
});

test("erro ao trocar a saída vira toast de erro", async () => {
  await menu("Saída de som");
  const hp = await screen.findByRole("menuitemradio", { name: "Fone USB" });
  sendResult = { id: 9, ok: false, output: null, error: "timeout" };
  fireEvent.click(hp);
  expect((await screen.findByRole("alert")).textContent).toContain("O PC não respondeu a tempo.");
});

test("atalhos mostrados vêm do Config real e atualizam ao reabrir o painel", async () => {
  await ready();
  const lock = () => screen.getByRole("switch", { name: /Travar cursor/ }).textContent;
  await waitFor(() => expect(lock()).toContain("⌃⌥L"));
  expect(screen.getByRole("switch", { name: /Pausar compartilhamento/ }).textContent).not.toContain("⌃");
  hotkeys = [{ combo: { modifiers: ["Cmd", "Shift"], key: "KeyK" }, action: "ToggleLock" }];
  await act(async () => void window.dispatchEvent(new Event("focus")));
  await waitFor(() => expect(lock()).toContain("⇧⌘K"));
});

test("Ir para lista as telas na ordem do pulo, com o atalho configurado, e pula", async () => {
  await menu("Ir para");
  const dell2 = await screen.findByRole("button", { name: /Dell 2/ });
  expect(screen.getByRole("button", { name: /Dell 1/ }).textContent).toContain("⌃⌥2"); // n = 2
  expect(dell2.textContent).not.toContain("⌃"); // sem atalho configurado para a tela 3
  const names = screen.getAllByRole("button").map((b) => b.textContent ?? "").filter((t) => /MacBook|Dell/.test(t));
  expect(names[0]).toContain("MacBook");
  fireEvent.click(dell2);
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("jump_to", { n: 3 }));
});

test("Layout lista os perfis com check no atual e troca o perfil em uso", async () => {
  await ready();
  fireEvent.click(await screen.findByRole("button", { name: "Layout" }));
  const jogo = await screen.findByRole("menuitemradio", { name: "Jogo" });
  expect(screen.getByRole("menuitemradio", { name: "Casa" }).getAttribute("aria-checked")).toBe("true");
  fireEvent.click(jogo);
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("set_layout_profiles", { profiles: PROFILES, active: "Jogo" }));
  expect(jogo.getAttribute("aria-checked")).toBe("true");
});

test("Capturar tela do PC só envia o comando e avisa que foi copiada", async () => {
  sendResult = { id: 3, ok: true, output: null, error: null };
  await ready();
  fireEvent.click(await screen.findByRole("button", { name: "Capturar tela de Desktop" }));
  fireEvent.click(await screen.findByRole("button", { name: "Dell 2" }));
  expect(await screen.findByText("Captura copiada para o clipboard")).toBeTruthy();
  expect(bridge.invoke).toHaveBeenCalledWith("send_command", { command: { CaptureScreen: { display_id: "d2" } } });
});

test("falha na captura vira toast de erro e não diz que copiou", async () => {
  sendResult = { id: 4, ok: false, output: null, error: "timeout" };
  await ready();
  fireEvent.click(await screen.findByRole("button", { name: "Capturar tela de Desktop" }));
  fireEvent.click(await screen.findByRole("button", { name: "Dell 1" }));
  expect((await screen.findByRole("alert")).textContent).toContain("O PC não respondeu a tempo.");
  expect(screen.queryByText(/copiada/)).toBeNull();
});

test("itens condicionais: Capturar só online; Reconectar só desconectado", async () => {
  await ready();
  expect(await screen.findByRole("button", { name: "Capturar tela de Desktop" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Reconectar" })).toBeNull();
  await emit("relay:status", { ...STATUS, status: "Disconnected" });
  fireEvent.click(await screen.findByRole("button", { name: "Reconectar" }));
  expect(screen.queryByRole("button", { name: "Capturar tela de Desktop" })).toBeNull();
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("reconnect"));
});

test("sem PC pareado o painel leva à tela de pareamento e esconde os controles", async () => {
  await ready();
  await emit("relay:status", { ...STATUS, status: "Disconnected", peer_name: null });
  expect(await screen.findByRole("button", { name: "Conectar outro computador…" })).toBeTruthy();
  expect(screen.getByText("Nenhum computador pareado")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Travar cursor" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Ajustes…" })).toBeNull();
});

const MAC_BASELINE = { os: "Mac", features: 544 } as const;
const WIN_BASELINE = { os: "Windows", features: 479 } as const;
const WIN_LOCAL: StatusSnapshot = { ...STATUS, local_os: "Windows", local_caps: WIN_BASELINE, peer_os: "Mac", peer_caps: MAC_BASELINE, peer_name: "Desktop-Joao" };

test("status de Mac mostra os itens da 0.2: volume, saída, comandos, captura, microfone e acordar", async () => {
  await ready();
  expect(screen.getByLabelText("Volume de Desktop")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Saída de som" })).toBeTruthy();
  expect(await screen.findByRole("button", { name: "Capturar tela de Desktop" })).toBeTruthy();
  expect(screen.getByRole("switch", { name: /Microfone deste computador em Desktop/ })).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Desktop" }));
  expect(screen.getByRole("button", { name: "Reiniciar" })).toBeTruthy();
  expect(screen.getByRole("button", { name: /Acordar/ })).toBeTruthy();
});

test("status de Windows esconde os itens condicionados às capacidades do peer Mac", async () => {
  render(<Panel />);
  await waitFor(() => expect(bridge.listeners.has("relay:status")).toBe(true));
  await emit("relay:status", WIN_LOCAL);
  expect(await screen.findByText("Desktop-Joao")).toBeTruthy();
  expect(screen.queryByLabelText(/Volume de/)).toBeNull();
  expect(screen.queryByRole("button", { name: "Saída de som" })).toBeNull();
  expect(screen.queryByRole("button", { name: /Capturar tela/ })).toBeNull();
  expect(screen.queryByRole("button", { name: "Desktop-Joao" })).toBeNull();
  expect(screen.queryByRole("switch", { name: /Microfone/ })).toBeNull();
  expect(screen.getByText("Foco: Este computador")).toBeTruthy();
});
