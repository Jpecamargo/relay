import type { DisplayInfo, PlacedDisplay } from "@relay/protocol-types";
import { useMemo, useRef, useState, type PointerEvent } from "react";
import styles from "./editor.module.css";
import { bottom, bounds, dragDisplays, right, sharedSegments } from "./geometry";

const MARGIN_MM = 100;

interface Drag {
  ids: ReadonlySet<string>;
  startX: number;
  startY: number;
  mmPerPx: number;
}

export interface CanvasProps {
  displays: PlacedDisplay[];
  info: ReadonlyMap<string, DisplayInfo>;
  jump: ReadonlyMap<string, number>;
  detached: boolean;
  selected: string | undefined;
  onSelect: (id: string) => void;
  onChange: (displays: PlacedDisplay[]) => void;
}

/** Canvas SVG em mm. As telas Mac são fixas; as Windows arrastam em bloco ou soltas. */
export function Canvas({ displays, info, jump, detached, selected, onSelect, onChange }: CanvasProps) {
  const svg = useRef<SVGSVGElement>(null);
  const [drag, setDrag] = useState<Drag>();
  const [preview, setPreview] = useState<PlacedDisplay[]>();
  const shown = preview ?? displays;

  // O enquadramento só muda com o layout confirmado: durante o arraste a escala não se mexe.
  const vb = useMemo(() => {
    const b = bounds(displays.map((d) => d.rect_mm));
    return { x: b.x - MARGIN_MM, y: b.y - MARGIN_MM, w: b.w + 2 * MARGIN_MM, h: b.h + 2 * MARGIN_MM };
  }, [displays]);
  const unit = vb.w / 720;

  const links = useMemo(() => sharedSegments(shown), [shown]);
  const macs = new Map(shown.filter((d) => d.machine === "Mac").map((d) => [d.display_id, d]));

  const mmPerPx = () => {
    const r = svg.current?.getBoundingClientRect();
    if (!r || r.width === 0 || r.height === 0) return 1;
    return 1 / Math.min(r.width / vb.w, r.height / vb.h);
  };

  const onDown = (e: PointerEvent<SVGGElement>, d: PlacedDisplay) => {
    onSelect(d.display_id);
    if (d.machine === "Mac") return;
    const ids = new Set(detached ? [d.display_id] : displays.filter((x) => x.machine === "Windows").map((x) => x.display_id));
    e.currentTarget.setPointerCapture?.(e.pointerId);
    setDrag({ ids, startX: e.clientX, startY: e.clientY, mmPerPx: mmPerPx() });
  };
  const onMove = (e: PointerEvent<SVGGElement>) => {
    if (!drag) return;
    setPreview(
      dragDisplays(displays, drag.ids, (e.clientX - drag.startX) * drag.mmPerPx, (e.clientY - drag.startY) * drag.mmPerPx),
    );
  };
  const onUp = () => {
    if (preview) onChange(preview);
    setDrag(undefined);
    setPreview(undefined);
  };

  return (
    <svg
      ref={svg}
      className={styles.canvas}
      viewBox={`${vb.x} ${vb.y} ${vb.w} ${vb.h}`}
      role="group"
      aria-label="Disposição das telas"
    >
      {shown.map((d) => {
        const r = d.rect_mm;
        const fixed = d.machine === "Mac";
        const meta = info.get(d.display_id);
        return (
          <g
            key={d.display_id}
            data-testid={`display-${d.display_id}`}
            data-machine={d.machine}
            className={[styles.display, fixed ? styles.fixed : styles.movable, selected === d.display_id ? styles.selected : ""].join(" ")}
            onPointerDown={(e) => onDown(e, d)}
            onPointerMove={onMove}
            onPointerUp={onUp}
            onPointerCancel={onUp}
          >
            <rect x={r.x} y={r.y} width={r.w} height={r.h} rx={3 * unit} strokeWidth={1.5 * unit} />
            <text x={r.x + r.w / 2} y={r.y + r.h / 2 - 4 * unit} textAnchor="middle" fontSize={13 * unit} className={styles.name}>
              {meta?.name ?? d.display_id}
            </text>
            <text x={r.x + r.w / 2} y={r.y + r.h / 2 + 14 * unit} textAnchor="middle" fontSize={11 * unit} className={styles.dim}>
              {Math.round(r.w)} × {Math.round(r.h)} mm{fixed ? " · fixa" : ""}
            </text>
            <circle cx={r.x + 16 * unit} cy={r.y + 16 * unit} r={10 * unit} className={styles.badge} />
            <text x={r.x + 16 * unit} y={r.y + 16 * unit + 4 * unit} textAnchor="middle" fontSize={11 * unit} className={styles.badgeText}>
              {jump.get(d.display_id)}
            </text>
          </g>
        );
      })}
      {links.map((l) => {
        const m = macs.get(l.from_display)?.rect_mm;
        if (!m) return null;
        const [s0, s1] = l.segment_mm;
        const horizontal = l.side === "Top" || l.side === "Bottom";
        const fixedAxis = l.side === "Right" ? right(m) : l.side === "Left" ? m.x : l.side === "Bottom" ? bottom(m) : m.y;
        const pts = horizontal
          ? { x1: s0, x2: s1, y1: fixedAxis, y2: fixedAxis }
          : { x1: fixedAxis, x2: fixedAxis, y1: s0, y2: s1 };
        return (
          <line
            key={`${l.from_display}-${l.to_display}-${l.side}`}
            data-testid="shared-edge"
            data-segment={`${s0.toFixed(1)},${s1.toFixed(1)}`}
            className={drag ? styles.edgeLive : styles.edge}
            strokeWidth={4 * unit}
            {...pts}
          />
        );
      })}
    </svg>
  );
}
