// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { UpdaterProvider } from "@relay/ui";
import { App } from "./App";

const bridge = vi.hoisted(() => ({ invoke: vi.fn(), listeners: new Map<string, (p: unknown) => void>() }));

vi.mock("@relay/protocol-types", () => ({
  invokeCommand: bridge.invoke,
  listenEvent: async (name: string, cb: (p: unknown) => void) => {
    bridge.listeners.set(name, cb);
    return () => bridge.listeners.delete(name);
  },
}));

const config = {
  audio: { jitter_target_ms: 20, mac_input_id: null },
  clipboard: { enabled: true },
  general: { launch_at_login: false },
};

function setup(snapshot: Record<string, unknown> = { status: "Connected", peer_name: "MacBook", focus: "Mac" }) {
  bridge.invoke.mockImplementation(async (name: string) => {
    switch (name) {
      case "agent_get_status":
        return snapshot;
      case "agent_get_service_status":
        return { installed: true, running: true, helper_active: true, vbcable_detected: false };
      case "agent_get_config":
        return config;
      case "agent_new_pairing_code":
        return { code: "123456", expires_at: Date.now() + 120_000 };
      default:
        return undefined;
    }
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  bridge.listeners.clear();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

test("o código é gerado ao abrir e a contagem regressiva o esconde ao expirar", async () => {
  setup();
  render(<UpdaterProvider><App /></UpdaterProvider>);
  await act(async () => {});
  expect(bridge.invoke).toHaveBeenCalledWith("agent_new_pairing_code");
  expect(screen.getByLabelText("Código de pareamento").textContent).toBe("123456");
  expect(screen.getByText("2:00")).toBeTruthy();

  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000);
  });
  expect(screen.getByText("1:00")).toBeTruthy();

  await act(async () => {
    await vi.advanceTimersByTimeAsync(61_000);
  });
  expect(screen.queryByLabelText("Código de pareamento")).toBeNull();
  expect(screen.getByText("O código expirou.")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Gerar código" })).toBeTruthy();
});

test("revogar exige confirmação", async () => {
  setup();
  render(<UpdaterProvider><App /></UpdaterProvider>);
  await act(async () => {});

  fireEvent.click(screen.getByRole("button", { name: "Revogar" }));
  expect(bridge.invoke).not.toHaveBeenCalledWith("agent_revoke_pair");

  fireEvent.click(screen.getByRole("button", { name: "Cancelar" }));
  expect(bridge.invoke).not.toHaveBeenCalledWith("agent_revoke_pair");

  fireEvent.click(screen.getByRole("button", { name: "Revogar" }));
  const dialog = screen.getByRole("alertdialog");
  await act(async () => {
    fireEvent.click(dialog.querySelector("button:last-child")!);
  });
  expect(bridge.invoke).toHaveBeenCalledWith("agent_revoke_pair");
});

test("relay:pairing paired fecha o fluxo e esconde o código", async () => {
  setup({ status: "Pairing", peer_name: null, focus: "Windows" });
  render(<UpdaterProvider><App /></UpdaterProvider>);
  await act(async () => {});
  expect(screen.getByLabelText("Código de pareamento")).toBeTruthy();
  await act(async () => {
    bridge.listeners.get("relay:pairing")!({ state: "paired", code: null, expires_at: null });
  });
  expect(screen.queryByLabelText("Código de pareamento")).toBeNull();
  expect(screen.getByText("Pareado com sucesso.")).toBeTruthy();
});

test("Ajustes traz o bloco Sobre com a atualização manual desabilitada fora do app instalado", async () => {
  setup();
  render(<UpdaterProvider><App /></UpdaterProvider>);
  await act(async () => {});
  expect(screen.getByRole("heading", { name: "Sobre" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Procurar atualizações" }).hasAttribute("disabled")).toBe(true);
});

test("sem Mac pareado o código aparece grande, dígito a dígito, com a contagem, antes do status", async () => {
  setup({ status: "Pairing", peer_name: null, focus: "Windows" });
  render(<UpdaterProvider><App /></UpdaterProvider>);
  await act(async () => {});
  expect(screen.getByRole("heading", { name: "Conecte seu Mac" })).toBeTruthy();
  const code = screen.getByLabelText("Código de pareamento");
  expect([...code.children].map((d) => d.textContent)).toEqual(["1", "2", "3", "4", "5", "6"]);
  expect(screen.getByText("2:00")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Revogar" })).toBeNull();
});
