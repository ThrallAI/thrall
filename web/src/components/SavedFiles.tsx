// Files a step wrote on the computer (a saved document, a picture's copy, a download), as cards
// under the step. In the desktop app they open with their usual program or show in their folder;
// in a browser they download. The engine finds each file from the step that wrote it.

import { createContext, useContext, useState } from "react";

import { CLIENT_KIND } from "../constants";
import type { SavedFile } from "../protocol";
import { size } from "./Attachments";

/** What a file card can do, for the session on screen. */
export type SavedFileActions = {
  /** Open it, or show it in its folder (the desktop app does it). */
  open: (step: string, index: number, reveal: boolean) => Promise<unknown>;
  /** Its bytes, for a download. */
  read: (step: string, index: number) => Promise<Blob>;
};

export const SavedFileActionsContext = createContext<SavedFileActions | null>(null);

function why(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function SavedCard({ step, index, file }: { step: string; index: number; file: SavedFile }) {
  const actions = useContext(SavedFileActionsContext);
  const [failed, setFailed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = (work: (a: SavedFileActions) => Promise<unknown>) => {
    if (!actions || busy) return;
    setFailed(null);
    setBusy(true);
    work(actions)
      .catch((e) => setFailed(why(e)))
      .finally(() => setBusy(false));
  };
  const download = () =>
    run((a) =>
      a.read(step, index).then((blob) => {
        const url = URL.createObjectURL(blob);
        const link = document.createElement("a");
        link.href = url;
        link.download = file.name;
        document.body.append(link);
        link.click();
        link.remove();
        // Long enough for the download to start.
        setTimeout(() => URL.revokeObjectURL(url), 60_000);
      }),
    );
  const desktop = CLIENT_KIND === "desktop";
  return (
    <div className={`th-saved${failed ? " is-failed" : ""}`} title={failed ?? file.path}>
      <span aria-hidden="true">{failed ? "✗" : "▤"}</span>
      <span className="th-file-name">{file.name}</span>
      <span className="th-file-size">{size(file.size)}</span>
      {desktop ? (
        <>
          <button type="button" className="th-copy" disabled={!actions || busy} onClick={() => run((a) => a.open(step, index, false))}>
            Open
          </button>
          <button type="button" className="th-copy" disabled={!actions || busy} onClick={() => run((a) => a.open(step, index, true))}>
            Show in folder
          </button>
        </>
      ) : (
        <button type="button" className="th-copy" disabled={!actions || busy} onClick={download}>
          {busy ? "Downloading…" : "Download"}
        </button>
      )}
      {failed ? <span className="th-saved-why">{failed}</span> : null}
    </div>
  );
}

export function SavedFiles({ step, files }: { step: string; files: SavedFile[] }) {
  if (files.length === 0) return null;
  return (
    <div className="th-files is-start">
      {files.map((f, i) => (
        <SavedCard key={f.path} step={step} index={i} file={f} />
      ))}
    </div>
  );
}
