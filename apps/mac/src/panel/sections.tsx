import {
  invokeCommand,
  type AudioDevice,
  type AudioDevices,
  type AudioStats,
  type AudioState,
  type CommandOutput,
  type DisplayInfo,
  type MediaAction,
  type PowerAction,
  type RemoteCommand,
  type StatusSnapshot,
  type SystemMetrics,
  type TransferProgress,
} from "@relay/protocol-types";
import { Button, ConfirmDialog, ProgressBar, Section, Select, Slider, Toggle, useRelayEvent } from "@relay/ui";
import { useEffect, useRef, useState, type FormEvent } from "react";
import styles from "./panel.module.css";

/** Executa uma ação e mostra o resultado (sucesso ou erro) como toast. */
export type Act = <T>(label: string, fn: () => Promise<T>) => Promise<T | undefined>;

const send = (command: RemoteCommand) => invokeCommand("send_command", { command });
const DEFAULT = "";

function deviceOptions(list: ReadonlyArray<AudioDevice>) {
  return list.map((d) => ({ value: d.id, label: d.name }));
}
const defaultId = (list: ReadonlyArray<AudioDevice>) => list.find((d) => d.is_default)?.id ?? list[0]?.id ?? DEFAULT;

interface Common {
  act: Act;
  refresh: () => void;
  /** Comandos que vão ao PC só valem com a conexão ativa. */
  online: boolean;
}

export function AudioSection({
  act, refresh, online, audio, stats, macDevices, streamVolume, setStreamVolume,
}: Common & {
  audio: AudioState;
  stats?: AudioStats;
  macDevices?: AudioDevices;
  streamVolume: number;
  setStreamVolume: (v: number) => void;
}) {
  const [volume, setVolume] = useState(0.5);
  const [muted, setMuted] = useState(false);
  // Volume e mudo reais do PC chegam nas métricas: inicializam o controle uma vez.
  const metrics = useRelayEvent("relay:metrics");
  const seeded = useRef(false);
  useEffect(() => {
    if (seeded.current || !metrics) return;
    if (metrics.volume != null) setVolume(metrics.volume);
    if (metrics.muted != null) setMuted(metrics.muted);
    if (metrics.volume != null || metrics.muted != null) seeded.current = true;
  }, [metrics]);
  const [pcOutputs, setPcOutputs] = useState<AudioDevice[]>([]);
  const [pcOutput, setPcOutput] = useState(DEFAULT);
  const [macOut, setMacOut] = useState(DEFAULT);
  const [macIn, setMacIn] = useState(DEFAULT);

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

  useEffect(() => {
    if (!macDevices) return;
    setMacOut(defaultId(macDevices.outputs));
    setMacIn(defaultId(macDevices.inputs));
  }, [macDevices]);

  return (
    <Section title="Áudio">
      <Slider
        label="Volume do Windows" value={volume} min={0} max={1} step={0.01} debounceMs={50}
        format={(v) => `${Math.round(v * 100)}%`} disabled={!online}
        onChange={(v) => { setVolume(v); void act("Volume", () => send({ SetVolume: { level: v } })); }}
      />
      <Toggle
        label="Mudo no Windows" checked={muted} disabled={!online}
        onChange={(m) => { setMuted(m); void act("Mudo", () => send({ SetMute: { muted: m } })); }}
      />
      <Select
        label="Saída do Windows" value={pcOutput} options={deviceOptions(pcOutputs)} disabled={!online || pcOutputs.length === 0}
        onChange={(id) => { setPcOutput(id); void act("Saída do Windows", () => send({ SetOutputDevice: { id } })); }}
      />
      <Toggle
        label="Som do PC no Mac" checked={audio.pc_to_mac}
        onChange={(v) => void act("Som do PC no Mac", () => invokeCommand("set_audio_streams", { pc_to_mac: v, mac_to_pc: audio.mac_to_pc })).then(refresh)}
      />
      <Toggle
        label="Microfone do Mac no PC" checked={audio.mac_to_pc}
        onChange={(v) => void act("Microfone do Mac no PC", () => invokeCommand("set_audio_streams", { pc_to_mac: audio.pc_to_mac, mac_to_pc: v })).then(refresh)}
      />
      <Select
        label="Saída do Mac" value={macOut} options={deviceOptions(macDevices?.outputs ?? [])} disabled={!macDevices?.outputs.length}
        onChange={(id) => { setMacOut(id); void act("Saída do Mac", () => invokeCommand("set_audio_devices", { output_id: id })); }}
      />
      <Select
        label="Entrada do Mac" value={macIn} options={deviceOptions(macDevices?.inputs ?? [])} disabled={!macDevices?.inputs.length}
        onChange={(id) => { setMacIn(id); void act("Entrada do Mac", () => invokeCommand("set_audio_devices", { input_id: id })); }}
      />
      <Slider
        label="Volume do stream" value={streamVolume} min={0} max={1} step={0.01} debounceMs={50}
        format={(v) => `${Math.round(v * 100)}%`}
        onChange={(v) => { setStreamVolume(v); void act("Volume do stream", () => invokeCommand("set_stream_volume", { volume: v })); }}
      />
      <div className={styles.stat}>
        <span>Latência de áudio</span>
        <b>{stats ? `${Math.round(stats.latency_ms)} ms · perda ${stats.loss_pct.toFixed(1)}%` : "—"}</b>
      </div>
    </Section>
  );
}

