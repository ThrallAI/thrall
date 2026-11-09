// Under the message box: the permission mode (click for the menu, Shift+Tab to cycle), the
// folder, the model and what this session has cost so far.

import { useEffect, useRef, useState } from "react";
import { FULL_AUTO_LENGTHS, MODE_CYCLE, MODE_LABELS } from "../constants";
import { money } from "./TranscriptItem";

export function StatusBar({
  mode,
  workspace,
  model,
  cost,
  canFullAuto,
  onSetMode,
}: {
  mode: string;
  workspace: string;
  model: string;
  cost: number;
  /** Full auto is the owner's only. */
  canFullAuto: boolean;
  onSetMode: (mode: string, until?: string, minutes?: number) => void;
}) {
  const [menu, setMenu] = useState<{ left: number; bottom: number } | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    const close = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return;
      setMenu(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenu(null);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu]);

  const pick = (next: string, until?: string, minutes?: number) => {
    setMenu(null);
    onSetMode(next, until, minutes);
  };

  return (
    <div className="th-status">
      <div className="th-row">
        <button
          type="button"
          className={`th-chip is-${mode}`}
          onMouseDown={(e) => {
            // Keeps the window's close-on-click from reopening it straight away.
            if (menu) e.stopPropagation();
          }}
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            setMenu(menu ? null : { left: r.left, bottom: window.innerHeight - r.top + 4 });
          }}
          title="Change mode (Shift+Tab cycles Ask, Auto-accept edits and Plan)"
          aria-haspopup="menu"
          aria-expanded={menu !== null}
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
      {menu ? (
        <div ref={menuRef} className="th-menu" role="menu" style={{ left: menu.left, bottom: menu.bottom }}>
          {MODE_CYCLE.map((m) => (
            <button key={m} type="button" role="menuitemradio" aria-checked={mode === m} onClick={() => pick(m)}>
              {MODE_LABELS[m]}
            </button>
          ))}
          {canFullAuto ? (
            <>
              <p className="th-menu-label">{MODE_LABELS.full_auto}</p>
              {FULL_AUTO_LENGTHS.map((l) => (
                <button
                  key={l.label}
                  type="button"
                  role="menuitem"
                  className="th-menu-indent"
                  onClick={() => pick("full_auto", l.until, l.minutes)}
                >
                  {l.label}
                </button>
              ))}
            </>
          ) : null}
        </div>
      ) : null}
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
