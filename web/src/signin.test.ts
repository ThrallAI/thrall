import { describe, expect, it } from "vitest";

import { deviceName } from "./signin";

describe("deviceName", () => {
  it("names the browser and the system", () => {
    const edge =
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 Edg/130.0";
    expect(deviceName(edge)).toBe("Edge on Windows");
    const phone =
      "Mozilla/5.0 (Linux; Android 16) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Mobile Safari/537.36";
    expect(deviceName(phone)).toBe("Chrome on Android");
    expect(deviceName("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) Version/17 Safari/605")).toBe(
      "Safari on macOS",
    );
    expect(deviceName("curl/8")).toBe("A browser");
  });
});