export function MediaSection({ act, online }: Common) {
  const media = (label: string, action: MediaAction) => () => void act(label, () => send({ Media: { action } }));
  return (
    <Section title="Mídia">
      <div className={styles.row}>
        <Button disabled={!online} onClick={media("Anterior", "Previous")}>Anterior</Button>
        <Button disabled={!online} onClick={media("Play/pause", "PlayPause")}>Play/pause</Button>
        <Button disabled={!online} onClick={media("Próxima", "Next")}>Próxima</Button>
      </div>
    </Section>
  );
}

const CONFIRM: Record<"Restart" | "Shutdown", { title: string; message: string; label: string }> = {
  Restart: { title: "Reiniciar o PC?", message: "O PC será reiniciado e a conexão cai até ele voltar.", label: "Reiniciar" },
  Shutdown: { title: "Desligar o PC?", message: "O PC será desligado. Para religar use 'Ligar o PC'.", label: "Desligar" },
};

export function PowerSection({ act, online, canWake }: Common & { canWake: boolean }) {
  const [pending, setPending] = useState<"Restart" | "Shutdown">();
  const power = (label: string, action: PowerAction, confirm: boolean) =>
    act(label, () => send({ Power: { action, confirm } }));
  return (
    <Section title="Energia">
      <div className={styles.row}>
        <Button disabled={!online} onClick={() => void power("Bloquear", "Lock", false)}>Bloquear</Button>
        <Button disabled={!online} onClick={() => void power("Suspender", "Suspend", false)}>Suspender</Button>
      </div>
      <div className={styles.row}>
        <Button variant="danger" disabled={!online} onClick={() => setPending("Restart")}>Reiniciar</Button>
        <Button variant="danger" disabled={!online} onClick={() => setPending("Shutdown")}>Desligar</Button>
        <Button disabled={!canWake} onClick={() => void act("Ligar o PC", () => invokeCommand("wake_pc"))}>Ligar o PC</Button>
      </div>
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
    </Section>
  );
}

export function CursorClipboardSections({ act, refresh, status }: Common & { status: StatusSnapshot }) {
  return (
    <>
      <Section title="Cursor">
        <Toggle
          label="Travar cursor" checked={status.locked}
          onChange={(locked) => void act("Travar cursor", () => invokeCommand("set_cursor_lock", { locked })).then(refresh)}
        />
        <Toggle
          label="Pausar compartilhamento" checked={status.paused}
          onChange={(paused) => void act("Pausar", () => invokeCommand("set_sharing_paused", { paused })).then(refresh)}
        />
      </Section>
      <Section title="Clipboard">
        <Toggle
          label="Sincronizar clipboard" checked={status.clipboard_sync}
          onChange={(enabled) => void act("Clipboard", () => invokeCommand("set_clipboard_sync", { enabled })).then(refresh)}
        />
        <Button onClick={() => void act("Clipboard enviado", () => invokeCommand("clipboard_send_now"))}>Enviar agora</Button>
      </Section>
    </>
  );
}

