import { describe, expect, it } from "vitest";

import { arrange, groupByFolder } from "./components/Sidebar";
import type { Session } from "./types";

function s(session: string, workspace: string | null, extra: Partial<Session> = {}): Session {
  return { session, title: session, workspace, mode: "ask", state: "idle", last_seq: 0, updated: 0, cost_usd: 0, ...extra };
}

describe("groupByFolder", () => {
  it("groups by folder in the order sessions arrive (newest first)", () => {
    const groups = groupByFolder([
      s("a", "C:\\Code\\thrall-sandbox"),
      s("b", "C:\\Code\\thrall-next"),
      s("c", "C:\\Code\\thrall-sandbox"),
      s("d", null),
    ]);
    expect(groups.map((g) => g.label)).toEqual(["thrall-sandbox", "thrall-next", "No folder"]);
    expect(groups[0]?.sessions.map((x) => x.session)).toEqual(["a", "c"]);
  });
});
describe("arrange", () => {
  it("shows pinned first, then own groups, then folders, each session once", () => {
    const groups = [{ group: "g1", name: "Clients" }, { group: "g2", name: "Empty" }];
    const out = arrange(
      [
        s("a", "C:\\x", { pinned: true, group: "g1" }),
        s("b", "C:\\x", { group: "g1" }),
        s("c", "C:\\x"),
        s("d", "C:\\y", { group: "gone" }),
      ],
      groups,
    );
    expect(out.pinned.map((x) => x.session)).toEqual(["a"]);
    expect(out.inGroup.get("g1")?.map((x) => x.session)).toEqual(["b"]);
    expect(out.inGroup.get("g2")).toEqual([]);
    // A session whose group no longer exists falls back to its folder.
    expect(out.folders.flatMap((f) => f.sessions.map((x) => x.session))).toEqual(["c", "d"]);
  });
});