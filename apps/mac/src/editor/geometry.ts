// Geometria do editor de telas (D17): snap e arraste ao vivo em TS, sem IPC. Tudo em mm, no plano do layout.
// O core (C6) valida e calcula as bordas oficiais no save_layout; aqui é só a pré-visualização.
import type { DisplayInfo, EdgeLink, PlacedDisplay, RectMm, Side } from "@relay/protocol-types";

export const SNAP_MM = 5;
/** Alcance do ímã em pixels de tela (independe do zoom do canvas). */
export const MAGNET_PX = 14;
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

/** Linha de guia de alinhamento (mm): vertical em `at` = x (de `from` a `to` em y) ou horizontal. */
export interface Guide {
  axis: "x" | "y";
  at: number;
  from: number;
  to: number;
}

interface Candidate {
  delta: number;
  /** Coordenada (no eixo do deslocamento) da linha onde as bordas se encontram. */
  line: number;
  target: RectMm;
}

/** Candidatos de um eixo: colar de encontro, alinhar início, fim e centro. */
function axisCandidates(m0: number, m1: number, t0: number, t1: number, target: RectMm): Candidate[] {
  const mc = (m0 + m1) / 2;
  const tc = (t0 + t1) / 2;
  return [
    { delta: t1 - m0, line: t1, target },
    { delta: t0 - m1, line: t0, target },
    { delta: t0 - m0, line: t0, target },
    { delta: t1 - m1, line: t1, target },
    { delta: tc - mc, line: tc, target },
  ];
}

interface Option {
  delta: number;
  ties: Candidate[];
}

/** Deslocamentos possíveis de um eixo, do menor para o maior, com os candidatos que empatam; por último, não colar. */
function options(cands: Candidate[], limit: number): Option[] {
  const within = cands.filter((c) => Math.abs(c.delta) <= limit).sort((a, b) => Math.abs(a.delta) - Math.abs(b.delta));
  const out: Option[] = [];
  for (const c of within) {
    if (!out.some((o) => Math.abs(o.delta - c.delta) < 0.01)) {
      out.push({ delta: c.delta, ties: within.filter((t) => Math.abs(t.delta - c.delta) < 0.01) });
    }
  }
  out.push({ delta: 0, ties: [] });
  return out;
}

/**
 * Ímã: deslocamento extra que cola `moving` nas telas `targets` quando alguma borda ou centro está
 * a até `threshold` mm. Cada eixo é resolvido sozinho: encosta borda com borda (direita com esquerda,
 * embaixo com em cima) e alinha topo/centro/base e bordas laterais. Entre as combinações dos dois
 * eixos fica a mais próxima que `fits` aceita: colar um eixo não pode jogar a tela em cima de outra.
 * Devolve também as guias do alinhamento atingido, já na posição colada.
 */
export function snapDelta(
  moving: RectMm,
  targets: RectMm[],
  threshold = SNAP_MM,
  fits: (dx: number, dy: number) => boolean = () => true,
): { dx: number; dy: number; guides: Guide[] } {
  const xs: Candidate[] = [];
  const ys: Candidate[] = [];
  for (const t of targets) {
    xs.push(...axisCandidates(moving.x, right(moving), t.x, right(t), t));
    ys.push(...axisCandidates(moving.y, bottom(moving), t.y, bottom(t), t));
  }
  const ox = options(xs, threshold);
  const oy = options(ys, threshold);
  const pairs = ox.flatMap((px, i) => oy.map((py, j) => ({ px, py, rank: i + j })));
  pairs.sort((a, b) => a.rank - b.rank);
  const { px, py } = pairs.find((p) => fits(p.px.delta, p.py.delta)) ?? pairs[0]!;
  const snapped = translate(moving, px.delta, py.delta);
  const guides: Guide[] = [];
  for (const c of px.ties) {
    guides.push({ axis: "x", at: c.line, from: Math.min(c.target.y, snapped.y), to: Math.max(bottom(c.target), bottom(snapped)) });
  }
  for (const c of py.ties) {
    guides.push({ axis: "y", at: c.line, from: Math.min(c.target.x, snapped.x), to: Math.max(right(c.target), right(snapped)) });
  }
  return { dx: px.delta, dy: py.delta, guides };
}

