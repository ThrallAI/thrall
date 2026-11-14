// Files attached to a message: pictures as thumbnails (click to enlarge), everything else as a
// chip with its kind and size. Pictures and clips are fetched with the access token and kept as
// object URLs, so scrolling a session doesn't fetch them again; only the latest few are kept,
// and switching sessions lets them all go.

import { createContext, useContext, useEffect, useState } from "react";

import { KEPT_FILES } from "../constants";
import type { FileRef } from "../protocol";

/** Fetches an attached file's bytes (the engine client's `download`). */
export const FileLoader = createContext<((file: string) => Promise<Blob>) | null>(null);

// Least recently used first (a Map keeps the order things were added in).
const urls = new Map<string, Promise<string>>();

function letGo(file: string, url: Promise<string>): void {
  if (urls.get(file) === url) urls.delete(file);
  url.then((u) => URL.revokeObjectURL(u), () => undefined);
}

export function pictureUrl(load: (file: string) => Promise<Blob>, file: string): Promise<string> {
  let url = urls.get(file);
  if (url) {
    // Used again: it moves to the newest end, so the ones let go are the ones not looked at.
    urls.delete(file);
    urls.set(file, url);
  } else {
    const fetched = load(file).then((blob) => URL.createObjectURL(blob));
    // A failed fetch can be tried again later.
    fetched.catch(() => {
      if (urls.get(file) === fetched) urls.delete(file);
    });
    urls.set(file, fetched);
    url = fetched;
    for (const [old, u] of urls) {
      if (urls.size <= KEPT_FILES) break;
      letGo(old, u);
    }
  }
  return url;
}

/** Lets go of every loaded picture and clip (a different session is opening). */
export function forgetFiles(): void {
  for (const [file, url] of urls) letGo(file, url);
}

/** How many pictures and clips are loaded (for tests). */
export function keptFiles(): number {
  return urls.size;
}

export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const KIND_GLYPH: Record<string, string> = { document: "▤", text: "≡", other: "◇" };

/** A file that isn't a picture or a clip: click to save it (a PDF Thrall made, a document). */
function FileChip({ file }: { file: FileRef }) {
  const load = useContext(FileLoader);
  const [failed, setFailed] = useState(false);
  const save = () => {
    if (!load) return;
    setFailed(false);
    load(file.file)
      .then((blob) => {
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = file.name;
        document.body.append(a);
        a.click();
        a.remove();
        // Long enough for the download to start.
        setTimeout(() => URL.revokeObjectURL(url), 60_000);
      })
      .catch(() => setFailed(true));
  };
  return (
    <button
      type="button"
      className={`th-file-chip${failed ? " is-failed" : ""}`}
      title={failed ? `${file.name} couldn't be downloaded. Click to try again.` : `Save ${file.name}`}
      onClick={save}
      disabled={!load}
    >
      <span aria-hidden="true">{failed ? "✗" : (KIND_GLYPH[file.kind] ?? "◇")}</span>
      <span className="th-file-name">{file.name}</span>
      <span className="th-file-size">{size(file.size)}</span>
    </button>
  );
}

function Picture({ file, onOpen }: { file: FileRef; onOpen: (url: string, name: string) => void }) {
  const load = useContext(FileLoader);
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!load) return;
    let live = true;
    pictureUrl(load, file.file)
      .then((u) => live && setUrl(u))
      .catch(() => live && setFailed(true));
    return () => {
      live = false;
    };
  }, [load, file.file]);
  if (failed) {
    return (
      <span className="th-file-chip is-failed" title="This picture couldn't be loaded.">
        ✗ {file.name}
      </span>
    );
  }
  return (
    <button
      type="button"
      className="th-thumb"
      title={`${file.name} · ${size(file.size)}`}
      onClick={() => url && onOpen(url, file.name)}
      disabled={!url}
    >
      {url ? <img src={url} alt={file.name} /> : <span className="th-thumb-wait" aria-label="Loading" />}
    </button>
  );
}

/** A recording or video with play controls (fetched once with the access token, like pictures). */
function MediaClip({ file }: { file: FileRef }) {
  const load = useContext(FileLoader);
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!load) return;
    let live = true;
    pictureUrl(load, file.file)
      .then((u) => live && setUrl(u))
      .catch(() => live && setFailed(true));
    return () => {
      live = false;
    };
  }, [load, file.file]);
  if (failed) {
    return <span className="th-file-chip is-failed">✗ {file.name}</span>;
  }
  return (
    <div className="th-audio" title={`${file.name} · ${size(file.size)}`}>
      {url ? (
        file.kind === "video" ? (
          <video controls preload="metadata" src={url} aria-label={file.name} />
        ) : (
          <audio controls preload="metadata" src={url} aria-label={file.name} />
        )
      ) : (
        <span className="th-thumb-wait" />
      )}
      <span className="th-file-name">{file.name}</span>
    </div>
  );
}

/** A picture filling the window; Escape or a click closes it. */
export function Lightbox({ url, name, onClose }: { url: string; name: string; onClose: () => void }) {
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [onClose]);
  return (
    <div className="th-lightbox" role="dialog" aria-label={name} onClick={onClose}>
      <img src={url} alt={name} />
      <span className="th-lightbox-name">{name}</span>
    </div>
  );
}

export function Attachments({ files, align = "end" }: { files: FileRef[]; align?: "start" | "end" }) {
  const [open, setOpen] = useState<{ url: string; name: string } | null>(null);
  if (files.length === 0) return null;
  const pictures = files.filter((f) => f.kind === "image");
  const clips = files.filter((f) => f.kind === "audio" || f.kind === "video");
  const others = files.filter((f) => !["image", "audio", "video"].includes(f.kind));
  return (
    <div className={`th-files${align === "start" ? " is-start" : ""}`}>
      {pictures.length > 0 ? (
        <div className="th-thumbs">
          {pictures.map((f) => (
            <Picture key={f.file} file={f} onOpen={(url, name) => setOpen({ url, name })} />
          ))}
        </div>
      ) : null}
      {clips.map((f) => (
        <MediaClip key={f.file} file={f} />
      ))}
      {others.map((f) => (
        <FileChip key={f.file} file={f} />
      ))}
      {open ? <Lightbox url={open.url} name={open.name} onClose={() => setOpen(null)} /> : null}
    </div>
  );
}
