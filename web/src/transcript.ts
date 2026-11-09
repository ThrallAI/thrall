// What a session looks like on screen, built from protocol events (live or replayed). Pure:
// `apply` returns a new view and leaves unchanged items as they were, so React redraws only
// what changed (long sessions stay fast while text streams in).

import type { Diff, EventFrame, FileRef, ModeLimit, QuestionItem, TaskItem, Usage } from "./protocol";
import type { Session as SessionInfo } from "./types";
import { CHECK_IN_AGENT, MODE_UNTIL_TASK_END, QUESTION_TIMEOUT } from "./constants";

export type StepStatus = "ok" | "error" | "warn" | "denied" | "cancelled";

export type Item =
  | { kind: "user"; id: string; text: string; files: FileRef[]; turn: string | null }
  | { kind: "assistant"; id: string; text: string; streaming: boolean; thinking: boolean }
  | {
      kind: "step";
      id: string;
      title: string;
      tool: string;
      status: StepStatus | null;
      summary: string;
      output: string;
      diff: Diff | null;
      /** Files the step made (pictures). */
      files: FileRef[];
      /** The step of the helper that took this step; it's listed under that one. */
      helper: string | null;
    }
  | { kind: "notice"; id: string; level: "info" | "warning"; text: string }
  | { kind: "fact"; id: string; topic: string; content: string; forgotten: boolean }
  | { kind: "error"; id: string; text: string }
  | { kind: "command"; id: string; name: string; markdown: string }
  | { kind: "plan"; id: string; markdown: string }
  | { kind: "turn_end"; id: string; usage: Usage; status: string };

/** A decision waiting for the user. */
export type Card =
  | {
      kind: "permission";
      request: string;
      summary: string;
      risk: "low" | "medium" | "high";
      diff: Diff | null;
      always: boolean;
    }
  | { kind: "plan"; request: string; markdown: string }
  | { kind: "question"; request: string; items: QuestionItem[]; why: string; expiresAt: number };

/** What the living mark shows. */
export type Activity = "idle" | "thinking" | "tool" | "writing" | "waiting" | "done" | "failed";

export interface View {
  items: Item[];
  tasks: TaskItem[];
  card: Card | null;
  activity: Activity;
  /** When the activity last changed (the mark holds Done briefly, then rests). */
  activityAt: number;
  busy: boolean;
  /** Last stored event seen; a reconnect resumes after it. */
  lastSeq: number;
  /** While replaying a reopened session: the seq that ends the replay, and the state to settle in. */
  replay: { until: number; state: SessionInfo["state"]; cost: number } | null;
  title: string;
  mode: string;
  /** How long Full auto lasts, while it's on. */
  modeLimit: ModeLimit | null;
  /** A task just ended with the chat still in a mode that doesn't ask: offer to go back to Ask. */
  offerAsk: boolean;
  model: string;
  cost: number;
  /** Set between a user message's start and its text. */
  pendingUser: boolean;
  /** The session's turns and where each one's items start, so an edit, retry or rewind can take
   * turns back. */
  turnStarts: { turn: string; index: number }[];
}

export function emptyView(model = ""): View {
  return {
    items: [],
    tasks: [],
    card: null,
    activity: "idle",
    activityAt: 0,
    busy: false,
    lastSeq: 0,
    replay: null,
    title: "",
    mode: "ask",
    modeLimit: null,
    offerAsk: false,
    model,
    cost: 0,
    pendingUser: false,
    turnStarts: [],
  };
}

/** A view for a session that's just been opened; its stored events follow. */
export function openedView(info: SessionInfo, model: string): View {
  const view: View = {
    ...emptyView(model),
    title: info.title,
    mode: info.mode,
    modeLimit: info.mode === "full_auto" ? (info.mode_limit ?? null) : null,
    cost: info.cost_usd ?? 0,
  };
  if (info.last_seq === 0) return settle(view, info.state, view.cost);
  return { ...view, replay: { until: info.last_seq, state: info.state, cost: view.cost } };
}

/** The chat is in a mode that goes ahead without asking and stays in it after the task. */
function keepsGoing(v: View): boolean {
  return v.mode === "auto_edits" || (v.mode === "full_auto" && v.modeLimit?.until !== MODE_UNTIL_TASK_END);
}

function activityFor(state: SessionInfo["state"]): Activity {
  if (state === "waiting_for_you") return "waiting";
  if (state === "working") return "thinking";
  return "idle";
}

/** The replay is over: a card from a turn that's no longer waiting is stale. */
function settle(view: View, state: SessionInfo["state"], cost: number): View {
  return {
    ...view,
    replay: null,
    card: state === "waiting_for_you" ? view.card : null,
    busy: state !== "idle",
    activity: activityFor(state),
    cost,
  };
}

let localIds = 0;
/** Ids for items the client adds itself (notices, command results). */
export function localId(prefix: string): string {
  localIds += 1;
  return `${prefix}-local-${localIds}`;
}

