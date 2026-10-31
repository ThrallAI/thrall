// A new person's first look: what Thrall is in three lines, starting it at sign-in (the owner,
// where the computer can), and the offer of the one-minute tour.

import { useEffect, useRef, useState } from "react";

import type { Engine, ReplyOf } from "../engine";
import { STARTUP_LABEL } from "../constants";

export function Welcome({
  name,
  owner,
  engine,
  onTour,
  onDone,
}: {
  name: string;
  owner: boolean;
  engine: Engine | null;
  onTour: () => void;
  onDone: () => void;
}) {
  const tour = useRef<HTMLButtonElement>(null);
  const [startup, setStartup] = useState<ReplyOf<"startup.shown"> | null>(null);
  useEffect(() => {
    tour.current?.focus();
    if (!owner) return;
    engine
      ?.request<"startup.shown">({ type: "startup.get" })
      .then(setStartup)
      .catch(() => setStartup(null));
  }, [engine, owner]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDone();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onDone]);
  const flip = () => {
    if (!startup) return;
    engine
      ?.request<"startup.shown">({ type: "startup.set", on: !startup.on })
      .then(setStartup)
      .catch(() => undefined);
  };
  const label = startup?.label || STARTUP_LABEL;
  return (
    <div className="th-modal-scrim">
      <div className="th-modal th-welcome" role="dialog" aria-modal="true" aria-labelledby="welcome-title">
        <h3 id="welcome-title">Welcome to Thrall{name ? `, ${name}` : ""}</h3>
        <ul>
          <li>Thrall is an AI agent that works on this computer: your files, commands and the web.</li>
          <li>It asks before it changes anything, unless you choose a mode that doesn't.</li>
          <li>It thinks with your own AI key; your sessions stay on this computer.</li>
        </ul>
        {startup?.available ? (
          <label className="th-welcome-row">
            <button
              type="button"
              role="switch"
              aria-checked={startup.on}
              aria-label={label}
              className={`th-switch${startup.on ? " is-on" : ""}`}
              onClick={flip}
            >
              <span />
            </button>
            <span>{label}, so the phone app works after a restart</span>
          </label>
        ) : null}
        <div className="th-choices">
          <button ref={tour} type="button" className="th-btn th-btn-primary" onClick={onTour}>
            Take the 1-minute tour
          </button>
          <button type="button" className="th-btn" onClick={onDone}>
            Not now
          </button>
        </div>
      </div>
    </div>
  );
}
