// @vitest-environment jsdom
import type { StatusSnapshot } from "@relay/protocol-types";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { Panel } from "./index";

const bridge = vi.hoisted(() => ({
  listeners: new Map<string, (p: unknown) => void>(),
  invoke: vi.fn(),
}));

vi.mock("@relay/protocol-types", () => ({
  listenEvent: async (name: string, cb: (p: unknown) => void) => {
    bridge.listeners.set(name, cb);
    return () => bridge.listeners.delete(name);
  },
  invokeCommand: bridge.invoke,
}));

const STATUS: StatusSnapshot = {
  status: "Connected", peer_name: "Desktop", focus: "Mac", locked: false, paused: false,
  secure_input: false, clipboard_sync: false, audio: { pc_to_mac: false, mac_to_pc: false },
};

let sendResult: unknown;

beforeEach(() => {
  bridge.listeners.clear();
  bridge.invoke.mockReset();
  sendResult = { id: 1, ok: true, output: null, error: null };
  bridge.invoke.mockImplementation(async (name: string) => {
    switch (name) {
      case "get_status": return STATUS;
      case "check_permissions": return { accessibility: "Granted", input_monitoring: "Granted" };
      case "check_updates": return null;
      case "list_audio_devices": return { outputs: [], inputs: [] };
      case "get_config": return { audio: { stream_volume: 1 } };
      case "get_displays": return { mac: [], windows: [] };
      case "send_command": return sendResult;
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
  await waitFor(() => expect(screen.getByRole("button", { name: "Bloquear" }).hasAttribute("disabled")).toBe(false));
}

test("Bloquear envia Power Lock sem confirmação e mostra toast de sucesso", async () => {
  await ready();
  fireEvent.click(screen.getByRole("button", { name: "Bloquear" }));
  await waitFor(() =>
    expect(bridge.invoke).toHaveBeenCalledWith("send_command", { command: { Power: { action: "Lock", confirm: false } } }),
  );
  expect(await screen.findByText("Bloquear: ok")).toBeTruthy();
});

test("Reiniciar só envia depois de confirmar, com confirm:true", async () => {
  await ready();
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
  await ready();
  sendResult = { id: 2, ok: false, output: null, error: "timeout" };
  fireEvent.click(screen.getByRole("button", { name: "Suspender" }));
  const alert = await screen.findByRole("alert");
  expect(alert.textContent).toContain("O PC não respondeu a tempo.");
});

test("evento de latência atualiza o texto", async () => {
  render(<Panel />);
  await waitFor(() => expect(bridge.listeners.has("relay:latency")).toBe(true));
  await emit("relay:latency", { rtt_ms: 4.2, jitter_ms: 0.8, loss_pct: 0 });
  expect(screen.getByText(/4\.2 ms · jitter 0\.8 ms/)).toBeTruthy();
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
    fireEvent.change(screen.getByLabelText("Volume do Windows"), { target: { value: "0.3" } });
    expect(sentWith("SetVolume")).toHaveLength(0);
    await act(async () => void vi.advanceTimersByTime(50));
    expect(bridge.invoke).toHaveBeenCalledWith("send_command", { command: { SetVolume: { level: 0.3 } } });
  } finally {
    vi.useRealTimers();
  }
});

test("banner de reconexão tem 'Tentar de novo' que chama reconnect", async () => {
  await ready();
  emit("relay:status", { ...STATUS, status: "Reconnecting" });
  fireEvent.click(await screen.findByRole("button", { name: "Tentar de novo" }));
  await waitFor(() => expect(bridge.invoke).toHaveBeenCalledWith("reconnect"));
});

test("relay:command-result de áudio mostra toast e desliga o toggle", async () => {
  await ready();
  await emit("relay:command-result", { id: 0, ok: false, output: null, error: "vbcable_missing" });
  expect((await screen.findByRole("alert")).textContent).toContain("VB-Cable");
  await waitFor(() =>
    expect(bridge.invoke).toHaveBeenCalledWith("set_audio_streams", { pc_to_mac: false, mac_to_pc: false }),
  );
});

test("resultado igual por invoke e por evento gera um toast só", async () => {
  await ready();
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
