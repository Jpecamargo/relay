import type { PlacedDisplay } from "@relay/protocol-types";

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
