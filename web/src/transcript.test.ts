import { describe, expect, it } from "vitest";

import { CLIENT_LABELS } from "./constants";
import type { EventFrame } from "./protocol";
import type { Session as SessionInfo } from "./types";
import { apply, emptyView, markForgotten, openedView, shownActivity, type View } from "./transcript";

let seq = 0;
function ev(body: Record<string, unknown>, stored = true): EventFrame {
  if (stored) seq += 1;
  return {
    v: 1,
    session: "s_1",
    at: 1000 + seq,
    ...(stored ? { seq } : {}),
    ...body,
  } as unknown as EventFrame;
}

function run(view: View, frames: EventFrame[], by = "web"): View {
  return frames.reduce((v, f) => apply(v, f, by, CLIENT_LABELS), view);
}

function info(overrides: Partial<SessionInfo> = {}): SessionInfo {
  return {
    session: "s_1",
    title: "Favourite drink",
    mode: "ask",
    state: "idle",
    last_seq: 0,
    updated: 0,
    cost_usd: 0.5,
    ...overrides,
  };
}

describe("modes", () => {
  const busy = ev({ type: "status", state: "working", model: "m" }, false);
  const idle = ev({ type: "status", state: "idle", model: "m" }, false);
  it("offers Ask after a task ends in Auto-accept edits, and not in Ask", () => {
    let v = run(emptyView(), [ev({ type: "session.updated", mode: "auto_edits" }), busy, idle]);
    expect(v.offerAsk).toBe(true);
    v = run(v, [ev({ type: "session.updated", mode: "ask" })]);
    expect(v.offerAsk).toBe(false);
    expect(run(emptyView(), [busy, idle]).offerAsk).toBe(false);
  });
  it("keeps how long Full auto lasts, and offers nothing when it ends with the task", () => {
    const limit = { until: "task_end" };
    let v = run(emptyView(), [ev({ type: "session.updated", mode: "full_auto", mode_limit: limit }), busy, idle]);
    expect(v.modeLimit).toEqual(limit);
    expect(v.offerAsk).toBe(false);
    v = run(emptyView(), [
      ev({ type: "session.updated", mode: "full_auto", mode_limit: { until: "manual" } }),
      busy,
      idle,
    ]);
    expect(v.offerAsk).toBe(true);
    v = run(v, [ev({ type: "session.updated", mode: "ask" })]);
    expect(v.modeLimit).toBeNull();
  });
  it("a title change leaves the mode alone", () => {
    const v = run(emptyView(), [
      ev({ type: "session.updated", mode: "full_auto", mode_limit: { until: "manual" } }),
      ev({ type: "session.updated", title: "New" }),
    ]);
    expect(v.mode).toBe("full_auto");
    expect(v.modeLimit).toEqual({ until: "manual" });
  });
  it("replaying an old Full auto switch leaves the chat's saved mode alone", () => {
    // Thrall restarted: the chat is saved as Ask, but its history still says Full auto.
    const v = run(openedView(info({ mode: "ask", last_seq: seq + 10 }), "m"), [
      ev({ type: "session.updated", mode: "full_auto", mode_limit: { until: "manual" } }),
    ]);
    expect(v.mode).toBe("ask");
    expect(v.modeLimit).toBeNull();
  });
});

