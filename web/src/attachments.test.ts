import { afterEach, describe, expect, it } from "vitest";

import { forgetFiles, keptFiles, pictureUrl } from "./components/Attachments";
import { KEPT_FILES } from "./constants";

const load = (file: string) => Promise.resolve(new Blob([file]));

describe("loaded pictures and clips", () => {
  afterEach(() => forgetFiles());

  it("keeps only the latest few", async () => {
    for (let i = 0; i < KEPT_FILES + 10; i++) await pictureUrl(load, `f${i}`);
    expect(keptFiles()).toBe(KEPT_FILES);
  });

  it("lets go of the one used longest ago, not one just looked at again", async () => {
    let fetches = 0;
    const counting = (file: string) => {
      fetches += 1;
      return load(file);
    };
    for (let i = 0; i < KEPT_FILES; i++) await pictureUrl(counting, `f${i}`);
    await pictureUrl(counting, "f0"); // looked at again
    await pictureUrl(counting, "new"); // pushes one out: f1, not f0
    fetches = 0;
    await pictureUrl(counting, "f0");
    expect(fetches).toBe(0);
    await pictureUrl(counting, "f1");
    expect(fetches).toBe(1);
  });

  it("lets go of everything when a different session opens", async () => {
    await pictureUrl(load, "a");
    await pictureUrl(load, "b");
    forgetFiles();
    expect(keptFiles()).toBe(0);
  });
});
