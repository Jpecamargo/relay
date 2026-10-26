import type { AudioDevice, CommandOutput } from "@relay/protocol-types";
import { MenuRow } from "@relay/ui";
import { useEffect, useState } from "react";
import { send } from "../shared/send";
import type { Act } from "../shared/useActions";
import styles from "./panel.module.css";

const currentOf = (list: ReadonlyArray<AudioDevice>) => list.find((d) => d.is_default)?.id ?? "";

/** Linha "Saída de som ›": saídas do outro computador com check na atual. A troca é imediata; erro vira toast. */
export function SoundMenu({ act, online, peer }: { act: Act; online: boolean; peer: string }) {
  const [open, setOpen] = useState(false);
  const [pc, setPc] = useState<AudioDevice[]>([]);
  const [pcCurrent, setPcCurrent] = useState("");

  // Lê a lista ao abrir: os dispositivos mudam (fone conectado) e o painel vive o dia todo.
  useEffect(() => {
    if (!open || !online) return;
    void send("ListOutputDevices").then((r) => {
      const out: CommandOutput | null = r.output;
      if (out && "Devices" in out) {
        setPc(out.Devices);
        setPcCurrent(currentOf(out.Devices));
      }
    }).catch(() => undefined);
  }, [open, online]);

  return (
    <>
      <MenuRow label="Saída de som" icon="speaker" chevron={open ? "open" : true} onClick={() => setOpen((o) => !o)} />
      {open
        ? online
          ? pc.map((d) => (
              <MenuRow
                nested key={d.id} label={d.name} selected={d.id === pcCurrent}
                onClick={() => { setPcCurrent(d.id); void act(`Saída de ${peer}`, () => send({ SetOutputDevice: { id: d.id } })); }}
              />
            ))
          : <div className={styles.caption}>{peer} desconectado</div>
        : null}
    </>
  );
}
