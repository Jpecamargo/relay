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

/** Volume do PC com mudo, e os controles de mídia como ícones pequenos. */
export function VolumeBlock({ act, online }: { act: Act; online: boolean }) {
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

  const icon: IconName = muted || volume === 0 ? "volumeMute" : volume < 0.5 ? "volumeLow" : "volume";
  return (
    <div className={styles.volume}>
      <div className={styles.volumeRow}>
        <button
          type="button" className={styles.iconBtn} disabled={!online}
          aria-label="Mudo no Windows" aria-pressed={muted}
          onClick={() => { const m = !muted; setMuted(m); void act("Mudo", () => send({ SetMute: { muted: m } }), true); }}
        >
          <Icon name={icon} size={16} />
        </button>
        <Slider
          compact label="Volume do Windows" value={volume} min={0} max={1} step={0.01} debounceMs={50}
          format={(v) => `${Math.round(v * 100)}%`} disabled={!online}
          onChange={(v) => { setVolume(v); void act("Volume", () => send({ SetVolume: { level: v } }), true); }}
        />
      </div>
      <div className={styles.media}>
        {MEDIA.map((m) => (
          <button
            key={m.action} type="button" className={styles.iconBtn} disabled={!online} aria-label={m.label}
            onClick={() => void act(m.label, () => send({ Media: { action: m.action } }), true)}
          >
            <Icon name={m.icon} size={16} />
          </button>
        ))}
      </div>
    </div>
  );
}
