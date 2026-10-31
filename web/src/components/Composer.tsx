// The message box. Typing here never redraws the conversation (its text lives only in this
// component). Enter sends, Shift+Enter adds a line, / opens command suggestions, Esc stops a
// running turn, Shift+Tab switches mode. Drop files on it (or use Attach) to send them along.

import { forwardRef, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState } from "react";

import {
  CLIENT_COMMANDS,
  COMPOSER_MAX_HEIGHT,
  COMPOSER_PLACEHOLDER,
  MAX_UPLOAD_BYTES,
  SUGGESTIONS_MAX,
} from "../constants";
import type { CommandItem } from "../protocol";

export interface ComposerHandle {
  focus(): void;
  isEmpty(): boolean;
  /** Put text in the box (replacing what's there) and focus it. */
  fill(text: string): void;
}

interface Props {
  commands: CommandItem[];
  busy: boolean;
  disabled: boolean;
  onSend: (text: string, files: string[]) => void;
  /** Upload a file; resolves with its id. */
  onUpload: (file: File) => Promise<{ file: string; name: string }>;
  onCommand: (name: string, args: string) => void;
  onStop: () => void;
  onCycleMode: () => void;
  onEmptyChange: (empty: boolean) => void;
  /** Turn a recording into text (the microphone button); resolves with the words. */
  onDictate?: (recording: File) => Promise<string>;
  /** Replacing the last message (Enter sends the new version). */
  editing: boolean;
  onCancelEdit: () => void;
}

interface Attachment {
  key: string;
  name: string;
  state: "uploading" | "ready" | "failed";
  id?: string;
  note?: string;
  /** A local preview, for pictures. */
  preview?: string;
}

