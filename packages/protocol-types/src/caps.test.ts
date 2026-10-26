import { describe, expect, test } from "vitest";
import { CAPS, hasCap } from "./caps";
import type { Caps } from "./generated";

const windows: Caps = {
  os: "Windows",
  features: CAPS.COMMANDS | CAPS.VOLUME | CAPS.SCREEN_CAPTURE | CAPS.METRICS | CAPS.MIC_SINK
    | CAPS.FOREGROUND_APP | CAPS.WOL_TARGET | CAPS.DEFERRED_FILE_LIST,
};

describe("caps", () => {
  test("os bits têm os valores do Rust", () => {
    expect(CAPS).toEqual({
      COMMANDS: 1, VOLUME: 2, SCREEN_CAPTURE: 4, METRICS: 8, MIC_SINK: 16,
      MIC_SOURCE: 32, FOREGROUND_APP: 64, WOL_TARGET: 128, DEFERRED_FILE_LIST: 256, FILE_PULL: 512,
    });
  });

  test("hasCap lê o bit nas features", () => {
    expect(hasCap(windows, CAPS.WOL_TARGET)).toBe(true);
    expect(hasCap(windows, CAPS.MIC_SOURCE)).toBe(false);
    expect(hasCap(windows, CAPS.FILE_PULL)).toBe(false);
  });

  test("hasCap exige todos os bits pedidos", () => {
    expect(hasCap(windows, CAPS.COMMANDS | CAPS.VOLUME)).toBe(true);
    expect(hasCap(windows, CAPS.COMMANDS | CAPS.FILE_PULL)).toBe(false);
  });

  test("sem caps, nenhuma capacidade", () => {
    expect(hasCap(null, CAPS.COMMANDS)).toBe(false);
    expect(hasCap(undefined, CAPS.COMMANDS)).toBe(false);
  });
});
