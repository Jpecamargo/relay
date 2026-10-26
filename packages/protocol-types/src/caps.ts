// Bits de capacidade: mesmos valores de `Caps::*` em crates/core/src/types/caps.rs.
import type { Caps } from "./generated";

export const CAPS = {
  COMMANDS: 1 << 0,
  VOLUME: 1 << 1,
  SCREEN_CAPTURE: 1 << 2,
  METRICS: 1 << 3,
  MIC_SINK: 1 << 4,
  MIC_SOURCE: 1 << 5,
  FOREGROUND_APP: 1 << 6,
  WOL_TARGET: 1 << 7,
  DEFERRED_FILE_LIST: 1 << 8,
  FILE_PULL: 1 << 9,
} as const;

/** `true` se `caps` existe e tem todos os bits de `bit` ligados. */
export function hasCap(caps: Caps | null | undefined, bit: number): boolean {
  return caps != null && (caps.features & bit) === bit;
}
