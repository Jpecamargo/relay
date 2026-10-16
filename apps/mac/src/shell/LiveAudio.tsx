import {
  invokeCommand,
  type AudioDevice,
  type AudioDevices,
  type AudioStats,
  type AudioState,
  type CommandOutput,
} from "@relay/protocol-types";
import { Section, Select, Slider, Toggle } from "@relay/ui";
import { useEffect, useState } from "react";
import { send } from "../shared/send";
import { decimal } from "../shared/status";
import type { Act } from "../shared/useActions";
import styles from "./main.module.css";

const deviceOptions = (list: ReadonlyArray<AudioDevice>) => list.map((d) => ({ value: d.id, label: d.name }));
const defaultId = (list: ReadonlyArray<AudioDevice>) => list.find((d) => d.is_default)?.id ?? list[0]?.id ?? "";

/** Áudio ao vivo: streams, dispositivos e volume. Vale na hora (não passa pelo "Salvar"). */
export function LiveAudio({ act, online, audio, stats, refresh }: {
  act: Act;
  online: boolean;
  audio: AudioState;
  stats: AudioStats | undefined;
  refresh: () => void;
}) {
  const [macDevices, setMacDevices] = useState<AudioDevices>();
  const [pcOutputs, setPcOutputs] = useState<AudioDevice[]>([]);
  const [pcOutput, setPcOutput] = useState("");
  const [macOut, setMacOut] = useState("");
  const [macIn, setMacIn] = useState("");
  const [streamVolume, setStreamVolume] = useState(1);

  useEffect(() => {
    invokeCommand("list_audio_devices").then((d) => {
      setMacDevices(d);
      setMacOut(defaultId(d.outputs));
      setMacIn(defaultId(d.inputs));
    }).catch(() => undefined);
    invokeCommand("get_config").then((c) => setStreamVolume(c.audio.stream_volume)).catch(() => undefined);
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

  const streams = (label: string, next: AudioState) =>
    void act(label, () => invokeCommand("set_audio_streams", next)).then(refresh);

  return (
    <>
      <Section title="Streams" grouped>
        <Toggle label="Som do PC no Mac" checked={audio.pc_to_mac} onChange={(v) => streams("Som do PC no Mac", { ...audio, pc_to_mac: v })} />
        <Toggle label="Microfone do Mac no PC" checked={audio.mac_to_pc} onChange={(v) => streams("Microfone do Mac no PC", { ...audio, mac_to_pc: v })} />
        <Slider
          label="Volume do stream" value={streamVolume} min={0} max={1} step={0.01} debounceMs={50}
          format={(v) => `${Math.round(v * 100)}%`}
          onChange={(v) => { setStreamVolume(v); void act("Volume do stream", () => invokeCommand("set_stream_volume", { volume: v })); }}
        />
        <div className={styles.infoRow}>
          <span>Latência do áudio</span>
          <b>{stats ? `${Math.round(stats.latency_ms)} ms · perda ${decimal(stats.loss_pct)}%` : "sem stream ativo"}</b>
        </div>
      </Section>
      <Section title="Dispositivos" grouped>
        <Select
          label="Saída do Windows" value={pcOutput} options={deviceOptions(pcOutputs)} disabled={!online || pcOutputs.length === 0}
          onChange={(id) => { setPcOutput(id); void act("Saída do Windows", () => send({ SetOutputDevice: { id } })); }}
        />
        <Select
          label="Saída do Mac" value={macOut} options={deviceOptions(macDevices?.outputs ?? [])} disabled={!macDevices?.outputs.length}
          onChange={(id) => { setMacOut(id); void act("Saída do Mac", () => invokeCommand("set_audio_devices", { output_id: id })); }}
        />
        <Select
          label="Entrada do Mac" value={macIn} options={deviceOptions(macDevices?.inputs ?? [])} disabled={!macDevices?.inputs.length}
          onChange={(id) => { setMacIn(id); void act("Entrada do Mac", () => invokeCommand("set_audio_devices", { input_id: id })); }}
        />
      </Section>
    </>
  );
}