export function snapRect(moving: RectMm, targets: RectMm[], threshold = SNAP_MM): RectMm {
  const { dx, dy } = snapDelta(moving, targets, threshold);
  return translate(moving, dx, dy);
}

/**
 * Arrasta `ids` por (dx, dy). O conjunto inteiro se move junto, então as posições relativas
 * se preservam; o ímã é calculado sobre o retângulo envolvente do conjunto contra todas as outras
 * telas (qualquer máquina). `threshold` em mm; 0 desliga o ímã (Option/Alt). O resultado nunca
 * sobrepõe outra tela: solto em cima de uma, o conjunto sai pelo lado mais próximo.
 */
export function dragWithGuides(
  displays: PlacedDisplay[],
  ids: ReadonlySet<string>,
  dx: number,
  dy: number,
  threshold = SNAP_MM,
): { displays: PlacedDisplay[]; guides: Guide[] } {
  const moved = displays.filter((d) => ids.has(d.display_id));
  if (moved.length === 0) return { displays, guides: [] };
  const targets = displays.filter((d) => !ids.has(d.display_id)).map((d) => d.rect_mm);
  const rects = moved.map((d) => translate(d.rect_mm, dx, dy));
  const fits = (sx: number, sy: number) => !collides(rects.map((r) => translate(r, sx, sy)), targets);
  const box = bounds(rects);
  let snap = threshold > 0 ? snapDelta(box, targets, threshold, fits) : { dx: 0, dy: 0, guides: [] as Guide[] };
  if (!fits(snap.dx, snap.dy)) {
    const out = pushOut(rects, targets);
    snap = out ? { dx: out[0], dy: out[1], guides: [] } : { dx: -dx, dy: -dy, guides: [] };
  }
  const [tx, ty] = [dx + snap.dx, dy + snap.dy];
  return {
    displays: displays.map((d) => (ids.has(d.display_id) ? { ...d, rect_mm: translate(d.rect_mm, tx, ty) } : d)),
    guides: snap.guides,
  };
}

function collides(rects: RectMm[], targets: RectMm[]): boolean {
  return rects.some((a) => targets.some((b) => overlap(a.x, right(a), b.x, right(b)) && overlap(a.y, bottom(a), b.y, bottom(b))));
}

/** Menor deslocamento num eixo só que tira `rects` de cima de todas as `targets` (encostando numa delas). */
function pushOut(rects: RectMm[], targets: RectMm[]): [number, number] | null {
  const moves: [number, number][] = [];
  for (const m of rects) {
    for (const t of targets) moves.push([t.x - right(m), 0], [right(t) - m.x, 0], [0, t.y - bottom(m)], [0, bottom(t) - m.y]);
  }
  moves.sort((a, b) => Math.abs(a[0]) + Math.abs(a[1]) - (Math.abs(b[0]) + Math.abs(b[1])));
  return moves.find(([x, y]) => !collides(rects.map((r) => translate(r, x, y)), targets)) ?? null;
}

