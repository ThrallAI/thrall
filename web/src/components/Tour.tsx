// The one-minute tour: each stop outlines part of the real screen (a `data-tour` element) with a
// short note beside it. Skippable at every stop; Esc ends it, the arrow keys move through it.

import { useEffect, useLayoutEffect, useState } from "react";

import { TOUR_EDGE_PX, TOUR_NOTE_PX } from "../constants";
import { TOUR } from "../onboarding";

interface Box {
  top: number;
  left: number;
  width: number;
  height: number;
}

/** Where the stop's element is now, or null when it isn't on screen (the note goes in the middle). */
function boxOf(target: string | undefined): Box | null {
  if (!target) return null;
  const el = document.querySelector(`[data-tour="${target}"]`);
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) return null;
  return { top: r.top, left: r.left, width: r.width, height: r.height };
}

export function Tour({ onDone }: { onDone: () => void }) {
  const [at, setAt] = useState(0);
  const [box, setBox] = useState<Box | null>(null);
  const step = TOUR[at];
  const last = at === TOUR.length - 1;

  useLayoutEffect(() => {
    const place = () => setBox(boxOf(step?.target));
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [step]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDone();
      else if (e.key === "ArrowRight") setAt((i) => Math.min(i + 1, TOUR.length - 1));
      else if (e.key === "ArrowLeft") setAt((i) => Math.max(i - 1, 0));
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onDone]);

  if (!step) return null;
  // The note sits below the element, or above it when the element is low on the screen.
  const below = box ? box.top + box.height / 2 < window.innerHeight / 2 : true;
  const noteStyle = box
    ? {
        left: Math.max(TOUR_EDGE_PX, Math.min(box.left, window.innerWidth - TOUR_NOTE_PX - TOUR_EDGE_PX)),
        ...(below ? { top: box.top + box.height + 12 } : { bottom: window.innerHeight - box.top + 12 }),
      }
    : undefined;
  return (
    <div className="th-tour" role="dialog" aria-modal="true" aria-labelledby="tour-title">
      {box ? (
        <div
          className="th-tour-ring"
          style={{ top: box.top - 6, left: box.left - 6, width: box.width + 12, height: box.height + 12 }}
        />
      ) : (
        <div className="th-tour-dim" />
      )}
      <div className={`th-tour-note${box ? "" : " is-centred"}`} style={noteStyle}>
        <span className="th-tour-count">
          {at + 1} of {TOUR.length}
        </span>
        <h3 id="tour-title">{step.title}</h3>
        <p>{step.body}</p>
        <div className="th-choices">
          {at > 0 ? (
            <button type="button" className="th-btn th-btn-quiet" onClick={() => setAt(at - 1)}>
              Back
            </button>
          ) : null}
          <button
            type="button"
            className="th-btn th-btn-primary"
            onClick={() => (last ? onDone() : setAt(at + 1))}
            autoFocus
          >
            {last ? "Done" : "Next"}
          </button>
          {!last ? (
            <button type="button" className="th-btn" onClick={onDone}>
              Skip the tour
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
