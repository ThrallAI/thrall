import { useEffect, useState } from "react";
import type { Request } from "../engine";

/** Where a new folder can go, as `folder.places` answers. */
interface Places {
  default?: string | null;
  new_in?: string | null;
  recent: string[];
}

/**
 * Make a folder for a new session, as the phone's "＋ New folder" does: a name, and where it goes
 * (the `folders.new_in` setting first, then Thrall's own folder, then folders sessions use).
 */
export function NewFolder({
  request,
  onCreated,
  onCancel,
}: {
  request: (r: Request) => Promise<unknown>;
  onCreated: (path: string) => void;
  onCancel: () => void;
}) {
  const [parents, setParents] = useState<string[]>([]);
  const [parent, setParent] = useState("");
  const [name, setName] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    request({ type: "folder.places" })
      .then((reply) => {
        const p = reply as Places;
        const list = [p.new_in, p.default, ...p.recent].filter((d): d is string => !!d);
        const unique = [...new Set(list)];
        setParents(unique);
        setParent(unique[0] ?? "");
      })
      .catch((err: unknown) => setProblem(err instanceof Error ? err.message : String(err)));
  }, [request]);

  const create = async () => {
    if (!name.trim() || !parent || busy) return;
    setBusy(true);
    setProblem(null);
    try {
      const reply = (await request({ type: "folder.create", parent, name: name.trim() })) as { path: string };
      onCreated(reply.path);
    } catch (err) {
      setProblem(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="th-new-folder">
      <input
        autoFocus
        value={name}
        placeholder="Folder name"
        aria-label="New folder's name"
        onChange={(ev) => setName(ev.target.value)}
        onKeyDown={(ev) => {
          if (ev.key === "Enter") void create();
          if (ev.key === "Escape") onCancel();
        }}
      />
      <label>
        <span>In</span>
        <select value={parent} onChange={(ev) => setParent(ev.target.value)} aria-label="Where to make it">
          {parents.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
      </label>
      <div className="th-new-folder-actions">
        <button type="button" className="th-btn th-btn-quiet" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="th-btn" disabled={!name.trim() || !parent || busy} onClick={() => void create()}>
          Make folder
        </button>
      </div>
      {problem ? <p className="th-new-folder-problem">{problem}</p> : null}
    </div>
  );
}
