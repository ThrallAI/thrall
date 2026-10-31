import { describe, expect, it } from "vitest";

import { isNewer } from "./updates";

describe("isNewer", () => {
  it("compares versions number by number", () => {
    expect(isNewer("v0.2.0", "0.1.0")).toBe(true);
    expect(isNewer("0.1.10", "0.1.9")).toBe(true);
    expect(isNewer("v1.0", "0.9.9")).toBe(true);
    expect(isNewer("0.1.0", "0.1.0")).toBe(false);
    expect(isNewer("0.0.9", "0.1.0")).toBe(false);
  });
  it("ignores a pre-release tag", () => {
    expect(isNewer("v0.1.0-preview", "0.1.0")).toBe(false);
    expect(isNewer("v0.2.0-preview", "0.1.0")).toBe(true);
  });
});
