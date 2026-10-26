import {
  CAPS,
  hasCap,
  invokeCommand,
  type DisplayInfo,
  type LatencyStats,
  type StatusSnapshot,
  type SystemMetrics,
  type TransferProgress,
} from "@relay/protocol-types";
import { Button, LatencySparkline, ProgressBar, Section, Select, StatusDot } from "@relay/ui";
import { useEffect, useState, type FormEvent } from "react";
import { decimal, STATUS_LABELS, statusKey, statusTone } from "../shared/status";
import { send } from "../shared/send";
import type { Act } from "../shared/useActions";
import styles from "./main.module.css";

export interface OverviewProps {
  act: Act;
  status: StatusSnapshot;
  latency: LatencyStats | undefined;
  history: number[];
  metrics: SystemMetrics | undefined;
  transfers: TransferProgress[];
}

export function Overview({ act, status, latency, history, metrics, transfers }: OverviewProps) {
  const key = statusKey(status.status);
  const online = key === "Connected";
  return (
    <>
      <div className={styles.hero}>
        <div>
          <div className={styles.heroTitle}>
            <StatusDot tone={statusTone(key)} label={STATUS_LABELS[key] ?? key} />
            {STATUS_LABELS[key] ?? key}
          </div>
          <p className={styles.heroSub}>
            {status.peer_name ? `${status.peer_name} · ` : ""}foco {status.focus === "Local" ? "neste computador" : "no outro computador"}
          </p>
        </div>
        {latency && online ? (
          <div className={styles.latency}>
            <span className={styles.latencyValue}>{decimal(latency.rtt_ms)}<small> ms</small></span>
            <LatencySparkline values={history} width={140} height={28} hideValue />
            <span className={styles.heroSub}>jitter {decimal(latency.jitter_ms)} ms · perda {decimal(latency.loss_pct)}%</span>
          </div>
        ) : null}
      </div>

      {hasCap(status.peer_caps, CAPS.METRICS) ? <PeerMetrics metrics={metrics} /> : null}
      <Transfers act={act} transfers={transfers} canPull={hasCap(status.local_caps, CAPS.FILE_PULL)} />
      {hasCap(status.peer_caps, CAPS.COMMANDS) ? (
        <Utilities act={act} online={online} canCapture={hasCap(status.peer_caps, CAPS.SCREEN_CAPTURE)} />
      ) : null}
    </>
  );
}

const gb = (mb: number) => decimal(mb / 1024);

function Meter({ label, value, ratio }: { label: string; value: string; ratio: number | undefined }) {
  return (
    <div className={styles.meter}>
      <div className={styles.meterHead}><span>{label}</span><b>{value}</b></div>
      {ratio !== undefined ? <ProgressBar label={label} value={ratio} max={1} /> : null}
    </div>
  );
}

function PeerMetrics({ metrics }: { metrics: SystemMetrics | undefined }) {
  if (!metrics) {
    return (
      <Section title="Outro computador">
        <p className={styles.empty}>Sem métricas: conecte ao outro computador para ver CPU, memória e GPU.</p>
      </Section>
    );
  }
  const temp = metrics.gpu_temp_c ?? metrics.cpu_temp_c;
  return (
    <Section title="Outro computador" grouped>
      <Meter label="CPU" value={`${Math.round(metrics.cpu_pct)}%`} ratio={metrics.cpu_pct / 100} />
      <Meter
        label="Memória" value={`${gb(metrics.ram_used_mb)} de ${gb(metrics.ram_total_mb)} GB`}
        ratio={metrics.ram_total_mb > 0 ? metrics.ram_used_mb / metrics.ram_total_mb : undefined}
      />
      {metrics.gpu_pct != null ? <Meter label="GPU" value={`${Math.round(metrics.gpu_pct)}%`} ratio={metrics.gpu_pct / 100} /> : null}
      {temp != null ? <Meter label="Temperatura" value={`${Math.round(temp)} °C`} ratio={undefined} /> : null}
    </Section>
  );
}

const mb = (n: number) => `${decimal(n / 1048576)} MB`;

function Transfers({ act, transfers, canPull }: Pick<OverviewProps, "act" | "transfers"> & { canPull: boolean }) {
  return (
    <Section
      title="Transferências recentes" grouped
      actions={canPull ? <Button variant="ghost" onClick={() => void act("Receber arquivos", () => invokeCommand("receive_remote_files"))}>Receber arquivos</Button> : null}
    >
      {transfers.length === 0 ? <p className={styles.empty}>Nenhuma transferência nesta sessão.</p> : null}
      {transfers.map((t) => (
        <div key={t.transfer_id} className={styles.transfer}>
          <div className={styles.meterHead}>
            <span className={styles.name}>{t.file_name}</span>
            <span className={t.error ? styles.err : undefined}>{t.error ? "falhou" : t.done ? "concluída" : `${mb(t.bytes)} de ${mb(t.total)}`}</span>
          </div>
          {t.error ? <span className={styles.err}>{t.error}</span> : <ProgressBar label={`Progresso de ${t.file_name}`} value={t.bytes} max={t.total} />}
        </div>
      ))}
    </Section>
  );
}

function bytesToBase64(bytes: ReadonlyArray<number>): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}

function Utilities({ act, online, canCapture }: { act: Act; online: boolean; canCapture: boolean }) {
  const [displays, setDisplays] = useState<DisplayInfo[]>([]);
  const [display, setDisplay] = useState("");
  const [shot, setShot] = useState<string>();
  const [target, setTarget] = useState("");
  const [text, setText] = useState("");

  useEffect(() => {
    void invokeCommand("get_displays")
      .then((d) => { setDisplays(d.peer); setDisplay(d.peer[0]?.id ?? ""); })
      .catch(() => undefined);
  }, []);

  const capture = async () => {
    const r = await act("Captura de tela", () => send({ CaptureScreen: { display_id: display } }));
    const out = r?.ok ? r.output : null;
    if (out && "Png" in out) setShot(`data:image/png;base64,${bytesToBase64(out.Png)}`);
  };
  const submit = (run: () => void) => (e: FormEvent) => { e.preventDefault(); run(); };

  return (
    <Section title="Ferramentas do outro computador" grouped>
      {canCapture ? (
        <>
      <div className={styles.toolRow}>
        <Select
          label="Capturar tela" value={display} disabled={!online || displays.length === 0}
          options={displays.map((d) => ({ value: d.id, label: d.name }))} onChange={setDisplay}
        />
        <Button disabled={!online || !display} onClick={() => void capture()}>Capturar</Button>
      </div>
      {shot ? <img className={styles.shot} src={shot} alt="Captura da tela do outro computador" /> : null}
        </>
      ) : null}
      <form className={styles.toolRow} onSubmit={submit(() => { void act("Abrir", () => send({ OpenTarget: { target } })); })}>
        <input className={styles.input} aria-label="URL ou app" placeholder="Abrir URL ou app no outro computador" value={target} onChange={(e) => setTarget(e.target.value)} />
        <Button type="submit" disabled={!online || !target.trim()}>Abrir</Button>
      </form>
      <form className={styles.toolRow} onSubmit={submit(() => { void act("Texto enviado", () => send({ TypeText: { text } })); })}>
        <input className={styles.input} aria-label="Texto para digitar no outro computador" placeholder="Digitar texto no outro computador" value={text} onChange={(e) => setText(e.target.value)} />
        <Button type="submit" disabled={!online || !text}>Enviar</Button>
      </form>
    </Section>
  );
}