export function dragDisplays(
  displays: PlacedDisplay[],
  ids: ReadonlySet<string>,
  dx: number,
  dy: number,
  threshold = SNAP_MM,
): PlacedDisplay[] {
  return dragWithGuides(displays, ids, dx, dy, threshold).displays;
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

export const clampSensitivity = (v: number) => Math.min(SENSITIVITY_MAX, Math.max(SENSITIVITY_MIN, v));

/**
 * Tamanho em mm na orientação em uso. O EDID (`size_mm`) é sempre paisagem, mas `size_px` vem
 * girado quando o usuário gira a tela no Windows: se o aspecto dos px contraria o dos mm, troca os mm.
 * `rotation` (override salvo no perfil) manda quando presente: 90/270 = retrato. Espelha
 * `layout::oriented_size_mm` do core. Sem `size_mm`, estima por DPI (Windows: 96*scale; Mac: ~127 pt/pol).
 */
export function sizeMm(d: DisplayInfo, rotation: number | null = null): [number, number] {
  const [pw, ph] = d.size_px;
  const ppi = d.machine === "Windows" ? 96 * d.scale : 127;
  const [w, h] = d.size_mm && d.size_mm[0] > 0 && d.size_mm[1] > 0 ? d.size_mm : [(pw / ppi) * 25.4, (ph / ppi) * 25.4];
  const portrait = rotation === null ? ph > pw : rotation % 180 === 90;
  return portrait === h > w ? [w, h] : [h, w];
}

/** Orientação em graus de uma tela posta: o override, ou 90 se está em retrato, senão 0. */
export const orientationOf = (d: PlacedDisplay): number => d.rotation ?? (d.rect_mm.h > d.rect_mm.w ? 90 : 0);

/**
 * Gira a tela `id` (Windows) para `rotation` graus (`null` = automático, pelo aspecto dos px). A largura e a altura em mm trocam quando a
 * orientação muda, preservando o centro-superior esquerdo; vizinhos à direita na mesma faixa são
 * empurrados e, se ainda houver sobreposição, a tela vai para a direita do que ela cobre.
 */
export function rotateDisplay(displays: PlacedDisplay[], info: ReadonlyMap<string, DisplayInfo>, id: string, rotation: number | null): PlacedDisplay[] {
  const cur = displays.find((d) => d.display_id === id);
  const meta = info.get(id);
  if (!cur || !meta || cur.machine !== "Windows") return displays;
  const [w, h] = sizeMm(meta, rotation);
  const old = cur.rect_mm;
  const dw = w - old.w;
  let out = displays.map((d) => {
    if (d.display_id === id) return { ...d, rotation, rect_mm: { ...old, w, h } };
    const overlapsBand = d.rect_mm.y < bottom(old) - TOUCH_EPS_MM && bottom(d.rect_mm) > old.y + TOUCH_EPS_MM;
    return dw > 0 && d.machine === "Windows" && overlapsBand && d.rect_mm.x >= right(old) - TOUCH_EPS_MM
      ? { ...d, rect_mm: translate(d.rect_mm, dw, 0) }
      : d;
  });
  // Ainda sobrepõe alguém (ex.: Mac)? Vai para a direita da tela coberta.
  for (let i = 0; i < out.length; i++) {
    const me = out.find((d) => d.display_id === id)!.rect_mm;
    const hit = out.find((d) => d.display_id !== id && overlap(me.x, right(me), d.rect_mm.x, right(d.rect_mm)) && overlap(me.y, bottom(me), d.rect_mm.y, bottom(d.rect_mm)));
    if (!hit) break;
    out = out.map((d) => (d.display_id === id ? { ...d, rect_mm: { ...d.rect_mm, x: right(hit.rect_mm) } } : d));
  }
  return out;
}


const sameSet = (a: string[], b: string[]) => a.length === b.length && [...a].sort().join("|") === [...b].sort().join("|");

/** Perfil padrão (D28): bloco Windows à direita do MacBook, alinhado pelo topo, da esquerda para a direita. */
export function defaultLayout(displays: DisplayInfo[]): PlacedDisplay[] {
  const place = (d: DisplayInfo, x: number, y: number): PlacedDisplay => {
    const [w, h] = sizeMm(d);
    return { display_id: d.id, machine: d.machine, rect_mm: { x, y, w, h }, sensitivity: 1, rotation: null };
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

/** Perfil salvo com a tela girada depois (sem override): ajusta largura/altura à orientação atual dos px. */
function reorient(displays: PlacedDisplay[], connected: DisplayInfo[]): PlacedDisplay[] {
  return displays.map((p) => {
    const info = connected.find((d) => d.id === p.display_id);
    if (!info || p.rotation !== null && p.rotation !== undefined) return p;
    const [w, h] = sizeMm(info);
    return h > w === p.rect_mm.h > p.rect_mm.w ? p : { ...p, rect_mm: { ...p.rect_mm, w, h } };
  });
}

/** Garante que as telas do perfil são exatamente as conectadas; senão cai no padrão. */
export function reconcile(displays: PlacedDisplay[], connected: DisplayInfo[]): PlacedDisplay[] {
  return sameSet(
    displays.map((d) => d.display_id),
    connected.map((d) => d.id),
  )
    ? reorient(displays, connected)
    : defaultLayout(connected);
}
