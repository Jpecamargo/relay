import type { PlacedDisplay } from "@relay/protocol-types";
import { expect, test } from "vitest";
import { jumpIds } from "../shared/jump";
import type { DisplayInfo } from "@relay/protocol-types";
import { alignDisplays, defaultLayout, dragDisplays, dragWithGuides, orientationOf, reconcile, rotateDisplay, sharedSegments, sizeMm, snapDelta, snapRect } from "./geometry";

const placed = (id: string, machine: PlacedDisplay["machine"], x: number, y: number, w: number, h: number): PlacedDisplay => ({
  display_id: id,
  machine,
  rect_mm: { x, y, w, h },
  sensitivity: 1,
  rotation: null,
});
const mac = placed("mac", "Mac", 0, 0, 302, 196);
const d1 = (x: number, y: number) => placed("d1", "Windows", x, y, 597, 336);
const d2 = (x: number, y: number) => placed("d2", "Windows", x, y, 597, 336);

test("snap cola a até 5 mm da borda da outra máquina e ignora acima disso", () => {
  const near = snapRect({ x: 305, y: 3, w: 597, h: 336 }, [mac.rect_mm]);
  expect(near).toMatchObject({ x: 302, y: 0 });
  const far = snapRect({ x: 308, y: 6, w: 597, h: 336 }, [mac.rect_mm]);
  expect(far).toMatchObject({ x: 308, y: 6 });
});

test("arrastar o bloco preserva as posições relativas e cola o bloco no Mac", () => {
  const layout = [mac, d1(302, 0), d2(899, 0)];
  const out = dragDisplays(layout, new Set(["d1", "d2"]), 3, 2);
  const [a, b] = [out[1]!.rect_mm, out[2]!.rect_mm];
  expect(b.x - a.x).toBe(597);
  expect(b.y - a.y).toBe(0);
  expect(a).toMatchObject({ x: 302, y: 0 }); // colou
  expect(out[0]).toBe(mac);
});

test("arrastar solto move só a tela escolhida", () => {
  const layout = [mac, d1(302, 0), d2(899, 0)];
  const out = dragDisplays(layout, new Set(["d2"]), 100, 50);
  expect(out[1]!.rect_mm).toEqual(layout[1]!.rect_mm);
  expect(out[2]!.rect_mm).toMatchObject({ x: 999, y: 50 });
});

test("segmento compartilhado MacBook 302x196 + Dell 597x336 alinhado pelo topo = y em [0, 196]", () => {
  const links = sharedSegments([mac, d1(302, 0)]);
  expect(links).toEqual([{ from_display: "mac", to_display: "d1", side: "Right", segment_mm: [0, 196] }]);
});

test("sem contato ou só encostando no canto não há segmento", () => {
  expect(sharedSegments([mac, d1(310, 0)])).toEqual([]);
  expect(sharedSegments([mac, d1(302, 196)])).toEqual([]);
  expect(sharedSegments([mac, d1(302, 100)])[0]!.segment_mm).toEqual([100, 196]);
});

test("alinhar pelo topo e pelo centro move o bloco Windows inteiro", () => {
  const layout = [mac, d1(302, 40), d2(899, 40)];
  const ids = new Set(["d1", "d2"]);
  const top = alignDisplays(layout, ids, "top");
  expect(top.map((d) => d.rect_mm.y)).toEqual([0, 0, 0]);
  const center = alignDisplays(layout, ids, "center");
  expect(center[1]!.rect_mm.y).toBe(98 - 168);
  expect(center[2]!.rect_mm.x).toBe(899);
});

test("pulo: MacBook é 1; Windows da esquerda para a direita e de cima para baixo", () => {
  const ids = jumpIds([mac, d2(1000, 0), d1(302, 0)]);
  expect([ids.get("mac"), ids.get("d1"), ids.get("d2")]).toEqual([1, 2, 3]);
});

test("combinação nova de telas gera o perfil padrão: Windows à direita do Mac, pelo topo", () => {
  const info = (id: string, machine: "Mac" | "Windows", mm: [number, number]) => ({
    id, name: id, machine, origin_px: [0, 0] as [number, number], size_px: [1000, 1000] as [number, number],
    scale: 1, size_mm: mm, is_primary: true, is_builtin: false,
  });
  const connected = [info("mac", "Mac", [302, 196]), info("d1", "Windows", [597, 336])];
  const out = reconcile([mac], connected);
  expect(out.map((d) => d.rect_mm)).toEqual([
    { x: 0, y: 0, w: 302, h: 196 },
    { x: 302, y: 0, w: 597, h: 336 },
  ]);
  expect(reconcile([mac, d1(302, 0)], connected)).toHaveLength(2);
  expect(defaultLayout(connected)[1]!.sensitivity).toBe(1);
});