export function SystemSection({ metrics }: { metrics?: SystemMetrics }) {
  const gb = (mb: number) => (mb / 1024).toFixed(1);
  const temp = metrics?.gpu_temp_c ?? metrics?.cpu_temp_c;
  return (
    <Section title="Sistema">
      <div className={styles.stat}><span>CPU</span><b>{metrics ? `${Math.round(metrics.cpu_pct)}%` : "—"}</b></div>
      <div className={styles.stat}><span>RAM</span><b>{metrics ? `${gb(metrics.ram_used_mb)} / ${gb(metrics.ram_total_mb)} GB` : "—"}</b></div>
      <div className={styles.stat}><span>GPU</span><b>{metrics?.gpu_pct != null ? `${Math.round(metrics.gpu_pct)}%` : "—"}</b></div>
      <div className={styles.stat}><span>Temperatura</span><b>{temp != null ? `${Math.round(temp)} °C` : "—"}</b></div>
    </Section>
  );
}

export function UtilitiesSection({ act, online }: Common) {
  const [displays, setDisplays] = useState<DisplayInfo[]>([]);
  const [display, setDisplay] = useState(DEFAULT);
  const [shot, setShot] = useState<string>();
  const [target, setTarget] = useState("");
  const [text, setText] = useState("");

  useEffect(() => {
    void invokeCommand("get_displays")
      .then((d) => { setDisplays(d.windows); setDisplay(d.windows[0]?.id ?? DEFAULT); })
      .catch(() => undefined);
  }, []);

  const capture = async () => {
    const r = await act("Captura de tela", () => send({ CaptureScreen: { display_id: display } }));
    const out = r?.ok ? r.output : null;
    if (out && "Png" in out) setShot(`data:image/png;base64,${bytesToBase64(out.Png)}`);
  };
  const submit = (run: () => void) => (e: FormEvent) => { e.preventDefault(); run(); };

  return (
    <Section title="Utilidades">
      <Select
        label="Tela do PC" value={display} disabled={!online || displays.length === 0}
        options={displays.map((d) => ({ value: d.id, label: d.name }))} onChange={setDisplay}
      />
      <Button disabled={!online || !display} onClick={() => void capture()}>Capturar tela</Button>
      {shot ? <img className={styles.shot} src={shot} alt="Captura da tela do PC" /> : null}
      <form className={styles.field} onSubmit={submit(() => { void act("Abrir", () => send({ OpenTarget: { target } })); })}>
        <input className={styles.input} aria-label="URL ou app" placeholder="URL ou app" value={target} onChange={(e) => setTarget(e.target.value)} />
        <Button type="submit" disabled={!online || !target.trim()}>Abrir</Button>
      </form>
      <form className={styles.field} onSubmit={submit(() => { void act("Texto enviado", () => send({ TypeText: { text } })); })}>
        <input className={styles.input} aria-label="Texto para digitar no PC" placeholder="Texto para o PC" value={text} onChange={(e) => setText(e.target.value)} />
        <Button type="submit" disabled={!online || !text}>Enviar texto</Button>
      </form>
    </Section>
  );
}

function bytesToBase64(bytes: ReadonlyArray<number>): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}

const mb = (n: number) => `${(n / 1048576).toFixed(1)} MB`;

export function TransfersSection({ act, transfers }: Pick<Common, "act"> & { transfers: TransferProgress[] }) {
  return (
    <Section title="Transferências">
      {transfers.length === 0 ? <span className={styles.muted}>Nenhuma transferência.</span> : null}
      {transfers.map((t) => (
        <div key={t.transfer_id} className={styles.transfer}>
          <div className={styles.transferHead}>
            <span className={styles.name}>{t.file_name}</span>
            <span>{t.error ? "falhou" : t.done ? "concluída" : `${mb(t.bytes)} / ${mb(t.total)}`}</span>
          </div>
          {t.error ? <span className={styles.err}>{t.error}</span> : <ProgressBar label={`Progresso de ${t.file_name}`} value={t.bytes} max={t.total} />}
        </div>
      ))}
      <Button onClick={() => void act("Receber arquivos", () => invokeCommand("receive_remote_files"))}>Receber arquivos</Button>
    </Section>
  );
}