export const Composer = forwardRef<ComposerHandle, Props>(function Composer(props, ref) {
  const {
    commands,
    busy,
    disabled,
    onSend,
    onUpload,
    onCommand,
    onStop,
    onCycleMode,
    onEmptyChange,
    editing,
    onCancelEdit,
    onDictate,
  } = props;
  const [files, setFiles] = useState<Attachment[]>([]);
  const [dropping, setDropping] = useState(false);
  const picker = useRef<HTMLInputElement>(null);
  const uploading = files.some((f) => f.state === "uploading");

  const drop = (gone: Attachment[]) => {
    for (const f of gone) if (f.preview) URL.revokeObjectURL(f.preview);
  };
  const attach = (list: FileList | File[] | null) => {
    for (const file of Array.from(list ?? [])) {
      const key = `${file.name}-${Date.now()}-${Math.random()}`;
      if (file.size > MAX_UPLOAD_BYTES) {
        setFiles((fs) => [...fs, { key, name: file.name, state: "failed", note: "Over 25 MB" }]);
        continue;
      }
      const preview = file.type.startsWith("image/") ? URL.createObjectURL(file) : undefined;
      setFiles((fs) => [...fs, { key, name: file.name, state: "uploading", preview }]);
      onUpload(file)
        .then((saved) =>
          setFiles((fs) => fs.map((f) => (f.key === key ? { ...f, state: "ready", id: saved.file, name: saved.name } : f))),
        )
        .catch((err: unknown) =>
          setFiles((fs) =>
            fs.map((f) =>
              f.key === key ? { ...f, state: "failed", note: err instanceof Error ? err.message : "Upload failed" } : f,
            ),
          ),
        );
    }
  };
  const [text, setText] = useState("");
  const [pick, setPick] = useState(0);
  // Dictation: recording, then turning it into text.
  const [mic, setMic] = useState<"off" | "recording" | "transcribing">("off");
  const [micNote, setMicNote] = useState<string | null>(null);
  const recorder = useRef<MediaRecorder | null>(null);
  const toggleMic = async () => {
    if (!onDictate) return;
    if (mic === "recording") {
      recorder.current?.stop();
      return;
    }
    if (mic !== "off") return;
    setMicNote(null);
    let stream: MediaStream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    } catch {
      setMicNote("Thrall can't use the microphone (allow it in the browser or Windows settings).");
      return;
    }
    const chunks: Blob[] = [];
    const rec = new MediaRecorder(stream);
    rec.ondataavailable = (e) => {
      if (e.data.size > 0) chunks.push(e.data);
    };
    rec.onstop = () => {
      stream.getTracks().forEach((t) => t.stop());
      const type = rec.mimeType || "audio/webm";
      const ext = type.includes("ogg") ? "ogg" : type.includes("mp4") ? "m4a" : "webm";
      const file = new File(chunks, `dictation.${ext}`, { type });
      setMic("transcribing");
      onDictate(file)
        .then((words) => {
          const joined = text.trim() ? `${text.trimEnd()} ${words}` : words;
          set(joined);
          requestAnimationFrame(() => box.current?.focus());
        })
        .catch((err: unknown) => setMicNote(err instanceof Error ? err.message : "Couldn't turn that into text."))
        .finally(() => setMic("off"));
    };
    recorder.current = rec;
    rec.start();
    setMic("recording");
  };
  const box = useRef<HTMLTextAreaElement>(null);

  useImperativeHandle(ref, () => ({
    focus: () => box.current?.focus(),
    isEmpty: () => text.trim() === "",
    fill: (value: string) => {
      set(value);
      requestAnimationFrame(() => box.current?.focus());
    },
  }));

  // Grow with the text, up to a limit.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, COMPOSER_MAX_HEIGHT)}px`;
  }, [text]);

  const suggestions = useMemo(() => {
    const m = /^\/(\S*)$/.exec(text);
    if (!m) return [];
    const typed = (m[1] ?? "").toLowerCase();
    return [...commands.map((c) => ({ name: c.name, summary: c.summary })), ...CLIENT_COMMANDS]
      .filter((c) => c.name.startsWith(typed))
      .sort((a, b) => a.name.localeCompare(b.name))
      .slice(0, SUGGESTIONS_MAX);
  }, [text, commands]);

  const set = (next: string) => {
    setText(next);
    setPick(0);
    onEmptyChange(next.trim() === "");
  };

  const submit = () => {
    if (uploading) return;
    let value = text.trim();
    const chosen = suggestions[pick];
    if (chosen && value !== `/${chosen.name}` && chosen.name.startsWith(value.slice(1))) {
      value = `/${chosen.name}`;
    }
    const ready = files.filter((f) => f.state === "ready" && f.id).map((f) => f.id as string);
    if (!value && ready.length > 0) value = "Please look at the attached file.";
    if (!value) return;
    set("");
    const command = /^\/(\S+)\s*([\s\S]*)$/.exec(value);
    const name = (command?.[1] ?? "").toLowerCase();
    // A skill runs as a message: Thrall sees `/name …` and uses the skill.
    const skill = command && commands.some((c) => c.skill && c.name.toLowerCase() === name);
    if (command && !skill) onCommand(name, (command[2] ?? "").trim());
    else {
      onSend(value, ready);
      drop(files);
      setFiles([]);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (suggestions.length > 0) {
      if (e.key === "ArrowDown") {
        setPick((p) => Math.min(p + 1, suggestions.length - 1));
        e.preventDefault();
        return;
      }
      if (e.key === "ArrowUp") {
        setPick((p) => Math.max(p - 1, 0));
        e.preventDefault();
        return;
      }
      if (e.key === "Tab" && !e.shiftKey) {
        const chosen = suggestions[pick];
        if (chosen) set(`/${chosen.name} `);
        e.preventDefault();
        return;
      }
      if (e.key === "Escape") {
        set("");
        e.preventDefault();
        return;
      }
    }
    if (e.key === "Tab" && e.shiftKey) {
      onCycleMode();
      e.preventDefault();
      return;
    }
    if (e.key === "Escape" && editing) {
      set("");
      onCancelEdit();
      e.preventDefault();
      return;
    }
    if (e.key === "Escape" && busy) {
      onStop();
      e.preventDefault();
      return;
    }
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      submit();
      e.preventDefault();
    }
  };

  return (
    <div
      className={`th-composer${dropping ? " is-dropping" : ""}`}
      data-tour="composer"
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("Files") && !editing) {
          e.preventDefault();
          setDropping(true);
        }
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropping(false);
      }}
      onDrop={(e) => {
        if (!e.dataTransfer.types.includes("Files") || editing) return;
        e.preventDefault();
        setDropping(false);
        attach(e.dataTransfer.files);
      }}
    >
      {files.length > 0 ? (
        <div className="th-attachments">
          {files.map((f) => (
            <span key={f.key} className={`th-attachment is-${f.state}`} title={f.note}>
              {f.preview ? <img className="th-attachment-preview" src={f.preview} alt="" /> : null}
              {f.state === "uploading" ? "Uploading " : f.state === "failed" ? "✗ " : ""}
              {f.name}
              {f.note ? `: ${f.note}` : ""}
              <button
                type="button"
                aria-label={`Remove ${f.name}`}
                onClick={() => {
                  drop([f]);
                  setFiles((fs) => fs.filter((x) => x.key !== f.key));
                }}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      ) : null}
      {editing ? (
        <div className="th-editing">
          <span>Editing your last message. Changes Thrall already made to files stay.</span>
          <button
            type="button"
            className="th-copy"
            onClick={() => {
              set("");
              onCancelEdit();
            }}
          >
            Cancel (Esc)
          </button>
        </div>
      ) : null}
      {suggestions.length > 0 ? (
        <div className="th-suggest" role="listbox" aria-label="Commands">
          {suggestions.map((c, i) => (
            <button
              type="button"
              key={c.name}
              role="option"
              aria-selected={i === pick}
              className={i === pick ? "is-on" : ""}
              onMouseDown={(e) => {
                e.preventDefault();
                set(`/${c.name} `);
                box.current?.focus();
              }}
            >
              <b>/{c.name}</b>
              <span>{c.summary}</span>
            </button>
          ))}
        </div>
      ) : null}
      <textarea
        id="composer"
        ref={box}
        rows={1}
        value={text}
        placeholder={COMPOSER_PLACEHOLDER}
        aria-label="Message Thrall"
        disabled={disabled}
        onChange={(e) => set(e.target.value)}
        onKeyDown={onKeyDown}
        onPaste={(e) => {
          // A pasted screenshot or copied file is attached; pasted text goes in as usual.
          const pasted = Array.from(e.clipboardData.files);
          if (pasted.length > 0 && !editing) {
            e.preventDefault();
            attach(pasted);
          }
        }}
      />
      <input
        ref={picker}
        id="attach-files"
        type="file"
        multiple
        hidden
        onChange={(e) => {
          attach(e.target.files);
          e.target.value = "";
        }}
      />
      <div className="th-bar">
        {!editing && !busy ? (
          <button type="button" className="th-btn th-btn-quiet" onClick={() => picker.current?.click()}>
            Attach
          </button>
        ) : null}
        {onDictate && !editing ? (
          <button
            type="button"
            className={`th-btn th-btn-quiet th-mic is-${mic}`}
            onClick={() => void toggleMic()}
            disabled={mic === "transcribing" || disabled}
            aria-pressed={mic === "recording"}
            title={mic === "recording" ? "Stop and turn it into text" : "Speak your message"}
          >
            {mic === "recording" ? "● Stop" : mic === "transcribing" ? "Writing it out…" : "Speak"}
          </button>
        ) : null}
        <span className="th-hint">
          {micNote ? <span className="th-mic-note">{micNote} </span> : null}
          {busy ? "Enter adds to the current turn · Esc stops" : "Enter sends · Shift+Enter new line · Shift+Tab mode"}
        </span>
        {busy ? (
          <button type="button" className="th-btn" onClick={onStop}>
            Stop
          </button>
        ) : (
          <button
            type="button"
            className="th-btn th-btn-primary"
            onClick={submit}
            disabled={disabled || uploading || (!text.trim() && !files.some((f) => f.state === "ready"))}
          >
            Send
          </button>
        )}
      </div>
    </div>
  );
});