test("ímã alinha topo, centro e base e cola borda com borda de qualquer tela", () => {
  const t = { x: 300, y: 100, w: 200, h: 100 };
  // topo a 8 mm do topo do alvo, lado direito colado ao esquerdo do alvo a 6 mm
  const a = snapDelta({ x: 94, y: 108, w: 200, h: 100 }, [t], 12);
  expect([a.dx, a.dy]).toEqual([6, -8]);
  // centro vertical: alvo cy=150, movendo cy=157
  const c = snapDelta({ x: 0, y: 127, w: 100, h: 60 }, [t], 12);
  expect(c.dy).toBe(-7);
  // base alinhada: alvo bottom=200, movendo bottom=205 (y=125,h=80) ganha do topo (25 mm)
  expect(snapDelta({ x: 0, y: 125, w: 100, h: 80 }, [t], 12).dy).toBe(-5);
  // fora do alcance não gruda
  expect(snapDelta({ x: 0, y: 80, w: 100, h: 20 }, [t], 5).dy).toBe(0);
});

test("ímã devolve a guia do alinhamento e threshold 0 (Alt) desliga", () => {
  const layout = [mac, d1(302, 0), d2(1000, 4)];
  const on = dragWithGuides(layout, new Set(["d2"]), -100, -2, 12);
  // d2 gruda no d1 (x = 899) e alinha o topo com ele (y = 0)
  expect(on.displays[2]!.rect_mm).toMatchObject({ x: 899, y: 0 });
  expect(on.guides.some((g) => g.axis === "x" && g.at === 899)).toBe(true);
  expect(on.guides.some((g) => g.axis === "y" && g.at === 0)).toBe(true);
  const off = dragWithGuides(layout, new Set(["d2"]), -100, -2, 0);
  expect(off.displays[2]!.rect_mm).toMatchObject({ x: 900, y: 2 });
  expect(off.guides).toEqual([]);
});

const dell = (px: [number, number]): DisplayInfo => ({
  id: "d1", name: "Dell", machine: "Windows", origin_px: [0, 0], size_px: px, scale: 1,
  size_mm: [597, 336], is_primary: false, is_builtin: false,
});

test("tamanho em mm segue o aspecto dos px (EDID sempre paisagem) e o override manda", () => {
  expect(sizeMm(dell([2560, 1440]))).toEqual([597, 336]);
  expect(sizeMm(dell([1440, 2560]))).toEqual([336, 597]);
  expect(sizeMm(dell([2560, 1440]), 90)).toEqual([336, 597]);
  expect(sizeMm(dell([1440, 2560]), 0)).toEqual([597, 336]);
});

test("girar a tela troca w/h, empurra o vizinho que ficaria sobreposto e grava o override", () => {
  const info = new Map([["d1", dell([2560, 1440])]]);
  const portrait = [mac, placed("d1", "Windows", 302, 0, 336, 597), d2(638, 0)];
  const wide = rotateDisplay(portrait, info, "d1", 0);
  expect(wide[1]!.rect_mm).toMatchObject({ x: 302, w: 597, h: 336 });
  expect(wide[1]!.rotation).toBe(0);
  expect(wide[2]!.rect_mm.x).toBe(899); // vizinho segue a largura nova (+261)
  const tall = rotateDisplay([mac, d1(302, 0), d2(899, 0)], info, "d1", 90);
  expect(tall[1]!.rect_mm).toMatchObject({ w: 336, h: 597 });
  expect(orientationOf(tall[1]!)).toBe(90);
  expect(rotateDisplay([mac], info, "mac", 90)).toEqual([mac]); // Mac não gira
});

test("perfil salvo acompanha a tela girada no Windows, mas não mexe no override", () => {
  const connected = [
    { ...dell([1440, 2560]) },
    { id: "mac", name: "mac", machine: "Mac" as const, origin_px: [0, 0] as [number, number], size_px: [1440, 900] as [number, number], scale: 2, size_mm: [302, 196] as [number, number], is_primary: true, is_builtin: true },
  ];
  const out = reconcile([mac, d1(302, 0)].map((d) => (d.display_id === "d1" ? { ...d, display_id: "d1" } : d)), connected);
  expect(out[1]!.rect_mm).toMatchObject({ w: 336, h: 597 });
  const manual = reconcile([mac, { ...d1(302, 0), rotation: 0 }], connected);
  expect(manual[1]!.rect_mm).toMatchObject({ w: 597, h: 336 });
});
