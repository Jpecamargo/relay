import type { PlacedDisplay } from "@relay/protocol-types";
import { expect, test } from "vitest";
import { alignDisplays, defaultLayout, dragDisplays, jumpIds, reconcile, sharedSegments, snapRect } from "./geometry";

const placed = (id: string, machine: PlacedDisplay["machine"], x: number, y: number, w: number, h: number): PlacedDisplay => ({
  display_id: id,
  machine,
  rect_mm: { x, y, w, h },
  sensitivity: 1,
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
