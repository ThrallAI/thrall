// Above the message box: a red bar while Full auto is on (with how long it lasts and a Stop), or,
// after a task ends in a mode that doesn't ask, an offer to go back to Ask.

import { MODE_LABELS, MODE_UNTIL_MANUAL, MODE_UNTIL_TIME } from "../constants";
import type { ModeLimit } from "../protocol";

export function ModeBanner({
  mode,
  limit,
  offerAsk,
  onAsk,
  onKeep,
}: {
  mode: string;
  limit: ModeLimit | null;
  offerAsk: boolean;
  onAsk: () => void;
  onKeep: () => void;
}) {
  if (mode === "full_auto") {
    return (
      <div className="th-modebar is-full_auto" role="status">
        <span>
          <strong>Full auto is on.</strong> Edits and commands go ahead without asking. {lasts(limit)}
        </span>
        <button type="button" className="th-btn" onClick={onAsk}>
          Stop Full auto
        </button>
      </div>
    );
  }
  if (!offerAsk) return null;
  return (
    <div className="th-modebar" role="status">
      <span>This chat is still on {MODE_LABELS[mode] ?? mode}. Switch back to Ask before changes?</span>
      <span className="th-row">
        <button type="button" className="th-btn th-btn-primary" onClick={onAsk}>
          Switch to Ask
        </button>
        <button type="button" className="th-btn th-btn-quiet" onClick={onKeep}>
          Keep it
        </button>
      </span>
    </div>
  );
}

/** When Full auto ends, in plain words. */
function lasts(limit: ModeLimit | null): string {
  if (limit?.until === MODE_UNTIL_MANUAL) return "It stays on until you stop it.";
  if (limit?.until === MODE_UNTIL_TIME && limit.ends_at)
    return `It ends at ${new Date(limit.ends_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}.`;
  return "It ends when this task finishes.";
}
