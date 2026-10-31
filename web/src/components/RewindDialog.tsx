// Go back to before one of your messages: put files back, take the conversation back, or both.
// Lists the files first; ones changed since Thrall's edit are flagged so you can leave them out.

import { useEffect, useRef, useState } from "react";

import type { Engine } from "../engine";
import type { RewindFile } from "../protocol";

const ACTION_LABEL: Record<RewindFile["action"], string> = {
  restore: "put back",
  delete: "removed (Thrall made it)",
  unavailable: "too big to keep a copy, stays as it is",
};

/** What the app needs after a rewind (the engine posts what was done as a notice). */
export interface Rewound {
  text: string;
  conversation: boolean;
}

export function RewindDialog({
  engine,
  session,
  turn,
  onDone,
  onClose,
}: {
  engine: Engine;
  session: string;
  turn: string;
  onDone: (result: Rewound) => void;
  onClose: () => void;
}) {
  const [text, setText] = useState<string | null>(null);
  const [files, setFiles] = useState<RewindFile[]>([]);
  const [ticked, setTicked] = useState<Set<string>>(new Set());
  const [error, setError] = useState("");
  const [working, setWorking] = useState(false);
  const first = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let live = true;
    engine
      .request<"checkpoint.shown">({ type: "checkpoint.preview", session, turn })
      .then((shown) => {
        if (!live) return;
        setText(shown.text);
        setFiles(shown.files);
        setTicked(new Set(shown.files.filter((f) => f.action !== "unavailable").map((f) => f.path)));
      })
      .catch((e: unknown) => live && setError(e instanceof Error ? e.message : String(e)));
    return () => {
      live = false;
    };
  }, [engine, session, turn]);

  useEffect(() => {
    if (text !== null) first.current?.focus();
  }, [text]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const restorable = files.filter((f) => f.action !== "unavailable");
  const keep = restorable.filter((f) => !ticked.has(f.path)).map((f) => f.path);
  const anyFiles = restorable.length > keep.length;

  const go = async (withFiles: boolean, conversation: boolean) => {
    setWorking(true);
    setError("");
    try {
      const done = await engine.request<"turn.rewound">({
        type: "turn.rewind",
        session,
        turn,
        files: withFiles,
        conversation,
        keep,
      });
      onDone({ text: done.text, conversation });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setWorking(false);
    }
  };

  const toggle = (path: string) =>
    setTicked((s) => {
      const next = new Set(s);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  return (
    <div className="th-modal-scrim" onMouseDown={onClose}>
      <div
        className="th-modal th-rewind"
        role="dialog"
        aria-modal="true"
        aria-labelledby="rewind-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h3 id="rewind-title">Rewind to before this message</h3>
        {text === null && !error ? <p>Looking at what changed…</p> : null}
        {text ? <blockquote className="th-rewind-quote">{text}</blockquote> : null}
        {text !== null ? (
          files.length === 0 ? (
            <p>Thrall's file tools didn't change any files from here on.</p>
          ) : (
            <>
              <p>Files Thrall changed from here on. Untick any you want to leave as they are.</p>
              <ul className="th-rewind-files">
                {files.map((f) => (
                  <li key={f.path}>
                    <label className={f.action === "unavailable" ? "is-off" : undefined}>
                      <input
                        type="checkbox"
                        checked={ticked.has(f.path)}
                        disabled={f.action === "unavailable" || working}
                        onChange={() => toggle(f.path)}
                      />
                      <code>{f.path}</code>
                      <span className="th-rewind-what">{ACTION_LABEL[f.action]}</span>
                      {f.changed ? <span className="th-rewind-changed">changed since Thrall's edit</span> : null}
                    </label>
                  </li>
                ))}
              </ul>
            </>
          )
        ) : null}
        {text !== null ? (
          <p className="th-rewind-note">Changes made by commands Thrall ran (like scripts or installs) aren't tracked.</p>
        ) : null}
        {error ? (
          <p className="th-rewind-error" role="alert">
            {error}
          </p>
        ) : null}
        <div className="th-choices">
          {text !== null && anyFiles ? (
            <>
              <button
                ref={first}
                type="button"
                className="th-btn th-btn-primary"
                disabled={working}
                onClick={() => void go(true, true)}
              >
                Files and conversation
              </button>
              <button type="button" className="th-btn" disabled={working} onClick={() => void go(false, true)}>
                Conversation only
              </button>
              <button type="button" className="th-btn" disabled={working} onClick={() => void go(true, false)}>
                Files only
              </button>
            </>
          ) : text !== null ? (
            <button
              ref={first}
              type="button"
              className="th-btn th-btn-primary"
              disabled={working}
              onClick={() => void go(false, true)}
            >
              Rewind conversation
            </button>
          ) : null}
          <button type="button" className="th-btn" onClick={onClose}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
