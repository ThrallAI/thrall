// Settings → Providers: who does each of Thrall's jobs (a supplier and its model), and the
// suppliers with whether Thrall has a key for each and a Test. Keys are never typed here: they're
// kept in the keychain with `thrall key set`, and the Test runs in the engine with the key it has.

import { useEffect, useState, type ReactNode } from "react";

import type { Engine, ReplyOf } from "../engine";
import { JOBS, SUPPLIERS } from "../constants";
import { bareModel, supplier, supplierOf } from "../suppliers";

type Shown = ReplyOf<"settings.shown">;
type Tested = ReplyOf<"provider.tested">;
type Save = (key: string, value: boolean | number | string) => void;
type Status = (s: { text: string; warn?: boolean }) => void;

/** The setting `key` (`provider.model`) in the config. */
function setting(config: Record<string, unknown>, key: string): string {
  const [section = "", name = ""] = key.split(".");
  const table = config[section];
  const v = table && typeof table === "object" ? (table as Record<string, unknown>)[name] : undefined;
  return typeof v === "string" ? v : "";
}

/** One job: pick its supplier, then the model (the supplier's own name for it). */
function JobRow({
  label,
  k,
  value,
  choices,
  models,
  onSave,
  onPick,
  Row,
}: {
  label: string;
  k: string;
  value: string;
  choices: readonly string[];
  /** Models the picked supplier listed when tested, by its own names. */
  models: (id: string) => string[];
  onSave: Save;
  /** A supplier was picked: test it, so its models can be listed. */
  onPick: (id: string) => void;
  Row: (p: { label: string; hint?: string; children: ReactNode }) => ReactNode;
}) {
  const current = supplierOf(value);
  const [picked, setPicked] = useState(current.id);
  const [draft, setDraft] = useState(bareModel(value));
  useEffect(() => {
    setPicked(supplierOf(value).id);
    setDraft(bareModel(value));
  }, [value]);
  const s = supplier(picked) ?? current;
  const listId = `th-models-${k.replace(".", "-")}`;
  const commit = () => {
    const model = draft.trim();
    if (!model) {
      // Nothing to save until there's a model: go back to what's in use.
      if (picked === current.id) setDraft(bareModel(value));
      return;
    }
    if (s.prefix + model !== value) onSave(k, s.prefix + model);
  };
  return (
    <Row label={label} hint={picked !== current.id && !draft.trim() ? `Now pick a ${s.name} model` : undefined}>
      <select
        className="th-input"
        aria-label={`${label}: supplier`}
        value={picked}
        onChange={(e) => {
          setPicked(e.target.value);
          setDraft("");
          onPick(e.target.value);
        }}
      >
        {choices.map((id) => (
          <option key={id} value={id}>
            {supplier(id)?.name ?? id}
          </option>
        ))}
      </select>
      <input
        className="th-input th-set-mono"
        aria-label={`${label}: model`}
        placeholder="model"
        list={listId}
        value={draft}
        spellCheck={false}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          if (e.key === "Escape") {
            e.stopPropagation();
            setPicked(current.id);
            setDraft(bareModel(value));
          }
        }}
      />
      <datalist id={listId}>
        {models(picked).map((m) => (
          <option key={m} value={m} />
        ))}
      </datalist>
    </Row>
  );
}

export function Providers({
  engine,
  shown,
  onSave,
  onStatus,
  Row,
  Group,
  Field,
}: {
  engine: Engine | null;
  shown: Shown | null;
  onSave: Save;
  onStatus: Status;
  Row: (p: { label: string; hint?: string; children: ReactNode }) => ReactNode;
  Group: (p: { title: string; note?: string; children: ReactNode }) => ReactNode;
  Field: (p: { k: string; v: unknown; label: string; onSave: Save; mono?: boolean; allowEmpty?: boolean }) => ReactNode;
}) {
  const config = (shown?.config ?? {}) as Record<string, unknown>;
  const provider = (config.provider ?? {}) as Record<string, unknown>;
  const [tests, setTests] = useState<Record<string, Tested | "testing">>({});
  const keySet = (name?: string) => (name ? (shown?.keys.find((k) => k.name === name)?.set ?? false) : true);

  const test = (id: string) => {
    if (!engine) return;
    setTests((t) => ({ ...t, [id]: "testing" }));
    engine
      .request<"provider.tested">({ type: "provider.test", supplier: id })
      .then((r) => setTests((t) => ({ ...t, [id]: r })))
      .catch((e: unknown) => {
        setTests((t) => {
          const { [id]: _gone, ...rest } = t;
          return rest;
        });
        onStatus({ text: e instanceof Error ? e.message : String(e), warn: true });
      });
  };
  // Picking a supplier for a job lists its models, once.
  const pick = (id: string) => {
    if (!tests[id] && id !== "openrouter") test(id);
  };
  const models = (id: string) => {
    const t = tests[id];
    const prefix = supplier(id)?.prefix ?? "";
    return t && t !== "testing" ? (t.models ?? []).map((m) => m.slice(prefix.length)) : [];
  };

  return (
    <>
      <Group
        title="Who does each job"
        note="Pick a supplier, then its model. OpenRouter reaches many companies with one key; the others need their own key or sign-in (below)."
      >
        {JOBS.map((job) => (
          <JobRow
            key={job.key}
            label={job.label}
            k={job.key}
            value={setting(config, job.key)}
            choices={job.suppliers}
            models={models}
            onSave={onSave}
            onPick={pick}
            Row={Row}
          />
        ))}
        <Row label="Makes videos" hint="Video models are picked per video (make_video)">
          <span className="th-set-mono">fal.ai</span>
        </Row>
        <Row label="Voice" hint="For ElevenLabs, a voice id from your ElevenLabs voice library">
          <Field k="provider.speech_voice" v={provider.speech_voice} label="Voice" onSave={onSave} />
        </Row>
        <Row label="Local model server" hint="Ollama (http://localhost:11434/v1) or LM Studio (http://localhost:1234/v1)">
          <Field k="provider.local_base_url" v={provider.local_base_url} label="Local model server" onSave={onSave} mono allowEmpty />
        </Row>
      </Group>
      <Group
        title="Suppliers"
        note="Keys stay in this computer's keychain and never pass through this screen. Add or replace one in a terminal with the command shown, then restart Thrall. Test checks it with the key Thrall has."
      >
        {SUPPLIERS.map((s) => {
          const t = tests[s.id];
          const hasKey = s.id === "chatgpt" ? keySet("chatgpt") : keySet(s.key);
          return (
            <Row key={s.id} label={s.name} hint={t && t !== "testing" ? t.message : s.about}>
              {s.key || s.id === "chatgpt" ? (
                <span className={`th-badge${hasKey ? " is-ok" : ""}`}>
                  {s.id === "chatgpt" ? (hasKey ? "Signed in" : "Not signed in") : hasKey ? "Key saved" : "No key"}
                </span>
              ) : null}
              {t && t !== "testing" ? (
                <span className={`th-badge${t.ok ? " is-ok" : " is-warn"}`}>{t.ok ? "Works" : "Not working"}</span>
              ) : null}
              {s.key && !hasKey ? <code className="th-set-mono">thrall key set {s.key}</code> : null}
              <button type="button" className="th-btn" disabled={t === "testing"} onClick={() => test(s.id)}>
                {t === "testing" ? "Testing…" : "Test"}
              </button>
            </Row>
          );
        })}
      </Group>
    </>
  );
}
