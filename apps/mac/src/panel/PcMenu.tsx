import { invokeCommand, type PowerAction } from "@relay/protocol-types";
import { ConfirmDialog, MenuRow } from "@relay/ui";
import { useState } from "react";
import { send } from "../shared/send";
import type { Act } from "../shared/useActions";

const confirmOf = (peer: string): Record<"Restart" | "Shutdown", { title: string; message: string; label: string }> => ({
  Restart: { title: `Reiniciar ${peer}?`, message: "Ele será reiniciado e a conexão cai até voltar.", label: "Reiniciar" },
  Shutdown: { title: `Desligar ${peer}?`, message: "Ele será desligado. Para religar use 'Acordar'.", label: "Desligar" },
});

/**
 * Linha com o nome do outro computador que abre, no próprio painel, as ações de energia (COMMANDS) e
 * o acordar (WOL_TARGET). Reiniciar/desligar confirmam.
 */
export function PcMenu({ act, online, peer, canCommand, hasWake, canWake, wakeShortcut }: {
  act: Act; online: boolean; peer: string; canCommand: boolean; hasWake: boolean; canWake: boolean; wakeShortcut?: string;
}) {
  const CONFIRM = confirmOf(peer);
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState<"Restart" | "Shutdown">();
  const power = (label: string, action: PowerAction, confirm: boolean) =>
    act(label, () => send({ Power: { action, confirm } }));
  if (!canCommand && !hasWake) return null;
  return (
    <>
      <MenuRow label={peer} icon="monitor" chevron={open ? "open" : true} onClick={() => setOpen((o) => !o)} />
      {open ? (
        <>
          {canCommand ? (
            <>
              <MenuRow nested label="Bloquear" icon="lock" disabled={!online} onClick={() => void power("Bloquear", "Lock", false)} />
              <MenuRow nested label="Suspender" icon="moon" disabled={!online} onClick={() => void power("Suspender", "Suspend", false)} />
              <MenuRow nested label="Reiniciar" icon="restart" disabled={!online} onClick={() => setPending("Restart")} />
              <MenuRow nested label="Desligar" icon="power" disabled={!online} onClick={() => setPending("Shutdown")} />
            </>
          ) : null}
          {hasWake ? (
            <MenuRow
              nested label="Acordar" icon="wifi" disabled={!canWake} shortcut={wakeShortcut}
              onClick={() => void act("Acordar", () => invokeCommand("wake_peer"))}
            />
          ) : null}
        </>
      ) : null}
      <ConfirmDialog
        open={pending !== undefined}
        destructive
        title={pending ? CONFIRM[pending].title : ""}
        message={pending ? CONFIRM[pending].message : undefined}
        confirmLabel={pending ? CONFIRM[pending].label : undefined}
        onCancel={() => setPending(undefined)}
        onConfirm={() => {
          const action = pending;
          setPending(undefined);
          if (action) void power(CONFIRM[action].label, action, true);
        }}
      />
    </>
  );
}
