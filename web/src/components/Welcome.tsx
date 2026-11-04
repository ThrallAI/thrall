// A new person's first look: what Thrall is in three lines, starting it at sign-in and where its
// working folder is (the owner, where the computer can), and the offer of the one-minute tour.

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
  const [work, setWork] = useState<string | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const loadWork = () =>
    engine
      ?.request<"folders.shown">({ type: "folders.get" })
      .then((r) => setWork(r.work ?? null))
      .catch(() => setWork(null));
  useEffect(() => {
    tour.current?.focus();
    if (!owner) return;
    engine
      ?.request<"startup.shown">({ type: "startup.get" })
      .then(setStartup)
      .catch(() => setStartup(null));
    void loadWork();
  }, [engine, owner]);
  const saveWork = () => {
    if (draft === null) return;
    setProblem(null);
    engine
      ?.request({ type: "settings.set", key: "folders.new_in", value: draft.trim() })
      .then(() => {
        setDraft(null);
        void loadWork();
      })
      .catch((e: unknown) => setProblem(e instanceof Error ? e.message : String(e)));
  };
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
        {work ? (
          <div className="th-welcome-folder">
            <span>What Thrall makes for you goes in</span>
            {draft === null ? (
              <div className="th-welcome-row">
                <code>{work}</code>
                <button type="button" className="th-btn" onClick={() => engine?.request({ type: "folder.open", folder: "work" }).catch(() => undefined)}>
                  Open
                </button>
                <button type="button" className="th-btn" onClick={() => setDraft(work)}>
                  Change
                </button>
              </div>
            ) : (
              <form
                className="th-welcome-row"
                onSubmit={(e) => {
                  e.preventDefault();
                  saveWork();
                }}
              >
                <input
                  className="th-input th-set-mono"
                  aria-label="Thrall's working folder"
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  autoFocus
                />
                <button type="submit" className="th-btn th-btn-primary">
                  Save
                </button>
                <button type="button" className="th-btn" onClick={() => setDraft(null)}>
                  Cancel
                </button>
              </form>
            )}
            {problem ? <small className="th-welcome-problem">{problem}</small> : null}
          </div>
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
