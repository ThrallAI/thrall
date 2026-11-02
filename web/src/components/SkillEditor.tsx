// Settings → Learning: one of your skills opened to read or change. "Make it safe" asks the model for
// a version that asks you before anything risky; it's shown beside yours and saved only if you
// choose it, so your own version stays until then.

import { useEffect, useState } from "react";

import type { Engine, ReplyOf } from "../engine";
import { RATING_LABELS, SKILL_DESCRIPTION_MAX_CHARS } from "../constants";
import type { SkillDetail } from "../protocol";

type Safer = ReplyOf<"skill.safer">;
type Status = (s: { text: string; warn?: boolean }) => void;

function said(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function Rating({ rating }: { rating?: string | null }) {
  if (!rating) return null;
  return <span className={`th-badge is-rating-${rating}`}>{RATING_LABELS[rating] ?? rating}</span>;
}

export function SkillEditor({
  engine,
  name,
  onStatus,
  onClose,
}: {
  engine: Engine | null;
  name: string;
  onStatus: Status;
  /** Closed; `saved` when the skill changed (the list needs reading again). */
  onClose: (saved: boolean) => void;
}) {
  const [skill, setSkill] = useState<SkillDetail | null>(null);
  const [description, setDescription] = useState("");
  const [instructions, setInstructions] = useState("");
  const [safer, setSafer] = useState<Safer | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    engine
      ?.request<"skill.shown">({ type: "skill.get", name })
      .then((r) => {
        setSkill(r.skill);
        setDescription(r.skill.description);
        setInstructions(r.skill.instructions);
      })
      .catch((e: unknown) => {
        onStatus({ text: said(e), warn: true });
        onClose(false);
      });
  }, [engine, name, onStatus, onClose]);

  const save = (d: string, i: string, done: string) => {
    setBusy(true);
    engine
      ?.request<"skill.shown">({ type: "skill.edit", name, description: d, instructions: i })
      .then((r) => {
        const rating = r.skill.rating ? ` It's now rated ${RATING_LABELS[r.skill.rating] ?? r.skill.rating}.` : "";
        onStatus({ text: `${done}${rating}` });
        onClose(true);
      })
      .catch((e: unknown) => onStatus({ text: said(e), warn: true }))
      .finally(() => setBusy(false));
  };

  const makeSafe = () => {
    setBusy(true);
    onStatus({ text: `Asking for a safer version of /${name}…` });
    engine
      ?.request<"skill.safer">({ type: "skill.make_safe", name })
      .then((r) => {
        setSafer(r);
        onStatus({ text: "Here's a safer version beside yours. Nothing changes until you choose it." });
      })
      .catch((e: unknown) => onStatus({ text: said(e), warn: true }))
      .finally(() => setBusy(false));
  };

  if (!skill) return null;
  const changed = description !== skill.description || instructions !== skill.instructions;
  const ok = description.trim() !== "" && instructions.trim() !== "";

  if (safer) {
    return (
      <div className="th-skill-edit">
        <h5>A safer /{name}</h5>
        <div className="th-skill-compare">
          <div>
            <span>
              Yours <Rating rating={skill.rating} />
            </span>
            <small>{skill.description}</small>
            <pre>{skill.instructions}</pre>
          </div>
          <div>
            <span>
              Safer <Rating rating={safer.rating} />
            </span>
            <small>{safer.description}</small>
            <pre>{safer.instructions}</pre>
            {safer.why.length > 0 ? <p className="th-skill-why">Still rated this because {safer.why.join(", ")}.</p> : null}
          </div>
        </div>
        <div className="th-set-add">
          <button
            type="button"
            className="th-btn th-btn-primary"
            disabled={busy}
            onClick={() => save(safer.description, safer.instructions, `/${name} now uses the safer version.`)}
          >
            Use the safer one
          </button>
          <button type="button" className="th-btn th-btn-quiet" disabled={busy} onClick={() => setSafer(null)}>
            Keep mine
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="th-skill-edit">
      <h5>
        /{name} <Rating rating={skill.rating} />
      </h5>
      <input
        className="th-input"
        aria-label="When to use it"
        placeholder="One sentence: when to use it"
        maxLength={SKILL_DESCRIPTION_MAX_CHARS}
        value={description}
        onChange={(e) => setDescription(e.target.value)}
      />
      <textarea
        className="th-input th-skill-text"
        aria-label="Instructions"
        spellCheck={false}
        value={instructions}
        onChange={(e) => setInstructions(e.target.value)}
      />
      <div className="th-set-add">
        <button
          type="button"
          className="th-btn th-btn-primary"
          disabled={busy || !changed || !ok}
          onClick={() => save(description, instructions, `Saved /${name}.`)}
        >
          Save
        </button>
        {skill.rating && skill.rating !== "low" ? (
          <button type="button" className="th-btn" disabled={busy || changed} title={changed ? "Save or undo your changes first" : undefined} onClick={makeSafe}>
            Make it safe
          </button>
        ) : null}
        <button type="button" className="th-btn th-btn-quiet" disabled={busy} onClick={() => onClose(false)}>
          Close
        </button>
      </div>
    </div>
  );
}
