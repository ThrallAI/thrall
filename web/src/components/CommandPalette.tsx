// Ctrl+K: type to find any session, slash command or action, then Enter. Arrow keys move, Esc
// closes.

import { useEffect, useMemo, useRef, useState } from "react";

import { PALETTE_MAX } from "../constants";

export interface PaletteItem {
  id: string;
  /** What it's called. */
  label: string;
  /** Small text after it (folder, command summary). */
  hint: string;
  /** Section it belongs to. */
  kind: "Action" | "Session" | "Command";
  run: () => void;
}

/** How well `query` matches `text`: higher is better, 0 is no match. Words can come in any order;
 * each must appear, and a match at the start of a word scores higher. */
export function score(text: string, query: string): number {
  const t = text.toLowerCase();
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return 1;
  let total = 0;
  for (const w of words) {
    const at = t.indexOf(w);
    if (at < 0) return 0;
    const wordStart = at === 0 || /[\s/\\·\-_]/.test(t[at - 1] ?? "");
    total += wordStart ? 3 : 1;
  }
  return total;
}

export function CommandPalette({ items, onClose }: { items: PaletteItem[]; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [pick, setPick] = useState(0);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => input.current?.focus(), []);

  const shown = useMemo(() => {
    return items
      .map((item) => ({ item, s: score(`${item.label} ${item.hint}`, query) }))
      .filter((x) => x.s > 0)
      .sort((a, b) => b.s - a.s)
      .slice(0, PALETTE_MAX)
      .map((x) => x.item);
  }, [items, query]);

  const choose = (item: PaletteItem | undefined) => {
    if (!item) return;
    onClose();
    item.run();
  };

  return (
    <div className="th-modal-scrim th-palette-scrim" onMouseDown={onClose}>
      <div
        className="th-palette"
        role="dialog"
        aria-modal="true"
        aria-label="Go to"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <input
          id="palette"
          ref={input}
          className="th-palette-input"
          placeholder="Jump to a session, command or action"
          aria-label="Search"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setPick(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              setPick((p) => Math.min(p + 1, shown.length - 1));
              e.preventDefault();
            } else if (e.key === "ArrowUp") {
              setPick((p) => Math.max(p - 1, 0));
              e.preventDefault();
            } else if (e.key === "Enter") {
              choose(shown[pick]);
              e.preventDefault();
            } else if (e.key === "Escape") {
              onClose();
              e.preventDefault();
            }
          }}
        />
        <ul className="th-palette-list" role="listbox" aria-label="Results">
          {shown.length === 0 ? <li className="th-palette-empty">Nothing matches.</li> : null}
          {shown.map((item, i) => (
            <li key={item.id}>
              <button
                type="button"
                role="option"
                aria-selected={i === pick}
                className={i === pick ? "is-on" : ""}
                onMouseEnter={() => setPick(i)}
                onClick={() => choose(item)}
              >
                <span className="th-palette-kind">{item.kind}</span>
                <span className="th-palette-label">{item.label}</span>
                <span className="th-palette-hint">{item.hint}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