describe("apply", () => {
  it("keeps the files attached to a user message", () => {
    seq = 0;
    const picture = { file: "f_1", name: "cat.png", kind: "image", size: 2048 };
    const v = run(emptyView(), [
      ev({ type: "turn.started", turn: "t1" }),
      ev({ type: "message.started", turn: "t1", message: "u1", role: "user" }),
      ev({ type: "message.completed", message: "u1", text: "what is this?", files: [picture] }),
      ev({ type: "message.started", turn: "t1", message: "u2", role: "user" }),
      ev({ type: "message.completed", message: "u2", text: "no files" }),
    ]);
    expect(v.items[0]).toMatchObject({ kind: "user", text: "what is this?", files: [picture] });
    expect(v.items[1]).toMatchObject({ kind: "user", files: [] });
  });
  it("builds a turn: user echo, streamed reply, end meter", () => {
    seq = 0;
    const v = run(emptyView(), [
      ev({ type: "turn.started", turn: "t1" }),
      ev({ type: "message.started", turn: "t1", message: "u1", role: "user" }),
      ev({ type: "message.completed", message: "u1", text: "hi" }),
      ev({ type: "message.started", turn: "t1", message: "m1", role: "assistant" }),
      ev({ type: "message.delta", message: "m1", text: "Hel" }, false),
      ev({ type: "message.delta", message: "m1", text: "lo" }, false),
    ]);
    expect(v.items.map((i) => i.kind)).toEqual(["user", "assistant"]);
    expect(v.items[1]).toMatchObject({ text: "Hello", streaming: true });
    expect(v.activity).toBe("writing");
    expect(v.busy).toBe(true);

    const done = run(v, [
      ev({ type: "message.completed", message: "m1", text: "Hello." }),
      ev({
        type: "turn.completed",
        turn: "t1",
        status: "done",
        usage: { input_tokens: 10, output_tokens: 4, cost_usd: 0.25 },
      }),
    ]);
    expect(done.items[1]).toMatchObject({ text: "Hello.", streaming: false });
    expect(done.items.at(-1)?.kind).toBe("turn_end");
    expect(done.cost).toBe(0.25);
    expect(done.activity).toBe("done");
    expect(done.busy).toBe(false);
  });

  it("keeps untouched items identical so they don't redraw", () => {
    seq = 0;
    const v = run(emptyView(), [
      ev({ type: "message.started", turn: "t1", message: "m0", role: "assistant" }),
      ev({ type: "message.completed", message: "m0", text: "earlier" }),
      ev({ type: "message.started", turn: "t1", message: "m1", role: "assistant" }),
    ]);
    const next = apply(v, ev({ type: "message.delta", message: "m1", text: "x" }, false), "web", CLIENT_LABELS);
    expect(next.items[0]).toBe(v.items[0]);
    expect(next.items[1]).not.toBe(v.items[1]);
  });

  it("tracks steps and their results", () => {
    seq = 0;
    const v = run(emptyView(), [
      ev({ type: "step.started", turn: "t1", step: "p1", tool: "shell", title: "Running cargo test" }),
      ev({ type: "step.output", step: "p1", text: "ok\n" }, false),
    ]);
    expect(v.activity).toBe("tool");
    const done = apply(
      v,
      ev({ type: "step.completed", step: "p1", status: "ok", summary: "Passed" }),
      "web",
      CLIENT_LABELS,
    );
    expect(done.items[0]).toMatchObject({ status: "ok", summary: "Passed", output: "ok\n", saved: [] });
  });

  it("keeps the files a step wrote, for its file cards", () => {
    seq = 0;
    const file = { path: "C:\\Users\\me\\Desktop\\cat.png", name: "cat.png", size: 2048 };
    const v = run(emptyView(), [
      ev({ type: "step.started", turn: "t1", step: "p1", tool: "make_image", title: "Making a picture" }),
      ev({ type: "step.completed", step: "p1", status: "ok", summary: "Made", saved: [file] }),
    ]);
    expect(v.items[0]).toMatchObject({ saved: [file] });
  });

  it("lists each helper's steps under its own row when helpers work side by side", () => {
    seq = 0;
    const step = (id: string, helper?: string) =>
      ev({ type: "step.started", turn: "t1", step: id, tool: "read_file", title: id, ...(helper ? { helper } : {}) });
    const v = run(emptyView(), [step("a"), step("b"), step("a1", "a"), step("b1", "b"), step("a2", "a"), step("c")]);
    expect(v.items.map((i) => i.id)).toEqual(["a", "a1", "a2", "b", "b1", "c"]);
    expect(v.items[1]).toMatchObject({ helper: "a" });
  });

  it("shows a permission card and closes it when answered elsewhere, saying where", () => {
    seq = 0;
    const v = run(emptyView(), [
      ev({
        type: "permission.request",
        request: "r1",
        turn: "t1",
        step: "p1",
        tool: "shell",
        summary: "Run `cargo test`",
        risk: "medium",
        choices: ["yes", "no", "always"],
      }),
    ]);
    expect(v.card).toMatchObject({ kind: "permission", request: "r1", always: true });
    expect(v.activity).toBe("waiting");
    const after = apply(
      v,
      ev({ type: "permission.resolved", request: "r1", choice: "yes", answered_by: "tui" }),
      "web",
      CLIENT_LABELS,
    );
    expect(after.card).toBeNull();
    expect(after.items.at(-1)).toMatchObject({ kind: "notice", text: "Answered in the terminal: Yes." });
  });

  it("says nothing extra when this client answered", () => {
    seq = 0;
    const v = run(emptyView(), [
      ev({ type: "plan.proposed", plan: "pl1", markdown: "- step" }),
      ev({ type: "plan.resolved", plan: "pl1", approved: true, answered_by: "web" }),
    ]);
    expect(v.card).toBeNull();
    expect(v.items.map((i) => i.kind)).toEqual(["plan"]);
  });

  it("shows a question card and says how it was answered", () => {
    seq = 0;
    const asked = run(emptyView(), [
      ev({
        type: "question.ask",
        question: "q1",
        items: [{ text: "Which database?", options: ["SQLite", "Postgres"], recommended: "SQLite" }],
        why: "It decides the design",
        expires_at: 1000,
      }),
    ]);
    expect(asked.card).toMatchObject({ kind: "question", request: "q1", why: "It decides the design", expiresAt: 1000 });
    expect(asked.activity).toBe("waiting");
    const answered = apply(
      asked,
      ev({ type: "question.resolved", question: "q1", answers: ["Postgres"], answered_by: "web" }),
      "web",
      CLIENT_LABELS,
    );
    expect(answered.card).toBeNull();
    expect(answered.items.at(-1)).toMatchObject({ kind: "notice", text: "You answered: Postgres" });
    const timedOut = apply(
      asked,
      ev({ type: "question.resolved", question: "q1", answers: ["SQLite"], answered_by: "timeout" }),
      "web",
      CLIENT_LABELS,
    );
    expect(timedOut.items.at(-1)).toMatchObject({
      text: "No answer in time, so Thrall went with its picks: SQLite",
    });
  });

  it("drops a stale card once a reopened session's replay ends", () => {
    seq = 0;
    const opened = openedView(info({ last_seq: 1, state: "idle" }), "m");
    expect(opened.replay).not.toBeNull();
    const v = apply(
      opened,
      ev({
        type: "permission.request",
        request: "r9",
        turn: "t1",
        step: "p1",
        tool: "shell",
        summary: "x",
        risk: "low",
        choices: ["yes", "no"],
      }),
      "web",
      CLIENT_LABELS,
    );
    expect(v.replay).toBeNull();
    expect(v.card).toBeNull();
    expect(v.cost).toBe(0.5);
    expect(v.activity).toBe("idle");
  });

  it("keeps a card that is really still waiting", () => {
    seq = 0;
    const opened = openedView(info({ last_seq: 1, state: "waiting_for_you" }), "m");
    const v = apply(
      opened,
      ev({
        type: "permission.request",
        request: "r9",
        turn: "t1",
        step: "p1",
        tool: "shell",
        summary: "x",
        risk: "low",
        choices: ["yes", "no"],
      }),
      "web",
      CLIENT_LABELS,
    );
    expect(v.card).not.toBeNull();
    expect(v.activity).toBe("waiting");
  });

  it("drops a taken-back turn, keeping what came before", () => {
    seq = 0;
    const v = run(emptyView(), [
      ev({ type: "turn.started", turn: "t1" }),
      ev({ type: "message.started", turn: "t1", message: "u1", role: "user" }),
      ev({ type: "message.completed", message: "u1", text: "first" }),
      ev({ type: "turn.completed", turn: "t1", status: "done", usage: { input_tokens: 1, output_tokens: 1 } }),
      ev({ type: "turn.started", turn: "t2" }),
      ev({ type: "message.started", turn: "t2", message: "u2", role: "user" }),
      ev({ type: "message.completed", message: "u2", text: "second" }),
      ev({ type: "turn.retracted", turn: "t2" }),
    ]);
    expect(v.items.map((i) => (i.kind === "user" ? i.text : i.kind))).toEqual(["first", "turn_end"]);
    // A turn that isn't there changes nothing.
    const same = apply(v, ev({ type: "turn.retracted", turn: "t9" }), "web", CLIENT_LABELS);
    expect(same.items).toBe(v.items);
  });

  it("rewinds to an earlier turn, dropping it and every later one", () => {
    seq = 0;
    const v = run(emptyView(), [
      ev({ type: "turn.started", turn: "t1" }),
      ev({ type: "message.started", turn: "t1", message: "u1", role: "user" }),
      ev({ type: "message.completed", message: "u1", text: "first" }),
      ev({ type: "turn.completed", turn: "t1", status: "done", usage: { input_tokens: 1, output_tokens: 1 } }),
      ev({ type: "turn.started", turn: "t2" }),
      ev({ type: "message.started", turn: "t2", message: "u2", role: "user" }),
      ev({ type: "message.completed", message: "u2", text: "second" }),
      ev({ type: "turn.completed", turn: "t2", status: "done", usage: { input_tokens: 1, output_tokens: 1 } }),
      ev({ type: "turn.started", turn: "t3" }),
      ev({ type: "message.started", turn: "t3", message: "u3", role: "user" }),
      ev({ type: "message.completed", message: "u3", text: "third" }),
    ]);
    // Each message knows its turn, so Rewind can name it.
    expect(v.items.filter((i) => i.kind === "user").map((i) => i.kind === "user" && i.turn)).toEqual([
      "t1",
      "t2",
      "t3",
    ]);
    const back = apply(v, ev({ type: "turn.retracted", turn: "t2" }), "web", CLIENT_LABELS);
    expect(back.items.map((i) => (i.kind === "user" ? i.text : i.kind))).toEqual(["first", "turn_end"]);
    expect(back.turnStarts.map((t) => t.turn)).toEqual(["t1"]);
  });

  it("shows a remembered fact, and marks it when forgotten", () => {
    seq = 0;
    const v = run(emptyView(), [
      ev({ type: "turn.started", turn: "t1" }),
      ev({ type: "memory.saved", fact: "k_1", topic: "editor", content: "uses VS Code" }),
    ]);
    expect(v.items[0]).toEqual({ kind: "fact", id: "k_1", topic: "editor", content: "uses VS Code", forgotten: false });
    const after = markForgotten(v, "k_1");
    expect(after.items[0]).toMatchObject({ kind: "fact", forgotten: true });
    expect(markForgotten(v, "k_9").items[0]).toMatchObject({ forgotten: false });
  });

  it("rests the mark a moment after Done", () => {
    const v = { ...emptyView(), activity: "done" as const, activityAt: 1000 };
    expect(shownActivity(v, 1500, 1600)).toBe("done");
    expect(shownActivity(v, 3000, 1600)).toBe("idle");
  });
});