function update<K extends Item["kind"]>(
  items: Item[],
  kind: K,
  id: string,
  change: (item: Extract<Item, { kind: K }>) => Extract<Item, { kind: K }>,
): Item[] {
  // Search from the end: the item being changed is almost always the newest.
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];
    if (item && item.kind === kind && item.id === id) {
      const next = items.slice();
      next[i] = change(item as Extract<Item, { kind: K }>);
      return next;
    }
  }
  return items;
}

function setActivity(view: View, activity: Activity, at: number): View {
  return view.activity === activity ? view : { ...view, activity, activityAt: at };
}

/**
 * Add a step. A helper's step goes right after that helper's row and its steps so far, so helpers
 * working side by side each keep their own list; any other step goes at the end.
 */
function underHelper(items: Item[], step: Item & { kind: "step" }): Item[] {
  const helper = step.helper;
  if (helper) {
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it?.kind === "step" && (it.id === helper || it.helper === helper)) {
        return [...items.slice(0, i + 1), step, ...items.slice(i + 1)];
      }
    }
  }
  return [...items, step];
}

/** Apply one event. `by` is this client's kind, so answers from elsewhere can be named. */
export function apply(view: View, frame: EventFrame, by: string, labels: Record<string, string>): View {
  let v = view;
  const at = frame.at;
  if (typeof frame.seq === "number" && frame.seq > v.lastSeq) v = { ...v, lastSeq: frame.seq };
  switch (frame.type) {
    case "turn.started":
      v = setActivity(
        { ...v, busy: true, turnStarts: [...v.turnStarts, { turn: frame.turn, index: v.items.length }] },
        "thinking",
        at,
      );
      break;
    case "turn.retracted": {
      // The turn goes, with every later one.
      const at = v.turnStarts.findIndex((t) => t.turn === frame.turn);
      const start = v.turnStarts[at];
      if (start) {
        v = { ...v, items: v.items.slice(0, start.index), turnStarts: v.turnStarts.slice(0, at) };
      }
      break;
    }
    case "message.started":
      if (frame.role === "user") v = { ...v, pendingUser: true };
      else
        v = {
          ...v,
          items: [
            ...v.items,
            { kind: "assistant", id: frame.message, text: "", streaming: true, thinking: false },
          ],
        };
      break;
    case "message.delta":
      v = setActivity(
        {
          ...v,
          items: update(v.items, "assistant", frame.message, (m) => ({
            ...m,
            text: m.text + frame.text,
            thinking: false,
          })),
        },
        "writing",
        at,
      );
      break;
    case "reasoning.delta":
      v = setActivity(
        {
          ...v,
          items: update(v.items, "assistant", frame.message, (m) =>
            m.thinking ? m : { ...m, thinking: true },
          ),
        },
        "thinking",
        at,
      );
      break;
    case "message.completed":
      if (v.pendingUser) {
        v = {
          ...v,
          pendingUser: false,
          items: [
            ...v.items,
            {
              kind: "user",
              id: frame.message,
              text: frame.text,
              files: frame.files ?? [],
              turn: v.turnStarts[v.turnStarts.length - 1]?.turn ?? null,
            },
          ],
        };
      } else {
        v = {
          ...v,
          items: update(v.items, "assistant", frame.message, (m) => ({
            ...m,
            text: frame.text,
            streaming: false,
            thinking: false,
          })),
        };
      }
      break;
    case "step.started": {
      const step: Item & { kind: "step" } = {
        kind: "step",
        id: frame.step,
        title: frame.title,
        tool: frame.tool,
        status: null,
        summary: "",
        output: "",
        diff: null,
        files: [],
        helper: frame.helper ?? null,
      };
      v = setActivity({ ...v, items: underHelper(v.items, step) }, "tool", at);
      break;
    }
    case "step.output":
      v = {
        ...v,
        items: update(v.items, "step", frame.step, (s) => ({ ...s, output: s.output + frame.text })),
      };
      break;
    case "step.completed":
      v = setActivity(
        {
          ...v,
          items: update(v.items, "step", frame.step, (s) => ({
            ...s,
            status: frame.status,
            summary: frame.summary,
            diff: frame.diff ?? null,
            files: frame.files ?? [],
          })),
        },
        "thinking",
        at,
      );
      break;
    case "tasks.updated":
      v = { ...v, tasks: frame.items };
      break;
    case "permission.request":
      v = setActivity(
        {
          ...v,
          card: {
            kind: "permission",
            request: frame.request,
            summary: frame.summary,
            risk: frame.risk,
            diff: frame.diff ?? null,
            always: frame.choices.includes("always"),
          },
        },
        "waiting",
        at,
      );
      break;
    case "plan.proposed":
      v = setActivity(
        {
          ...v,
          items: [...v.items, { kind: "plan", id: frame.plan, markdown: frame.markdown }],
          card: { kind: "plan", request: frame.plan, markdown: frame.markdown },
        },
        "waiting",
        at,
      );
      break;
    case "question.ask":
      v = setActivity(
        {
          ...v,
          card: {
            kind: "question",
            request: frame.question,
            items: frame.items,
            why: frame.why ?? "",
            expiresAt: frame.expires_at,
          },
        },
        "waiting",
        at,
      );
      break;
    case "question.resolved": {
      const mine = v.card?.request === frame.question;
      v = mine ? { ...v, card: null } : v;
      const answers = frame.answers.join(" · ");
      const text =
        frame.answered_by === QUESTION_TIMEOUT
          ? `No answer in time, so Thrall went with its picks: ${answers}`
          : frame.answered_by === by
            ? `You answered: ${answers}`
            : `Answered ${labels[frame.answered_by] ?? `on ${frame.answered_by}`}: ${answers}`;
      v = { ...v, items: [...v.items, { kind: "notice", id: `q-${frame.question}`, level: "info", text }] };
      v = setActivity(v, "thinking", at);
      break;
    }
    case "permission.resolved":
    case "plan.resolved": {
      const request = frame.type === "permission.resolved" ? frame.request : frame.plan;
      const mine = v.card?.request === request;
      v = mine ? { ...v, card: null } : v;
      if (frame.answered_by !== by) {
        const where = labels[frame.answered_by] ?? `on ${frame.answered_by}`;
        const what =
          frame.type === "permission.resolved"
            ? `Answered ${where}: ${frame.choice === "no" ? "No" : frame.choice === "always" ? "Always" : "Yes"}.`
            : `Plan ${frame.approved ? "approved" : "sent back"} ${where}.`;
        v = { ...v, items: [...v.items, { kind: "notice", id: `r-${request}`, level: "info", text: what }] };
      }
      v = setActivity(v, "thinking", at);
      break;
    }
    case "session.updated":
      v = { ...v, title: frame.title ?? v.title };
      // Replayed mode changes are history: the saved mode the chat opened with is the real one.
      if (frame.mode && !v.replay)
        v = {
          ...v,
          mode: frame.mode,
          modeLimit: frame.mode === "full_auto" ? (frame.mode_limit ?? null) : null,
          offerAsk: false,
        };
      break;
    case "status":
      // A task that ran in a mode that doesn't ask has ended (Full auto that ends with the task
      // switches itself back, so it needs no offer).
      if (frame.state === "idle" && v.busy && !v.replay && keepsGoing(v))
        v = { ...v, offerAsk: true };
      if (frame.state !== "idle") v = { ...v, offerAsk: false };
      v = { ...v, model: frame.model, busy: frame.state !== "idle" };
      if (frame.state === "waiting_for_you") v = setActivity(v, "waiting", at);
      else if (frame.state === "idle" && v.activity !== "done" && v.activity !== "failed")
        v = setActivity(v, "idle", at);
      break;
    case "agent.started":
      break;
    case "agent.completed":
      // A check-in's message is already in Home.
      if (frame.agent === CHECK_IN_AGENT) break;
      v = {
        ...v,
        items: [
          ...v.items,
          {
            kind: "notice",
            id: `a-${frame.agent}-${at}`,
            level: frame.status === "done" ? "info" : "warning",
            text: `Background task ${frame.status}: ${frame.title} · ${frame.summary}`,
          },
        ],
      };
      break;
    case "memory.saved":
      v = {
        ...v,
        items: [
          ...v.items,
          { kind: "fact", id: frame.fact, topic: frame.topic, content: frame.content, forgotten: false },
        ],
      };
      break;
    case "notice":
      v = {
        ...v,
        items: [
          ...v.items,
          { kind: "notice", id: `n-${frame.seq ?? at}-${v.items.length}`, level: frame.level, text: frame.text },
        ],
      };
      break;
    case "error":
      v = {
        ...v,
        items: [...v.items, { kind: "error", id: `e-${frame.seq ?? at}-${v.items.length}`, text: frame.message }],
      };
      break;
    case "turn.completed":
      v = {
        ...v,
        busy: false,
        cost: v.cost + (frame.usage.cost_usd ?? 0),
        items: [
          ...v.items.map((i) =>
            i.kind === "assistant" && i.streaming ? { ...i, streaming: false, thinking: false } : i,
          ),
          { kind: "turn_end", id: `t-${frame.turn}`, usage: frame.usage, status: frame.status },
        ],
      };
      v = setActivity(
        v,
        frame.status === "done" ? "done" : frame.status === "failed" ? "failed" : "idle",
        at,
      );
      break;
  }
  if (v.replay && typeof frame.seq === "number" && frame.seq >= v.replay.until) {
    v = settle(v, v.replay.state, v.replay.cost);
  }
  return v;
}

/** The view with a remembered fact shown as forgotten. */
export function markForgotten(view: View, fact: string): View {
  return {
    ...view,
    items: view.items.map((i) => (i.kind === "fact" && i.id === fact ? { ...i, forgotten: true } : i)),
  };
}

/** What the living mark should show now (Done rests after a moment). */
export function shownActivity(view: View, now: number, doneHoldMs: number): Activity {
  if (view.activity === "done" && now - view.activityAt > doneHoldMs) return "idle";
  return view.activity;
}
