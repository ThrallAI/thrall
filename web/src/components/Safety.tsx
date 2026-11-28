// Settings → Safety (the owner's alone): Unleashed. On, the owner's chats stop asking before
// acting, high-risk actions too, except wiping or formatting a disk. Turning it on means typing
// UNLEASH; turning it off (Chain him) is one press, here or in any chat.

import { useEffect, useState, type ReactNode } from "react";

import type { Engine } from "../engine";
import { UNLEASH_WORD } from "../constants";

type Status = (s: { text: string; warn?: boolean }) => void;

function why(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function Safety({
  engine,
  on,
  onStatus,
  Row,
  Group,
}: {
  engine: Engine | null;
  /** Unleashed is on now. */
  on: boolean;
  onStatus: Status;
  Row: (p: { label: string; hint?: string; children: ReactNode }) => ReactNode;
  Group: (p: { title: string; note?: string; children: ReactNode }) => ReactNode;
}) {
  const [unleashed, setUnleashed] = useState(on);
  // Switched elsewhere (Chain him in a chat, /chain, the phone): show it here too.
  useEffect(() => setUnleashed(on), [on]);
  const [typing, setTyping] = useState<string | null>(null);

  const set = (next: boolean, confirm: string) => {
    engine
      ?.request<"safety.shown">({ type: "safety.set", unleashed: next, confirm })
      .then((r) => {
        setUnleashed(r.unleashed);
        setTyping(null);
        onStatus({
          text: r.unleashed
            ? "Unleashed: your chats don't ask before acting now, except before wiping a disk. Chain him in any chat turns it off."
            : "Chained: Thrall asks again before risky actions, as each chat's mode says.",
          warn: r.unleashed,
        });
      })
      .catch((e: unknown) => onStatus({ text: why(e), warn: true }));
  };

  return (
    <Group
      title="Safety"
      note="Unleashed is for when you want Thrall to just get on with it. It's your computer and your risk: one misread instruction can delete, stop or change things with nobody asked first."
    >
      <Row
        label="Unleashed"
        hint="Every one of your chats stops asking, high-risk actions too (restarts, deleting outside the folder, stopping programs, force pushes, unofficial installs). Wiping or formatting a disk still asks. Registry and system changes still need a backup and a way to undo. Other people's chats keep asking"
      >
        {unleashed ? (
          <>
            <span className="th-badge is-warn">On</span>
            <button type="button" className="th-btn th-btn-primary" onClick={() => set(false, "")}>
              Chain him
            </button>
          </>
        ) : typing === null ? (
          <>
            <span className="th-badge">Off</span>
            <button type="button" className="th-btn th-btn-danger" onClick={() => setTyping("")}>
              Unleash…
            </button>
          </>
        ) : (
          <>
            <input
              className="th-input"
              value={typing}
              onChange={(e) => setTyping(e.target.value)}
              placeholder={`Type ${UNLEASH_WORD}`}
              aria-label={`Type ${UNLEASH_WORD} to confirm`}
              autoFocus
            />
            <button
              type="button"
              className="th-btn th-btn-danger"
              disabled={typing.trim() !== UNLEASH_WORD}
              onClick={() => set(true, typing.trim())}
            >
              Unleash
            </button>
            <button type="button" className="th-btn th-btn-quiet" onClick={() => setTyping(null)}>
              Cancel
            </button>
          </>
        )}
      </Row>
    </Group>
  );
}
