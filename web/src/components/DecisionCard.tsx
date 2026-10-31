// A decision Thrall is waiting on: a permission (with the diff for edits) or a plan to approve.
// The y / a / n keys work as in the terminal while the message box is empty.

import { useEffect } from "react";

import type { Card } from "../transcript";
import { DiffView } from "./DiffView";
import { Markdown } from "./Markdown";

export type Choice =
  | { kind: "permission"; choice: "yes" | "no" | "always" }
  | { kind: "plan"; approve: boolean; autoEdits: boolean };

/** Keys this soon after a card appears are ignored. */
const CARD_SETTLE_MS = 400;

const RISK_LABEL ={ low: "Low risk", medium: "Worth a look", high: "Could do damage" } as const;

export function DecisionCard({
  card,
  onChoose,
  keysActive,
}: {
  card: Card;
  onChoose: (choice: Choice) => void;
  keysActive: boolean;
}) {
  useEffect(() => {
    if (!keysActive) return;
    const shownAt = performance.now();
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey || e.metaKey || e.altKey || e.repeat) return;
      // Keys pressed as the card appears were meant for something else.
      if (performance.now() - shownAt < CARD_SETTLE_MS) return;
      // Typing in another field (a rename, the folder box) isn't an answer.
      const target = e.target as HTMLElement | null;
      if (target && target.tagName === "INPUT") return;
      const key = e.key.toLowerCase();
      if (card.kind === "permission") {
        if (key === "y") onChoose({ kind: "permission", choice: "yes" });
        else if (key === "n") onChoose({ kind: "permission", choice: "no" });
        else if (key === "a" && card.always) onChoose({ kind: "permission", choice: "always" });
        else return;
      } else {
        if (key === "y") onChoose({ kind: "plan", approve: true, autoEdits: false });
        else if (key === "a") onChoose({ kind: "plan", approve: true, autoEdits: true });
        else if (key === "n") onChoose({ kind: "plan", approve: false, autoEdits: false });
        else return;
      }
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [card, onChoose, keysActive]);

  if (card.kind === "plan") {
    return (
      <section className="th-card" aria-label="Approve this plan?">
        <h3>Approve this plan?</h3>
        <p className="th-why">Plan mode is on: Thrall changes nothing until you approve.</p>
        <div className="th-choices">
          <button
            type="button"
            className="th-btn th-btn-primary th-yes"
            onClick={() => onChoose({ kind: "plan", approve: true, autoEdits: false })}
          >
            <span className="th-key">y</span> Approve
          </button>
          <button
            type="button"
            className="th-btn"
            onClick={() => onChoose({ kind: "plan", approve: true, autoEdits: true })}
          >
            <span className="th-key">a</span> Approve, auto-accept edits
          </button>
          <button
            type="button"
            className="th-btn th-no"
            onClick={() => onChoose({ kind: "plan", approve: false, autoEdits: false })}
          >
            <span className="th-key">n</span> Keep planning
          </button>
        </div>
      </section>
    );
  }

  return (
    <section className="th-card" aria-label="Permission request">
      <span className={`th-risk is-${card.risk}`}>{RISK_LABEL[card.risk]}</span>
      <div className="th-card-title">
        <Markdown text={card.summary} />
      </div>
      {card.diff ? <DiffView diff={card.diff} /> : null}
      <div className="th-choices">
        <button
          type="button"
          className="th-btn th-btn-primary th-yes"
          onClick={() => onChoose({ kind: "permission", choice: "yes" })}
        >
          <span className="th-key">y</span> Yes
        </button>
        {card.always ? (
          <button type="button" className="th-btn" onClick={() => onChoose({ kind: "permission", choice: "always" })}>
            <span className="th-key">a</span> Always (this session)
          </button>
        ) : null}
        <button
          type="button"
          className="th-btn th-no"
          onClick={() => onChoose({ kind: "permission", choice: "no" })}
        >
          <span className="th-key">n</span> No
        </button>
      </div>
    </section>
  );
}
