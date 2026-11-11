// A decision Thrall is waiting on: a permission (with the diff for edits), a plan to approve, or a
// question card. The y / a / n keys work as in the terminal while the message box is empty.

import { useEffect, useState } from "react";

import type { Card } from "../transcript";
import { DiffView } from "./DiffView";
import { Markdown } from "./Markdown";

export type Choice =
  | { kind: "permission"; choice: "yes" | "no" | "always" }
  | { kind: "plan"; approve: boolean; autoEdits: boolean }
  | { kind: "question"; answers: string[] };

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
      if (card.kind === "question") {
        // y goes with Thrall's picks; anything else is answered on the card.
        if (key === "y") onChoose({ kind: "question", answers: card.items.map((q) => q.recommended) });
        else return;
      } else if (card.kind === "permission") {
        if (key === "y") onChoose({ kind: "permission", choice: "yes" });
        else if (key === "n") onChoose({ kind: "permission", choice: "no" });
        else if (key === "a" && card.always) onChoose({ kind: "permission", choice: "always" });
        else return;
      } else {
        // Approving goes on in Auto edits; a keeps asking before each change.
        if (key === "y") onChoose({ kind: "plan", approve: true, autoEdits: true });
        else if (key === "a") onChoose({ kind: "plan", approve: true, autoEdits: false });
        else if (key === "n") onChoose({ kind: "plan", approve: false, autoEdits: false });
        else return;
      }
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [card, onChoose, keysActive]);

  if (card.kind === "question") return <QuestionCard card={card} onChoose={onChoose} />;

  if (card.kind === "plan") {
    return (
      <section className="th-card" aria-label="Approve this plan?">
        <h3>Approve this plan?</h3>
        <p className="th-why">Plan mode is on: Thrall changes nothing until you approve.</p>
        <div className="th-choices">
          <button
            type="button"
            className="th-btn th-btn-primary th-yes"
            onClick={() => onChoose({ kind: "plan", approve: true, autoEdits: true })}
          >
            <span className="th-key">y</span> Approve
          </button>
          <button
            type="button"
            className="th-btn"
            onClick={() => onChoose({ kind: "plan", approve: true, autoEdits: false })}
          >
            <span className="th-key">a</span> Approve, ask before each change
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

type QuestionCardData = Extract<Card, { kind: "question" }>;

/** Thrall's questions, its pick for each already chosen. Any answer can be typed instead. */
function QuestionCard({ card, onChoose }: { card: QuestionCardData; onChoose: (choice: Choice) => void }) {
  const [answers, setAnswers] = useState(() => card.items.map((q) => q.recommended));
  const set = (i: number, value: string) => setAnswers((a) => a.map((old, j) => (j === i ? value : old)));
  const until = new Date(card.expiresAt).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  return (
    <section className="th-card" aria-label="Thrall has a question">
      <h3>{card.items.length > 1 ? "Thrall has a few questions" : "Thrall has a question"}</h3>
      {card.why ? <p className="th-why">{card.why}</p> : null}
      {card.items.map((q, i) => {
        const options = q.options ?? [];
        const custom = !options.includes(answers[i] ?? "");
        return (
          <fieldset key={i} className="th-question">
            <legend>{q.text}</legend>
            {options.map((o) => (
              <label key={o} className="th-option">
                <input type="radio" name={`q-${card.request}-${i}`} checked={answers[i] === o} onChange={() => set(i, o)} />
                {o}
                {o === q.recommended ? <span className="th-pick"> Thrall's pick</span> : null}
              </label>
            ))}
            <input
              className="th-input"
              aria-label={options.length ? "Another answer" : q.text}
              placeholder={options.length ? "Or type another answer" : q.recommended}
              value={custom ? answers[i] ?? "" : ""}
              onChange={(e) => set(i, e.target.value || q.recommended)}
            />
          </fieldset>
        );
      })}
      <p className="th-why">If nobody answers by {until}, Thrall goes ahead with its picks.</p>
      <div className="th-choices">
        <button type="button" className="th-btn th-btn-primary th-yes" onClick={() => onChoose({ kind: "question", answers })}>
          Send answers
        </button>
        <button
          type="button"
          className="th-btn"
          onClick={() => onChoose({ kind: "question", answers: card.items.map((q) => q.recommended) })}
        >
          <span className="th-key">y</span> Go with Thrall's picks
        </button>
      </div>
    </section>
  );
}