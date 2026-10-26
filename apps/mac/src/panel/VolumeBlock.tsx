import type { MediaAction } from "@relay/protocol-types";
import { Icon, Slider, useRelayEvent, type IconName } from "@relay/ui";
import { useEffect, useRef, useState } from "react";
import { send } from "../shared/send";
import type { Act } from "../shared/useActions";
import styles from "./panel.module.css";

const MEDIA: ReadonlyArray<{ action: MediaAction; label: string; icon: IconName }> = [
  { action: "Previous", label: "Anterior", icon: "prev" },
  { action: "PlayPause", label: "Play/pause", icon: "playPause" },
  { action: "Next", label: "Próxima", icon: "next" },
];

/** Volume do outro computador com mudo, e os controles de mídia como ícones pequenos. */
export function VolumeBlock({ act, online, peer }: { act: Act; online: boolean; peer: string }) {
  const [volume, setVolume] = useState(0.5);
  const [muted, setMuted] = useState(false);
  // O PC é a fonte da verdade (a troca de saída muda volume e mudo sem passar por aqui): segue as
  // métricas, mas só quando o valor do PC muda, para um tick repetido não desfazer o arraste do slider.
  const metrics = useRelayEvent("relay:metrics");
  const last = useRef<{ volume?: number | null; muted?: boolean | null }>({});
  useEffect(() => {
    if (!metrics) return;
    if (metrics.volume != null && metrics.volume !== last.current.volume) setVolume(metrics.volume);
    if (metrics.muted != null && metrics.muted !== last.current.muted) setMuted(metrics.muted);
    last.current = { volume: metrics.volume, muted: metrics.muted };
  }, [metrics]);

  const icon: IconName = muted || volume === 0 ? "volumeMute" : volume < 0.5 ? "volumeLow" : "volume";
  return (
    <div className={styles.volume}>
      <div className={styles.volumeRow}>
        <button
          type="button" className={styles.iconBtn} disabled={!online}
          aria-label={`Mudo em ${peer}`} aria-pressed={muted}
          onClick={() => { const m = !muted; setMuted(m); void act("Mudo", () => send({ SetMute: { muted: m } })); }}
        >
          <Icon name={icon} size={16} />
        </button>
        <Slider
          compact label={`Volume de ${peer}`} value={volume} min={0} max={1} step={0.01} debounceMs={50}
          format={(v) => `${Math.round(v * 100)}%`} disabled={!online}
          onChange={(v) => { setVolume(v); void act("Volume", () => send({ SetVolume: { level: v } })); }}
        />
      </div>
      <div className={styles.media}>
        {MEDIA.map((m) => (
          <button
            key={m.action} type="button" className={styles.iconBtn} disabled={!online} aria-label={m.label}
            onClick={() => void act(m.label, () => send({ Media: { action: m.action } }))}
          >
            <Icon name={m.icon} size={16} />
          </button>
        ))}
      </div>
    </div>
  );
}
