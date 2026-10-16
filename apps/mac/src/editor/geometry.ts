// Geometria do editor de telas (D17): snap e arraste ao vivo em TS, sem IPC. Tudo em mm, no plano do layout.
// O core (C6) valida e calcula as bordas oficiais no save_layout; aqui é só a pré-visualização.
import type { DisplayInfo, EdgeLink, PlacedDisplay, ProfileKey, RectMm, Side } from "@relay/protocol-types";

export const SNAP_MM = 5;
/** Tolerância para considerar duas bordas coladas. */
export const TOUCH_EPS_MM = 0.5;
export const SENSITIVITY_MIN = 0.25;
export const SENSITIVITY_MAX = 4;

export const right = (r: RectMm) => r.x + r.w;
export const bottom = (r: RectMm) => r.y + r.h;

export function bounds(rects: RectMm[]): RectMm {
  if (rects.length === 0) return { x: 0, y: 0, w: 0, h: 0 };
  const x0 = Math.min(...rects.map((r) => r.x));
  const y0 = Math.min(...rects.map((r) => r.y));
  const x1 = Math.max(...rects.map(right));
  const y1 = Math.max(...rects.map(bottom));
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

export function translate(r: RectMm, dx: number, dy: number): RectMm {
  return { ...r, x: r.x + dx, y: r.y + dy };
}

/** Menor deslocamento (em módulo) entre os candidatos que cabe no limite, ou 0. */
function smallest(deltas: number[], limit: number): number | null {
  let best: number | null = null;
  for (const d of deltas) {
    if (Math.abs(d) <= limit && (best === null || Math.abs(d) < Math.abs(best))) best = d;
  }
  return best;
}

/**
 * Deslocamento extra que cola `moving` nas bordas das telas `targets` (da outra máquina)
 * quando alguma borda está a até `threshold` mm. Cada eixo é resolvido sozinho.
 * Cola de encontro (direita com esquerda) e alinha bordas do mesmo lado.
 */
export function snapDelta(moving: RectMm, targets: RectMm[], threshold = SNAP_MM): { dx: number; dy: number } {
  const xs: number[] = [];
  const ys: number[] = [];
  for (const t of targets) {
    xs.push(right(t) - moving.x, t.x - right(moving), t.x - moving.x, right(t) - right(moving));
    ys.push(bottom(t) - moving.y, t.y - bottom(moving), t.y - moving.y, bottom(t) - bottom(moving));
  }
  return { dx: smallest(xs, threshold) ?? 0, dy: smallest(ys, threshold) ?? 0 };
}

export function snapRect(moving: RectMm, targets: RectMm[], threshold = SNAP_MM): RectMm {
  const { dx, dy } = snapDelta(moving, targets, threshold);
  return translate(moving, dx, dy);
}

/**
 * Arrasta `ids` por (dx, dy). O conjunto inteiro se move junto, então as posições relativas
 * se preservam; o snap é calculado sobre o retângulo envolvente do conjunto contra as telas
 * da outra máquina (as que não estão em `ids` e têm máquina diferente da do conjunto).
 */
export function dragDisplays(
  displays: PlacedDisplay[],
  ids: ReadonlySet<string>,
  dx: number,
  dy: number,
  threshold = SNAP_MM,
): PlacedDisplay[] {
  const moved = displays.filter((d) => ids.has(d.display_id));
  if (moved.length === 0) return displays;
  const machine = moved[0]?.machine;
  const targets = displays.filter((d) => d.machine !== machine).map((d) => d.rect_mm);
  const box = translate(bounds(moved.map((d) => d.rect_mm)), dx, dy);
  const snap = snapDelta(box, targets, threshold);
  const [tx, ty] = [dx + snap.dx, dy + snap.dy];
  return displays.map((d) => (ids.has(d.display_id) ? { ...d, rect_mm: translate(d.rect_mm, tx, ty) } : d));
}

const overlap = (a0: number, a1: number, b0: number, b1: number): [number, number] | null => {
  const lo = Math.max(a0, b0);
  const hi = Math.min(a1, b1);
  return hi - lo > TOUCH_EPS_MM ? [lo, hi] : null;
};

/**
 * Trechos de borda compartilhados entre telas de máquinas diferentes. `side` é o lado da tela Mac
 * (`from_display`); `segment_mm` é o intervalo no eixo da borda (y para Left/Right, x para Top/Bottom).
 */
export function sharedSegments(displays: PlacedDisplay[]): EdgeLink[] {
  const links: EdgeLink[] = [];
  const macs = displays.filter((d) => d.machine === "Mac");
  const wins = displays.filter((d) => d.machine === "Windows");
  for (const m of macs) {
    for (const w of wins) {
      const a = m.rect_mm;
      const b = w.rect_mm;
      const push = (side: Side, seg: [number, number] | null) =>
        seg && links.push({ from_display: m.display_id, to_display: w.display_id, side, segment_mm: seg });
      if (Math.abs(right(a) - b.x) <= TOUCH_EPS_MM) push("Right", overlap(a.y, bottom(a), b.y, bottom(b)));
      if (Math.abs(a.x - right(b)) <= TOUCH_EPS_MM) push("Left", overlap(a.y, bottom(a), b.y, bottom(b)));
      if (Math.abs(bottom(a) - b.y) <= TOUCH_EPS_MM) push("Bottom", overlap(a.x, right(a), b.x, right(b)));
      if (Math.abs(a.y - bottom(b)) <= TOUCH_EPS_MM) push("Top", overlap(a.x, right(a), b.x, right(b)));
    }
  }
  return links;
}

export type Align = "top" | "center";

/** Alinha o conjunto `ids` (como bloco) ao topo ou ao centro vertical da tela Mac. */
export function alignDisplays(displays: PlacedDisplay[], ids: ReadonlySet<string>, align: Align): PlacedDisplay[] {
  const ref = displays.find((d) => d.machine === "Mac");
  const moved = displays.filter((d) => ids.has(d.display_id));
  if (!ref || moved.length === 0) return displays;
  const box = bounds(moved.map((d) => d.rect_mm));
  const dy = align === "top" ? ref.rect_mm.y - box.y : ref.rect_mm.y + ref.rect_mm.h / 2 - (box.y + box.h / 2);
  return displays.map((d) => (ids.has(d.display_id) ? { ...d, rect_mm: translate(d.rect_mm, 0, dy) } : d));
}

/** Pulo 1-based (D19): MacBook = 1; telas Windows da esquerda para a direita e de cima para baixo. */
export function jumpIds(displays: PlacedDisplay[]): Map<string, number> {
  const order = [
    ...displays.filter((d) => d.machine === "Mac"),
    ...displays
      .filter((d) => d.machine === "Windows")
      .sort((a, b) => a.rect_mm.x - b.rect_mm.x || a.rect_mm.y - b.rect_mm.y),
  ];
  return new Map(order.map((d, i) => [d.display_id, i + 1]));
}

export const clampSensitivity = (v: number) => Math.min(SENSITIVITY_MAX, Math.max(SENSITIVITY_MIN, v));

/** Tamanho em mm: o físico se conhecido, senão por DPI (Windows: 96*scale; Mac, em pontos: ~127 pt/pol). */
export function sizeMm(d: DisplayInfo): [number, number] {
  if (d.size_mm) return d.size_mm;
  const ppi = d.machine === "Windows" ? 96 * d.scale : 127;
  return [(d.size_px[0] / ppi) * 25.4, (d.size_px[1] / ppi) * 25.4];
}

export const profileId = (key: ProfileKey) => key.join("|");

const sameSet = (a: string[], b: string[]) => a.length === b.length && [...a].sort().join("|") === [...b].sort().join("|");

/** Perfil padrão (D28): bloco Windows à direita do MacBook, alinhado pelo topo, da esquerda para a direita. */
export function defaultLayout(displays: DisplayInfo[]): PlacedDisplay[] {
  const place = (d: DisplayInfo, x: number, y: number): PlacedDisplay => {
    const [w, h] = sizeMm(d);
    return { display_id: d.id, machine: d.machine, rect_mm: { x, y, w, h }, sensitivity: 1 };
  };
  const macs = displays.filter((d) => d.machine === "Mac");
  const wins = displays.filter((d) => d.machine === "Windows");
  const out: PlacedDisplay[] = [];
  let x = 0;
  for (const m of macs) {
    const p = place(m, x, 0);
    out.push(p);
    x = right(p.rect_mm);
  }
  for (const w of wins) {
    const p = place(w, x, 0);
    out.push(p);
    x = right(p.rect_mm);
  }
  return out;
}

/** Garante que as telas do perfil são exatamente as conectadas; senão cai no padrão. */
export function reconcile(displays: PlacedDisplay[], connected: DisplayInfo[]): PlacedDisplay[] {
  return sameSet(
    displays.map((d) => d.display_id),
    connected.map((d) => d.id),
  )
    ? displays
    : defaultLayout(connected);
}

