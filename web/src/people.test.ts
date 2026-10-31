import { describe, expect, it } from "vitest";

import { dollars, monthStart, signInLink } from "./people";

describe("people helpers", () => {
  it("counts the month from local midnight on the first", () => {
    const start = new Date(monthStart(new Date(2026, 9, 27, 15, 30)));
    expect([start.getFullYear(), start.getMonth(), start.getDate(), start.getHours(), start.getMinutes()]).toEqual([2026, 9, 1, 0, 0]);
  });

  it("puts the code after this page's address", () => {
    expect(signInLink({ origin: "http://127.0.0.1:7417", pathname: "/" }, "ab12")).toBe("http://127.0.0.1:7417/#signin=ab12");
  });

  it("shows money to the cent", () => {
    expect(dollars(0)).toBe("$0.00");
    expect(dollars(0.004)).toBe("under $0.01");
    expect(dollars(3.456)).toBe("$3.46");
  });
});
