import { describe, expect, it } from "vitest";

import type { CommandItem } from "./protocol";
import { terminalOnly } from "./slash";

const known: CommandItem[] = [{ name: "model", summary: "Switch model", args_hint: "[model id]" }];

describe("terminal-only commands", () => {
  it("say where this app has them instead of going to the engine", () => {
    expect(terminalOnly("settings", known)).toContain("Settings at the bottom of the sidebar");
    expect(terminalOnly("model", known)).toBeNull();
    expect(terminalOnly("my-skill", known)).toBeNull();
    expect(terminalOnly("constructor", known)).toBeNull();
  });

  it("leave a skill of the person's own with the same name alone", () => {
    const withSkill = [...known, { name: "settings", summary: "Mine", args_hint: "", skill: true }];
    expect(terminalOnly("settings", withSkill)).toBeNull();
  });
});
