import { describe, expect, it } from "vitest";

import { score } from "./components/CommandPalette";

describe("palette score", () => {
  it("needs every word, in any order", () => {
    expect(score("Favourite drink thrall-sandbox", "drink fav")).toBeGreaterThan(0);
    expect(score("Favourite drink", "drink coffee")).toBe(0);
  });

  it("ranks matches at the start of a word above matches inside one", () => {
    expect(score("/cost Tokens and money", "cost")).toBeGreaterThan(score("Forecasts", "cast"));
  });

  it("matches everything when nothing is typed", () => {
    expect(score("anything", "  ")).toBe(1);
  });
});
