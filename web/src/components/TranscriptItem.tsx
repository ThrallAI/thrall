// One entry in the conversation. Memoised: an item redraws only when it changes, so a long
// session stays fast while new text streams in.

import { memo } from "react";

import type { Item } from "../transcript";
import { Attachments } from "./Attachments";
import { CopyButton } from "./CopyButton";
import { DiffView } from "./DiffView";
import { Markdown } from "./Markdown";

function money(usd: number): string {
  return usd < 0.01 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
}

const STEP_GLYPH: Record<string, string> = {
  ok: "✓",
  error: "✗",
  warn: "!",
  denied: "✗",
  cancelled: "–",
};

export const TranscriptItem = memo(function TranscriptItem({
  item,
  expanded,
  onEdit,
  onRetry,
  onRewind,
  onForget,
}: {
  item: Item;
  expanded: boolean;
  /** Only on your last message, when Thrall isn't working. */
  onEdit?: () => void;
  /** Only on Thrall's last reply, when it isn't working. */
  onRetry?: () => void;
  /** Go back to before a message's turn; only when Thrall isn't working. */
  onRewind?: (turn: string) => void;
  /** Forget a fact Thrall remembered. */
  onForget?: (fact: string) => void;
}) {
  switch (item.kind) {
    case "fact":
      return (
        <div className={`th-fact${item.forgotten ? " is-forgotten" : ""}`}>
          <span className="th-fact-label">{item.forgotten ? "Forgotten" : "Remembered"}</span>
          <span className="th-fact-text">
            <strong>{item.topic}</strong>: {item.content}
          </span>
          {!item.forgotten && onForget ? (
            <button
              type="button"
              className="th-copy"
              onClick={() => onForget(item.id)}
              title="Thrall won't recall this any more"
            >
              Forget
            </button>
          ) : null}
        </div>
      );
    case "user": {
      const turn = item.turn;
      return (
        <div className="th-user-wrap">
          <Attachments files={item.files} />
          <div className="th-msg-user">{item.text}</div>
          {onEdit || (onRewind && turn) ? (
            <div className="th-msg-tools th-user-tools">
              {onRewind && turn ? (
                <button
                  type="button"
                  className="th-copy"
                  onClick={() => onRewind(turn)}
                  title="Go back to before this message: put files back, the conversation, or both"
                >
                  Rewind
                </button>
              ) : null}
              {onEdit ? (
                <button type="button" className="th-copy" onClick={onEdit} title="Change it and send again">
                  Edit
                </button>
              ) : null}
            </div>
          ) : null}
        </div>
      );
    }
    case "assistant":
      if (!item.text && !item.streaming) return null;
      return (
        <div className="th-msg-thrall">
          <span className="th-mark" aria-label="Thrall">
            ●
          </span>
          <div className="md-wrap">
            {item.thinking && !item.text ? <div className="th-thinking">Thinking…</div> : null}
            {item.text ? <Markdown text={item.text} /> : null}
            {item.streaming ? <span className="th-caret" aria-hidden="true" /> : null}
            {!item.streaming && item.text ? (
              <div className="th-msg-tools">
                <CopyButton text={() => item.text} />
                {onRetry ? (
                  <button
                    type="button"
                    className="th-copy"
                    onClick={onRetry}
                    title="Ask again: this reply is replaced and the files it changed are put back."
                  >
                    Retry
                  </button>
                ) : null}
              </div>
            ) : null}
          </div>
        </div>
      );
    case "step": {
      const running = item.status === null;
      const cls = running ? "is-running" : `is-${item.status}`;
      const glyph = running ? "◐" : (STEP_GLYPH[item.status ?? ""] ?? "·");
      const open = expanded || item.status === "error" || (running && item.output !== "");
      const hasBody = item.output !== "" || item.diff !== null;
      return (
        <>
        <details className={`th-step ${cls}`} open={open && hasBody}>
          <summary>
            <span className="th-glyph" aria-hidden="true">
              {glyph}
            </span>
            <span className="th-title">{item.title}</span>
            {item.summary ? <span className="th-summary">· {item.summary}</span> : null}
          </summary>
          {item.output ? <pre>{item.output}</pre> : null}
          {item.diff ? (
            <div className="th-diff-wrap">
              <DiffView diff={item.diff} />
            </div>
          ) : null}
        </details>
        {item.files.length > 0 ? <Attachments files={item.files} align="start" /> : null}
        </>
      );
    }
    case "notice":
      return <div className={`th-notice${item.level === "warning" ? " is-warning" : ""}`}>{item.text}</div>;
    case "error":
      return (
        <div className="th-notice is-error" role="alert">
          {item.text}
        </div>
      );
    case "command":
      return (
        <div className="th-command">
          <div className="th-command-name">/{item.name}</div>
          <Markdown text={item.markdown} />
        </div>
      );
    case "plan":
      return (
        <div className="th-msg-thrall">
          <span className="th-mark" aria-label="Thrall">
            ●
          </span>
          <div className="th-plan-block">
            <p className="th-label" style={{ margin: "0 0 8px" }}>
              Plan
            </p>
            <Markdown text={item.markdown} />
          </div>
        </div>
      );
    case "turn_end": {
      const u = item.usage;
      const parts = [`${u.input_tokens} in`, `${u.output_tokens} out`];
      if (typeof u.cost_usd === "number") parts.push(money(u.cost_usd));
      if (item.status === "interrupted") parts.unshift("Stopped");
      return (
        <div className="th-turn-end">
          <span className="th-meter">{parts.join(" · ")}</span>
        </div>
      );
    }
  }
});

export { money };
