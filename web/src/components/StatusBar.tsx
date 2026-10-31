// Under the message box: the permission mode (click or Shift+Tab to change), the folder, the
// model and what this session has cost so far.

import { MODE_LABELS } from "../constants";
import { money } from "./TranscriptItem";

export function StatusBar({
  mode,
  workspace,
  model,
  cost,
  onCycleMode,
}: {
  mode: string;
  workspace: string;
  model: string;
  cost: number;
  onCycleMode: () => void;
}) {
  return (
    <div className="th-status">
      <div className="th-row">
        <button
          type="button"
          className={`th-chip is-${mode}`}
          onClick={onCycleMode}
          title="Change mode (Shift+Tab)"
          data-tour="mode"
        >
          {MODE_LABELS[mode] ?? mode}
        </button>
        {workspace ? <span className="th-meter" title={workspace}>{workspace}</span> : null}
      </div>
      <div className="th-row">
        {model ? <span className="th-meter">{model}</span> : null}
        <DotReadout text={money(cost)} label={`Spent ${money(cost)} in this session`} />
      </div>
    </div>
  );
}

/** A Doto readout whose decimal point is one round dot (Doto's own full stop reads as a "+"). */
function DotReadout({ text, label }: { text: string; label: string }) {
  const parts = text.split(".");
  return (
    <span className="th-readout" role="img" aria-label={label} title={label} data-tour="cost">
      {parts.map((part, i) => (
        <span key={i}>
          {i > 0 ? <span className="th-point" aria-hidden="true" /> : null}
          {part}
        </span>
      ))}
    </span>
  );
}
