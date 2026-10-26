import {
  invokeCommand,
  type AudioDevice,
  type AudioDevices,
  type AudioState,
  type CommandOutput,
} from "@relay/protocol-types";
import { Section, Select, Toggle } from "@relay/ui";
import { useEffect, useState } from "react";
import { send } from "../shared/send";
import type { Act } from "../shared/useActions";

const deviceOptions = (list: ReadonlyArray<AudioDevice>) => list.map((d) => ({ value: d.id, label: d.name }));
const defaultId = (list: ReadonlyArray<AudioDevice>) => list.find((d) => d.is_default)?.id ?? list[0]?.id ?? "";

/** Áudio ao vivo: microfone e dispositivos. Vale na hora (não passa pelo "Salvar"). */
export function LiveAudio({ act, online, audio, refresh }: {
  act: Act;
  online: boolean;
  audio: AudioState;
  refresh: () => void;
}) {
  const [macDevices, setMacDevices] = useState<AudioDevices>();
  const [pcOutputs, setPcOutputs] = useState<AudioDevice[]>([]);
  const [pcOutput, setPcOutput] = useState("");
  const [macIn, setMacIn] = useState("");

  useEffect(() => {
    invokeCommand("list_audio_devices").then((d) => {
      setMacDevices(d);
      setMacIn(defaultId(d.inputs));
    }).catch(() => undefined);
  }, []);

  useEffect(() => {
    if (!online) return;
    void send("ListOutputDevices").then((r) => {
      const out: CommandOutput | null = r.output;
      if (out && "Devices" in out) {
        setPcOutputs(out.Devices);
        setPcOutput(defaultId(out.Devices));
      }
    }).catch(() => undefined);
  }, [online]);

  const streams = (label: string, micToPeer: boolean) =>
    void act(label, () => invokeCommand("set_audio_streams", { mic_to_peer: micToPeer })).then(refresh);

  return (
    <>
      <Section title="Microfone" grouped>
        <Toggle label="Microfone do Mac no PC" checked={audio.mic_to_peer} onChange={(v) => streams("Microfone do Mac no PC", v)} />
      </Section>
      <Section title="Dispositivos" grouped>
        <Select
          label="Saída do Windows" value={pcOutput} options={deviceOptions(pcOutputs)} disabled={!online || pcOutputs.length === 0}
          onChange={(id) => { setPcOutput(id); void act("Saída do Windows", () => send({ SetOutputDevice: { id } })); }}
        />
        <Select
          label="Entrada do Mac" value={macIn} options={deviceOptions(macDevices?.inputs ?? [])} disabled={!macDevices?.inputs.length}
          onChange={(id) => { setMacIn(id); void act("Entrada do Mac", () => invokeCommand("set_audio_devices", { input_id: id })); }}
        />
      </Section>
    </>
  );
}
