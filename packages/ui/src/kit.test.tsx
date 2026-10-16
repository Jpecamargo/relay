import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { ConfirmDialog, KeyCombo, MenuRow, Slider, Toggle, useRelayCommand, useRelayEvent } from "./index";

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

beforeEach(() => bridge.listeners.clear());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

test("Toggle alterna e chama onChange", () => {
  const onChange = vi.fn();
  render(<Toggle label="Clipboard" checked={false} onChange={onChange} />);
  const sw = screen.getByRole("switch", { name: "Clipboard" });
  expect(sw.getAttribute("aria-checked")).toBe("false");
  fireEvent.click(sw);
  expect(onChange).toHaveBeenCalledExactlyOnceWith(true);
});

test("Slider faz debounce configurável", () => {
  vi.useFakeTimers();
  const onChange = vi.fn();
  render(<Slider label="Volume" value={10} onChange={onChange} debounceMs={200} />);
  const range = screen.getByLabelText("Volume");
  fireEvent.change(range, { target: { value: "20" } });
  fireEvent.change(range, { target: { value: "30" } });
  act(() => void vi.advanceTimersByTime(199));
  expect(onChange).not.toHaveBeenCalled();
  act(() => void vi.advanceTimersByTime(1));
  expect(onChange).toHaveBeenCalledExactlyOnceWith(30);
});

test("ConfirmDialog só chama onConfirm depois de confirmar", () => {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(<ConfirmDialog open title="Desparear?" onConfirm={onConfirm} onCancel={onCancel} />);
  expect(onConfirm).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Cancelar" }));
  expect(onConfirm).not.toHaveBeenCalled();
  expect(onCancel).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole("button", { name: "Confirmar" }));
  expect(onConfirm).toHaveBeenCalledOnce();
});

test("ConfirmDialog fechado não renderiza", () => {
  render(<ConfirmDialog open={false} title="x" onConfirm={vi.fn()} onCancel={vi.fn()} />);
  expect(screen.queryByRole("alertdialog")).toBeNull();
});

test("useRelayEvent atualiza com evento mockado", async () => {
  function Probe() {
    const latency = useRelayEvent("relay:latency");
    return <span>{latency ? `${latency.rtt_ms} ms` : "sem dado"}</span>;
  }
  render(<Probe />);
  expect(screen.getByText("sem dado")).toBeTruthy();
  await act(async () => {});
  act(() => bridge.listeners.get("relay:latency")?.({ rtt_ms: 7, jitter_ms: 1, loss_pct: 0 }));
  expect(screen.getByText("7 ms")).toBeTruthy();
});

test("useRelayCommand expõe resultado e erro", async () => {
  bridge.invoke.mockResolvedValueOnce(null).mockRejectedValueOnce("falhou");
  function Probe() {
    const cmd = useRelayCommand("check_updates");
    return (
      <>
        <button onClick={() => cmd.run().catch(() => undefined)}>go</button>
        <span>{cmd.error ?? (cmd.pending ? "..." : "ok")}</span>
      </>
    );
  }
  render(<Probe />);
  await act(async () => fireEvent.click(screen.getByText("go")));
  expect(bridge.invoke).toHaveBeenCalledWith("check_updates");
  await act(async () => fireEvent.click(screen.getByText("go")));
  expect(screen.getByText("falhou")).toBeTruthy();
});

test("KeyCombo mostra modificadores em ordem e a tecla", () => {
  render(<KeyCombo combo={{ modifiers: ["Cmd", "Shift"], key: "KeyL" }} />);
  expect(screen.getByLabelText("Shift Cmd L")).toBeTruthy();
});

test("MenuRow como alternância expõe switch e inverte pelo clique", () => {
  const onClick = vi.fn();
  render(<MenuRow label="Travar cursor" icon="lock" checked={false} onClick={onClick} />);
  const row = screen.getByRole("switch", { name: "Travar cursor" });
  expect(row.getAttribute("aria-checked")).toBe("false");
  fireEvent.click(row);
  expect(onClick).toHaveBeenCalledOnce();
});

test("MenuRow de ação mostra o atalho e some o switch", () => {
  render(<MenuRow label="Ajustes…" icon="gear" shortcut="⌘," />);
  expect(screen.getByRole("button", { name: /Ajustes/ }).textContent).toContain("⌘,");
  expect(screen.queryByRole("switch")).toBeNull();
});
