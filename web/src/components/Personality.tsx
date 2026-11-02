// Settings → Personality: how Thrall talks to you. Everyone starts on the standard one Thrall ships
// with; pick another, or write your own. It's yours alone (each person chooses for themselves) and
// changes only Thrall's tone, never its rules or what it asks permission for.

import { useCallback, useEffect, useState, type ReactNode } from "react";

import type { Engine, ReplyOf } from "../engine";
import { PERSONALITY_CUSTOM, PERSONALITY_MAX_CHARS } from "../constants";

type Shown = ReplyOf<"personality.shown">;
type Status = (s: { text: string; warn?: boolean }) => void;

function said(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function Personality({
  engine,
  onStatus,
  Group,
}: {
  engine: Engine | null;
  onStatus: Status;
  Group: (p: { title: string; note?: string; children: ReactNode }) => ReactNode;
}) {
  const [shown, setShown] = useState<Shown | null>(null);
  const [preset, setPreset] = useState("");
  const [custom, setCustom] = useState("");

  const take = useCallback((r: Shown) => {
    setShown(r);
    setPreset(r.preset);
    setCustom(r.custom);
  }, []);

  useEffect(() => {
    engine
      ?.request<"personality.shown">({ type: "personality.get" })
      .then(take)
      .catch((e: unknown) => onStatus({ text: said(e), warn: true }));
  }, [engine, onStatus, take]);

  const save = () =>
    engine
      ?.request<"personality.shown">({ type: "personality.set", preset, custom })
      .then((r) => {
        take(r);
        onStatus({ text: "Saved. Thrall talks to you this way from your next message." });
      })
      .catch((e: unknown) => onStatus({ text: said(e), warn: true }));

  if (!shown) return null;
  const changed = preset !== shown.preset || custom !== shown.custom;
  return (
    <Group
      title="How Thrall talks to you"
      note="Only yours: everyone who uses this Thrall chooses their own. It changes Thrall's tone, never its rules or what it asks you first."
    >
      <div className="th-personas" role="radiogroup" aria-label="Personality">
        {[...shown.presets, { id: PERSONALITY_CUSTOM, name: "Your own", description: "Describe it in your own words." }].map(
          (p) => (
            <label key={p.id} className={`th-persona${preset === p.id ? " is-on" : ""}`}>
              <input type="radio" name="personality" checked={preset === p.id} onChange={() => setPreset(p.id)} />
              <span className="th-persona-name">{p.name}</span>
              <span className="th-persona-about">{p.description}</span>
            </label>
          ),
        )}
      </div>
      {preset === PERSONALITY_CUSTOM ? (
        <textarea
          className="th-input th-persona-own"
          aria-label="Your own personality for Thrall"
          placeholder="For example: Talk to me like a patient coach. Keep it short, and celebrate small wins."
          maxLength={PERSONALITY_MAX_CHARS}
          rows={4}
          value={custom}
          onChange={(e) => setCustom(e.target.value)}
        />
      ) : null}
      <div className="th-set-add">
        <button type="button" className="th-btn th-btn-primary" disabled={!changed} onClick={() => void save()}>
          Save
        </button>
      </div>
    </Group>
  );
}
