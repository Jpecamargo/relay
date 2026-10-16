import { invokeCommand, type PowerAction, type RemoteCommand } from "@relay/protocol-types";
import { ConfirmDialog, MenuRow } from "@relay/ui";
import { useState } from "react";
import type { Act } from "../shared/useActions";

const send = (command: RemoteCommand) => invokeCommand("send_command", { command });

const CONFIRM: Record<"Restart" | "Shutdown", { title: string; message: string; label: string }> = {
  Restart: { title: "Reiniciar o PC?", message: "O PC será reiniciado e a conexão cai até ele voltar.", label: "Reiniciar" },
  Shutdown: { title: "Desligar o PC?", message: "O PC será desligado. Para religar use 'Ligar o PC'.", label: "Desligar" },
};

/** Linha "PC ›" que abre, no próprio painel, as ações de energia. Reiniciar/desligar confirmam. */
export function PcMenu({ act, online, canWake }: { act: Act; online: boolean; canWake: boolean }) {
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState<"Restart" | "Shutdown">();
  const power = (label: string, action: PowerAction, confirm: boolean) =>
    act(label, () => send({ Power: { action, confirm } }));
  return (
    <>
      <MenuRow label="PC" icon="monitor" chevron={open ? "open" : true} onClick={() => setOpen((o) => !o)} />
      {open ? (
        <>
          <MenuRow nested label="Bloquear" icon="lock" disabled={!online} onClick={() => void power("Bloquear", "Lock", false)} />
          <MenuRow nested label="Suspender" icon="moon" disabled={!online} onClick={() => void power("Suspender", "Suspend", false)} />
          <MenuRow nested label="Reiniciar" icon="restart" disabled={!online} onClick={() => setPending("Restart")} />
          <MenuRow nested label="Desligar" icon="power" disabled={!online} onClick={() => setPending("Shutdown")} />
          <MenuRow
            nested label="Ligar o PC" icon="wifi" disabled={!canWake}
            onClick={() => void act("Ligar o PC", () => invokeCommand("wake_pc"))}
          />
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
