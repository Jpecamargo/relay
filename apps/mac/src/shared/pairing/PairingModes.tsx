// As duas formas de parear, em qualquer SO: mostrar o código aqui ou digitar o código mostrado no outro computador.
import type { Host } from "@relay/protocol-types";
import { Select, Tabs } from "@relay/ui";
import { useState } from "react";
import { CodeInput } from "./CodeInput";
import { PeerList } from "./PeerList";
import { ShowCode } from "./ShowCode";
import styles from "./pairing.module.css";
import type { usePairing } from "./usePairing";

type Mode = "show" | "enter";
const TABS = [
  { id: "show", label: "Mostrar código" },
  { id: "enter", label: "Digitar código" },
] as const;
const HOST_OPTIONS = [
  { value: "Local", label: "Este computador" },
  { value: "Peer", label: "No outro computador" },
] as const;

export interface PairingModesProps {
  pairing: ReturnType<typeof usePairing>;
  /** Modo ao abrir (o primeiro uso começa digitando). */
  initial?: Mode;
  size?: "md" | "lg";
  autoFocus?: boolean;
  emptyText?: string;
}

export function PairingModes({ pairing: p, initial = "enter", size = "md", autoFocus, emptyText = "Nenhum computador encontrado na rede." }: PairingModesProps) {
  const [mode, setMode] = useState<Mode>(initial);
  return (
    <Tabs tabs={TABS} active={mode} onChange={setMode}>
      {mode === "show" ? (
        <ShowCode size={size} />
      ) : (
        <div className={styles.modes}>
          <PeerList peers={p.peers} selectedId={p.target?.device_id ?? null} onSelect={p.select} emptyText={emptyText} />
          <div className={styles.hostChoice}>
            <Select<Host> label="Guardar os layouts em" value={p.layoutHost} options={HOST_OPTIONS} onChange={p.setLayoutHost} />
          </div>
          <CodeInput size={size} autoFocus={autoFocus} value={p.code} onChange={p.changeCode} onComplete={(c) => void p.pair(c)} disabled={p.busy} invalid={p.error !== ""} />
        </div>
      )}
    </Tabs>
  );
}
