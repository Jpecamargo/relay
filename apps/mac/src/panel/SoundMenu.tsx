import { invokeCommand, type AudioDevice, type AudioDevices, type CommandOutput } from "@relay/protocol-types";
import { MenuRow } from "@relay/ui";
import { useEffect, useState } from "react";
import { send } from "../shared/send";
import type { Act } from "../shared/useActions";
import styles from "./panel.module.css";

const currentOf = (list: ReadonlyArray<AudioDevice>) => list.find((d) => d.is_default)?.id ?? "";

/**
 * Linha "Saída de som ›": saídas do Windows com check na atual e, com "Som do PC no Mac" ligado,
 * o grupo "No Mac" com a saída do stream. A troca é imediata; erro vira toast.
 */
export function SoundMenu({ act, online, streamToMac }: { act: Act; online: boolean; streamToMac: boolean }) {
  const [open, setOpen] = useState(false);
  const [pc, setPc] = useState<AudioDevice[]>([]);
  const [pcCurrent, setPcCurrent] = useState("");
  const [mac, setMac] = useState<AudioDevices>();
  const [macCurrent, setMacCurrent] = useState("");

  // Lê as listas ao abrir: os dispositivos mudam (fone conectado) e o painel vive o dia todo.
  useEffect(() => {
    if (!open) return;
    if (online) {
      void send("ListOutputDevices").then((r) => {
        const out: CommandOutput | null = r.output;
        if (out && "Devices" in out) {
          setPc(out.Devices);
          setPcCurrent(currentOf(out.Devices));
        }
      }).catch(() => undefined);
    }
    void Promise.all([invokeCommand("list_audio_devices"), invokeCommand("get_config")]).then(([d, c]) => {
      setMac(d);
      setMacCurrent(c.audio.mac_output_id ?? currentOf(d.outputs));
    }).catch(() => undefined);
  }, [open, online]);

  const showMac = streamToMac && (mac?.outputs.length ?? 0) > 0;
  return (
    <>
      <MenuRow label="Saída de som" icon="speaker" chevron={open ? "open" : true} onClick={() => setOpen((o) => !o)} />
      {open ? (
        <>
          {online && pc.length > 0 ? <div className={styles.caption}>No Windows</div> : null}
          {online
            ? pc.map((d) => (
                <MenuRow
                  nested key={d.id} label={d.name} selected={d.id === pcCurrent}
                  onClick={() => { setPcCurrent(d.id); void act("Saída do Windows", () => send({ SetOutputDevice: { id: d.id } })); }}
                />
              ))
            : <div className={styles.caption}>PC desconectado</div>}
          {showMac ? <div className={styles.caption}>No Mac</div> : null}
          {showMac
            ? mac!.outputs.map((d) => (
                <MenuRow
                  nested key={d.id} label={d.name} selected={d.id === macCurrent}
                  onClick={() => { setMacCurrent(d.id); void act("Saída do Mac", () => invokeCommand("set_audio_devices", { output_id: d.id })); }}
                />
              ))
            : null}
        </>
      ) : null}
    </>
  );
}
