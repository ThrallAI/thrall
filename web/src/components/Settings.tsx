// The settings screen. Appearance is kept in this browser; the engine's settings are read from and
// saved to config.toml (settings.get / settings.set); features still being built are marked Planned.

import { useEffect, useRef, useState, type ReactNode } from "react";

import type { Engine, ReplyOf } from "../engine";
import {
  FRESH_STARTS,
  HUMANISE_LEVELS,
  LOG_LEVELS,
  QUESTION_MODE_LABELS,
  QUESTION_MODES,
  RATING_LABELS,
  SETTINGS_SECTIONS,
  SIGN_IN_POLL_MS,
  SIGN_IN_WATCH_MS,
  RELEASES_PAGE,
  STARTUP_LABEL,
  type SettingsSection,
} from "../constants";
import { setTourOn, tourOn } from "../onboarding";
import { newerRelease } from "../updates";
import type { FactInfo, LessonInfo } from "../protocol";
import type { Session } from "../types";
import { THEME_LABELS, type Theme } from "../theme";
import { People } from "./People";
import { Personality } from "./Personality";
import { Phones } from "./Phones";
import { Providers } from "./Providers";

type Shown = ReplyOf<"settings.shown">;
type Obj = Record<string, unknown>;

function obj(v: unknown): Obj {
  return v && typeof v === "object" && !Array.isArray(v) ? (v as Obj) : {};
}

function text(v: unknown): string {
  if (v === undefined || v === null || v === "") return "—";
  if (typeof v === "boolean") return v ? "On" : "Off";
  if (typeof v === "number") return v.toLocaleString();
  return String(v);
}

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="th-set-row">
      <div className="th-set-label">
        <span>{label}</span>
        {hint ? <small>{hint}</small> : null}
      </div>
      <div className="th-set-value">{children}</div>
    </div>
  );
}

/** A value shown as it is (not a setting you change here). */
function Value({ v, mono }: { v: unknown; mono?: boolean }) {
  return <span className={mono ? "th-set-mono" : undefined}>{text(v)}</span>;
}

type Save = (key: string, value: boolean | number | string) => void;

/** A yes/no setting, saved when flipped. */
function Toggle({ k, v, label, onSave }: { k: string; v: unknown; label: string; onSave: Save }) {
  const on = v === true;
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      className={`th-switch${on ? " is-on" : ""}`}
      onClick={() => onSave(k, !on)}
    >
      <span />
    </button>
  );
}

/** A text or number setting: saved on Enter or when you leave the box; Esc puts it back. */
function Field({
  k,
  v,
  label,
  onSave,
  number,
  mono,
  list,
  allowEmpty,
}: {
  k: string;
  v: unknown;
  label: string;
  onSave: Save;
  number?: boolean;
  mono?: boolean;
  /** Empty is a real value here (saved as ""), not a mistake to undo. */
  allowEmpty?: boolean;
  /** Id of a <datalist> with suggestions. */
  list?: string;
}) {
  const current = v === undefined || v === null ? "" : String(v);
  const [draft, setDraft] = useState(current);
  useEffect(() => setDraft(current), [current]);
  const commit = () => {
    const d = draft.trim();
    if (d === current) return;
    if (number) {
      const n = Number(d);
      if (!d || !Number.isInteger(n) || n < 0) {
        setDraft(current);
        return;
      }
      onSave(k, n);
    } else if (d || allowEmpty) {
      onSave(k, d);
    } else {
      setDraft(current);
    }
  };
  return (
    <input
      className={`th-input${mono ? " th-set-mono" : ""}${number ? " is-number" : ""}`}
      aria-label={label}
      inputMode={number ? "numeric" : undefined}
      list={list}
      value={draft}
      spellCheck={false}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        if (e.key === "Escape") {
          e.stopPropagation();
          setDraft(current);
        }
      }}
    />
  );
}

/** One of a few choices, saved when picked. */
function Choice({
  k,
  v,
  label,
  options,
  labels,
  onSave,
}: {
  k: string;
  v: unknown;
  label: string;
  options: readonly string[];
  /** What each option is called on screen (else its config value). */
  labels?: Readonly<Record<string, string>>;
  onSave: Save;
}) {
  return (
    <select className="th-input" aria-label={label} value={String(v ?? "")} onChange={(e) => onSave(k, e.target.value)}>
      {options.map((o) => (
        <option key={o} value={o}>
          {labels?.[o] ?? o}
        </option>
      ))}
    </select>
  );
}

/** Most check-ins a day: the active hours' length over the interval. */
function checkInsPerDay(hours: string, every: number): number {
  const m = /^\s*(\d{1,2}):(\d{2})\s*-\s*(\d{1,2}):(\d{2})\s*$/.exec(hours);
  let span = 24 * 60;
  if (m) {
    const from = Number(m[1]) * 60 + Number(m[2]);
    const to = Number(m[3]) * 60 + Number(m[4]);
    span = to > from ? to - from : 24 * 60 - from + to;
  }
  return Math.max(1, Math.floor(span / Math.max(every, 1)));
}

function Planned({ label, hint, phase }: { label: string; hint: string; phase?: string }) {
  return (
    <Row label={label} hint={hint}>
      <span className="th-badge">{phase ?? "Planned"}</span>
    </Row>
  );
}

function Group({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return (
    <section className="th-set-group">
      <h4>{title}</h4>
      {note ? <p className="th-set-note">{note}</p> : null}
      <div className="th-set-rows">{children}</div>
    </section>
  );
}

/** A new model profile: a short name, the model, and what it's for. */
function ProfileAdder({ onAdd }: { onAdd: (name: string, model: string, about: string) => void }) {
  const [name, setName] = useState("");
  const [model, setModel] = useState("");
  const [about, setAbout] = useState("");
  const clean = name.trim().toLowerCase();
  const ok = /^[a-z0-9_-]+$/.test(clean) && model.trim() !== "";
  return (
    <form
      className="th-set-add"
      onSubmit={(e) => {
        e.preventDefault();
        if (!ok) return;
        onAdd(clean, model.trim(), about.trim());
        setName("");
        setModel("");
        setAbout("");
      }}
    >
      <input className="th-input" aria-label="Profile name" placeholder="name, e.g. fast" value={name} onChange={(e) => setName(e.target.value)} />
      <input className="th-input th-set-mono" aria-label="Profile model" placeholder="model id" value={model} onChange={(e) => setModel(e.target.value)} />
      <input className="th-input" aria-label="Profile description" placeholder="what it's for (optional)" value={about} onChange={(e) => setAbout(e.target.value)} />
      <button type="submit" className="th-btn" disabled={!ok}>
        Add profile
      </button>
    </form>
  );
}

export function Settings({
  engine,
  theme,
  onTheme,
  onClose,
  onHomeCleared,
  version,
  onTour,
}: {
  engine: Engine | null;
  theme: Theme;
  onTheme: (t: Theme) => void;
  onClose: () => void;
  /** Home was cleared; this is its fresh page. */
  onHomeCleared: (page: Session) => void;
  /** The engine's version, for Help's "Check for updates". */
  version: string;
  /** Start the one-minute tour (Settings closes first). */
  onTour: () => void;
}) {
  const [tour, setTour] = useState(tourOn);
  const [update, setUpdate] = useState<string | null>(null);
  const checkUpdate = () => {
    setUpdate("Checking…");
    newerRelease(version)
      .then((newer) => setUpdate(newer ? `Version ${newer} is out. Download it from:` : "You have the newest version."))
      .catch(() => setUpdate("Couldn't reach GitHub to check. Try again later."));
  };
  const [section, setSection] = useState<SettingsSection>("general");
  const [shown, setShown] = useState<Shown | null>(null);
  const [failed, setFailed] = useState("");
  const [status, setStatus] = useState<{ text: string; warn?: boolean } | null>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  // The app re-renders often (timers); keep the latest close without re-running the effect.
  const close = useRef(onClose);
  close.current = onClose;

  useEffect(() => {
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    engine
      ?.request<"settings.shown">({ type: "settings.get" })
      .then((r) => {
        setShown(r);
        if (r.pending) setStatus({ text: "Some saved settings are used once Thrall finishes what it's doing." });
      })
      .catch((e: unknown) => setFailed(e instanceof Error ? e.message : String(e)));
  }, [engine]);

  const config = obj(shown?.config);
  const provider = obj(config.provider);
  const agent = obj(config.agent);
  const memory = obj(config.memory);
  const writing = obj(config.writing);
  const home = obj(config.home);
  const heartbeat = obj(config.heartbeat);
  const notifications = obj(config.notifications);
  const desktop = obj(config.desktop);
  const folders = obj(config.folders);
  const logging = obj(config.logging);
  const servers = obj(obj(config.mcp).servers);
  const hooks = Array.isArray(config.hooks) ? (config.hooks as Obj[]) : [];

  // Save one setting to config.toml; the engine answers with the settings as they now are.
  const save: Save = (key, value) => {
    if (!engine) return;
    setStatus({ text: "Saving…" });
    engine
      .request<"settings.shown">({ type: "settings.set", key, value })
      .then((r) => {
        setShown(r);
        setStatus({
          text: r.pending ? "Saved. It's used once Thrall finishes what it's doing." : "Saved and in use.",
        });
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };
  const saved = "Saved to config.toml as you change it.";
  const signedIn = shown?.keys.find((k) => k.name === "chatgpt")?.set ?? false;
  const [signInUrl, setSignInUrl] = useState("");
  const signInTimer = useRef<number | null>(null);
  useEffect(() => () => {
    if (signInTimer.current !== null) window.clearInterval(signInTimer.current);
  }, []);

  // Run an engine slash command (sign in, switch model) from the panel.
  const command = async (name: string, args: string) => {
    if (!engine) throw new Error("Not connected to Thrall.");
    return (await engine.request<"command.result">({ type: "command.run", name, args })).markdown;
  };

  const signIn = () => {
    setStatus({ text: "Opening ChatGPT's sign-in page…" });
    command("login", "chatgpt")
      .then((markdown) => {
        const url = /\((https:\/\/auth\.openai\.com[^)\s]+)\)/.exec(markdown)?.[1] ?? "";
        setSignInUrl(url);
        setStatus({ text: "Sign in on the page that opened in your browser. This updates by itself when you're done." });
        // Watch for the sign-in to land (the engine reloads with it).
        if (signInTimer.current !== null) window.clearInterval(signInTimer.current);
        const started = Date.now();
        signInTimer.current = window.setInterval(() => {
          engine
            ?.request<"settings.shown">({ type: "settings.get" })
            .then((r) => {
              setShown(r);
              const done = r.keys.find((k) => k.name === "chatgpt")?.set;
              if (done || Date.now() - started > SIGN_IN_WATCH_MS) {
                if (signInTimer.current !== null) window.clearInterval(signInTimer.current);
                signInTimer.current = null;
                setSignInUrl("");
                setStatus(
                  done
                    ? { text: "Signed in to ChatGPT. Pick one of your plan's models in the Model box." }
                    : { text: "The sign-in wasn't finished. Try again.", warn: true },
                );
              }
            })
            .catch(() => undefined);
        }, SIGN_IN_POLL_MS);
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  const signOut = () => {
    command("logout", "chatgpt")
      .then(() => engine?.request<"settings.shown">({ type: "settings.get" }))
      .then((r) => {
        if (r) setShown(r);
        setStatus({ text: "Signed out of ChatGPT." });
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  // Signing in to an MCP server on the web: the engine opens its page; watch for the sign-in to land.
  const mcpSignedIn = shown?.mcp_signed_in ?? [];
  const [mcpSigningIn, setMcpSigningIn] = useState<{ server: string; url: string } | null>(null);
  const mcpTimer = useRef<number | null>(null);
  useEffect(() => () => {
    if (mcpTimer.current !== null) window.clearInterval(mcpTimer.current);
  }, []);
  const mcpSignIn = (server: string) => {
    setStatus({ text: `Opening ${server}'s sign-in page…` });
    command("mcp", `login ${server}`)
      .then((markdown) => {
        const url = /\]\((https?:\/\/[^)\s]+)\)\s*$/.exec(markdown)?.[1] ?? "";
        setMcpSigningIn({ server, url });
        setStatus({ text: `Sign in to ${server} on the page that opened in your browser. This updates by itself when you're done.` });
        if (mcpTimer.current !== null) window.clearInterval(mcpTimer.current);
        const started = Date.now();
        mcpTimer.current = window.setInterval(() => {
          engine
            ?.request<"settings.shown">({ type: "settings.get" })
            .then((r) => {
              setShown(r);
              const done = (r.mcp_signed_in ?? []).includes(server);
              if (done || Date.now() - started > SIGN_IN_WATCH_MS) {
                if (mcpTimer.current !== null) window.clearInterval(mcpTimer.current);
                mcpTimer.current = null;
                setMcpSigningIn(null);
                setStatus(
                  done
                    ? { text: `Signed in to ${server}. Thrall can use its tools now.` }
                    : { text: `Signing in to ${server} wasn't finished. Try again.`, warn: true },
                );
              }
            })
            .catch(() => undefined);
        }, SIGN_IN_POLL_MS);
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };
  const mcpSignOut = (server: string) => {
    command("mcp", `logout ${server}`)
      .then((markdown) => {
        setStatus({ text: markdown });
        return engine?.request<"settings.shown">({ type: "settings.get" });
      })
      .then((r) => {
        if (r) setShown(r);
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  const profiles = obj(config.profiles);
  const useProfile = (name: string) => {
    command("model", `@${name}`)
      .then(() => engine?.request<"settings.shown">({ type: "settings.get" }))
      .then((r) => {
        if (r) setShown(r);
        setStatus({ text: `Switched to @${name} until Thrall restarts. Make it the default with its model in the Model box.` });
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };
  const addProfile = (name: string, model: string, about: string) => {
    if (!engine) return;
    engine
      .request<"settings.shown">({ type: "settings.set", key: `profiles.${name}.model`, value: model })
      .then((r) =>
        about ? engine.request<"settings.shown">({ type: "settings.set", key: `profiles.${name}.description`, value: about }) : r,
      )
      .then((r) => {
        setShown(r);
        setStatus({ text: `Added @${name}. Switch to it with /model @${name} or Use.` });
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  // Facts load when the Memory section opens.
  const [facts, setFacts] = useState<FactInfo[] | null>(null);
  useEffect(() => {
    if (section !== "memory" || !engine) return;
    engine
      .request<"memory.listed">({ type: "memory.list" })
      .then((r) => setFacts(r.facts))
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  }, [section, engine]);
  const forgetFact = (fact: string) => {
    engine
      ?.request({ type: "memory.forget", fact })
      .then(() => {
        setFacts((list) => (list ?? []).filter((f) => f.fact !== fact));
        setStatus({ text: "Forgotten: Thrall won't recall it any more." });
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  // Lessons Thrall noted load with the Learning section too.
  const [lessons, setLessons] = useState<LessonInfo[] | null>(null);
  useEffect(() => {
    if (section !== "memory" || !engine) return;
    engine
      .request<"experience.listed">({ type: "experience.list" })
      .then((r) => setLessons(r.items))
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  }, [section, engine]);
  const forgetLesson = (lesson: string) => {
    engine
      ?.request<"experience.listed">({ type: "experience.forget", lesson })
      .then((r) => {
        setLessons(r.items);
        setStatus({ text: "Deleted: Thrall won't follow that lesson any more." });
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  // The check-in checklist lives in the data folder, beside config.toml.
  const [checklist, setChecklist] = useState<string | null>(null);
  const [checklistDraft, setChecklistDraft] = useState("");
  const [nextRun, setNextRun] = useState<number | null>(null);
  // Typing before the saved checklist arrives keeps what was typed.
  const checklistEdited = useRef(false);
  useEffect(() => {
    engine
      ?.request<"heartbeat.shown">({ type: "heartbeat.get" })
      .then((r) => {
        setChecklist(r.checklist);
        if (!checklistEdited.current) setChecklistDraft(r.checklist);
        setNextRun(r.next_run ?? null);
      })
      .catch(() => setChecklist(""));
    // Re-read when check-ins are switched on or off, for the next time.
  }, [engine, heartbeat.enabled, heartbeat.every_minutes]);
  const saveChecklist = () => {
    engine
      ?.request<"heartbeat.shown">({ type: "heartbeat.set", checklist: checklistDraft })
      .then((r) => {
        setChecklist(r.checklist);
        setChecklistDraft(r.checklist);
        checklistEdited.current = false;
        setStatus({ text: r.checklist ? "Checklist saved." : "Checklist removed: no check-ins until you write one." });
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };
  const checkNow = () => {
    engine
      ?.request({ type: "heartbeat.run" })
      .then(() =>
        setStatus({ text: "Checking now. Anything that needs you shows in Home; otherwise Home notes that all is well." }),
      )
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };
  const perDay = checkInsPerDay(String(heartbeat.active_hours ?? ""), Number(heartbeat.every_minutes) || 30);

  // Starting with Windows: the owner's alone (anyone else is refused and the row stays hidden).
  const [startup, setStartup] = useState<ReplyOf<"startup.shown"> | null>(null);
  useEffect(() => {
    engine
      ?.request<"startup.shown">({ type: "startup.get" })
      .then(setStartup)
      .catch(() => setStartup(null));
  }, [engine]);
  const saveStartup = (on: boolean) => {
    engine
      ?.request<"startup.shown">({ type: "startup.set", on })
      .then((r) => {
        setStartup(r);
        setStatus({
          text: r.on
            ? "Thrall starts in the tray when you sign in."
            : "Thrall won't start when you sign in. Start it yourself after a restart.",
        });
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  // Clearing Home asks once more in its row.
  const [clearing, setClearing] = useState(false);
  const clearHome = () => {
    setClearing(false);
    engine
      ?.request<"home.shown">({ type: "home.clear" })
      .then((r) => {
        onHomeCleared(r.page);
        setStatus({ text: "Home is cleared: every page is deleted and a fresh one has started." });
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  // Removing a skill asks once more in its row.
  const [removing, setRemoving] = useState<string | null>(null);
  const removeSkill = (name: string) => {
    setRemoving(null);
    engine
      ?.request<"settings.shown">({ type: "skill.remove", name })
      .then((r) => {
        setShown(r);
        setStatus({ text: `Removed /${name}.` });
      })
      .catch((e: unknown) => setStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  const chatgptRow = (
    <Row
      label="ChatGPT plan"
      hint={
        signedIn
          ? (shown?.plan_models ?? []).length > 0
            ? `Signed in. Your plan's models: ${(shown?.plan_models ?? []).join(", ")}`
            : "Signed in. Chat models named chatgpt/<model> use your plan (no OpenRouter cost)."
          : "Use your ChatGPT Plus or Pro plan for chat models named chatgpt/<model>"
      }
    >
      <span className={`th-badge${signedIn ? " is-ok" : ""}`}>{signedIn ? "Signed in" : "Not signed in"}</span>
      {signedIn ? (
        <button type="button" className="th-btn" onClick={signOut}>
          Sign out
        </button>
      ) : (
        <button type="button" className="th-btn th-btn-primary" onClick={signIn}>
          Sign in
        </button>
      )}
    </Row>
  );

  const body = (): ReactNode => {
    if (section === "general") {
      return (
        <>
          <Group title="Appearance">
            <Row label="Theme" hint="System follows your computer's setting. The sidebar switch picks light or dark and keeps it.">
              <div className="th-seg" role="radiogroup" aria-label="Theme">
                {(["system", "light", "dark"] as Theme[]).map((t) => (
                  <button
                    key={t}
                    type="button"
                    role="radio"
                    aria-checked={theme === t}
                    className={theme === t ? "is-on" : undefined}
                    onClick={() => onTheme(t)}
                  >
                    {THEME_LABELS[t]}
                  </button>
                ))}
              </div>
            </Row>
          </Group>
          <Group title="Help">
            <Row label="Welcome and tour" hint="Show new people the welcome and offer the one-minute tour (in this browser)">
              <button
                type="button"
                role="switch"
                aria-checked={tour}
                aria-label="Welcome and tour"
                className={`th-switch${tour ? " is-on" : ""}`}
                onClick={() => {
                  setTourOn(!tour);
                  setTour(!tour);
                }}
              >
                <span />
              </button>
            </Row>
            <Row label="The tour" hint="A one-minute look at the screen, any time (also Ctrl+K, Take the tour)">
              <button type="button" className="th-btn" onClick={onTour}>
                Take the tour
              </button>
            </Row>
            <Row label={`Thrall ${version}`} hint="Asks GitHub whether a newer version is out; nothing else is sent">
              <button type="button" className="th-btn" onClick={checkUpdate}>
                Check for updates
              </button>
            </Row>
            {update ? (
              <p className="th-set-status" role="status">
                {update} {update.startsWith("Version") ? <code className="th-set-mono">{RELEASES_PAGE}</code> : null}
              </p>
            ) : null}
          </Group>
          <Group
            title="Home"
            note="Your own conversation with Thrall, apart from your projects. Thrall remembers you there the same as everywhere, and can read your other sessions."
          >
            <Row label="Fresh start" hint="When Home starts an empty page. Earlier pages stay under Earlier">
              <Choice k="home.fresh_start" v={home.fresh_start ?? "daily"} label="Fresh start" options={FRESH_STARTS} onSave={save} />
            </Row>
            <Row label="Keep earlier pages" hint="Days, then they're deleted. 0 keeps them for good">
              <Field k="home.keep_days" v={home.keep_days} label="Keep earlier pages" onSave={save} number />
            </Row>
            <Row label="Clear Home" hint="Deletes every Home page now. Facts Thrall saved stay">
              {clearing ? (
                <>
                  <button type="button" className="th-btn th-btn-danger" onClick={clearHome}>
                    Delete every page
                  </button>
                  <button type="button" className="th-btn th-btn-quiet" onClick={() => setClearing(false)}>
                    Keep
                  </button>
                </>
              ) : (
                <button type="button" className="th-btn th-btn-quiet" onClick={() => setClearing(true)}>
                  Clear Home now
                </button>
              )}
            </Row>
          </Group>
          <Group
            title="Check-ins"
            note="Now and then Thrall goes through your checklist in Home, looking only (it can't change anything or run commands). If nothing needs you, you never see it."
          >
            <Row label="Check in" hint={`Each check-in is one model call. Up to ${perDay} a day with these settings`}>
              <Toggle k="heartbeat.enabled" v={heartbeat.enabled} label="Check in" onSave={save} />
            </Row>
            <Row label="Every" hint="Minutes between check-ins (5 or more)">
              <Field k="heartbeat.every_minutes" v={heartbeat.every_minutes} label="Minutes between check-ins" onSave={save} number />
            </Row>
            <Row label="Active hours" hint="Like 08:00-22:00 (may run past midnight). Empty: any time">
              <Field k="heartbeat.active_hours" v={heartbeat.active_hours} label="Active hours" onSave={save} mono allowEmpty />
            </Row>
            <Row label="Model" hint="A cheaper model just for check-ins. Empty: the chat model">
              <Field k="heartbeat.model" v={heartbeat.model} label="Check-in model" onSave={save} mono allowEmpty />
            </Row>
            <div className="th-set-checklist">
              <label htmlFor="checklist">
                <span>Checklist</span>
                <small>
                  What Thrall looks at each time, one item a line.{" "}
                  {heartbeat.enabled && nextRun ? `Next check-in ${new Date(nextRun).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}.` : ""}
                </small>
              </label>
              <textarea
                id="checklist"
                className="th-input"
                rows={5}
                placeholder={"- Any session waiting on me?\n- Did last night's scheduled jobs work?\n- Anything new in my Downloads folder?"}
                value={checklistDraft}
                onChange={(e) => {
                  checklistEdited.current = true;
                  setChecklistDraft(e.target.value);
                }}
              />
              <div className="th-set-checklist-actions">
                <button type="button" className="th-btn" disabled={checklist === null || checklistDraft === checklist} onClick={saveChecklist}>
                  Save checklist
                </button>
                <button type="button" className="th-btn th-btn-quiet" disabled={!checklist} onClick={checkNow}>
                  Check now
                </button>
              </div>
            </div>
          </Group>
          <Group title="Startup">
            {startup?.available ? (
              <Row
                label={startup.label || STARTUP_LABEL}
                hint="Starts Thrall in the tray when you sign in, so the phone app works after a restart. The same as the tray menu's tick"
              >
                <button
                  type="button"
                  role="switch"
                  aria-checked={startup.on}
                  aria-label={startup.label || STARTUP_LABEL}
                  className={`th-switch${startup.on ? " is-on" : ""}`}
                  onClick={() => saveStartup(!startup.on)}
                >
                  <span />
                </button>
              </Row>
            ) : startup ? (
              <Row label={startup.label || STARTUP_LABEL} hint="Needs Thrall's desktop app, installed beside the engine">
                <span className="th-badge">Not available</span>
              </Row>
            ) : null}
            <Row
              label="Show or hide shortcut"
              hint="Opens Thrall from anywhere (desktop app), like Ctrl+Alt+T. Change it if a game or another app uses the same keys. Empty: none"
            >
              <Field k="desktop.shortcut" v={desktop.shortcut} label="Show or hide shortcut" onSave={save} mono allowEmpty />
            </Row>
            <Row
              label="New folders go in"
              hint="Where “New folder” makes one (the phone app) unless you pick another place. A full path. Empty: Thrall's own folder, Documents\Thrall"
            >
              <Field k="folders.new_in" v={folders.new_in} label="New folders go in" onSave={save} mono allowEmpty />
            </Row>
            <Planned label="Closing the window" hint="Keep running in the tray, or quit" />
          </Group>
          <Group
            title="Notifications"
            note="Pop-ups from the desktop app. Clicking one opens the session (or Home) it's about."
          >
            <Row label="Needs you" hint="Thrall is waiting for your answer or your OK on a plan">
              <Toggle k="notifications.needs_you" v={notifications.needs_you} label="Needs you notifications" onSave={save} />
            </Row>
            <Row label="Done" hint="Something you asked for is finished">
              <Toggle k="notifications.done" v={notifications.done} label="Done notifications" onSave={save} />
            </Row>
            <Row label="Couldn't finish" hint="Something you asked for failed">
              <Toggle k="notifications.failed" v={notifications.failed} label="Couldn't finish notifications" onSave={save} />
            </Row>
            <Row label="Background work" hint="A helper or scheduled job finished">
              <Toggle k="notifications.background" v={notifications.background} label="Background work notifications" onSave={save} />
            </Row>
            <Row label="Check-ins" hint="A check-in found something worth saying">
              <Toggle k="notifications.check_ins" v={notifications.check_ins} label="Check-in notifications" onSave={save} />
            </Row>
            <Row label="Quiet hours" hint="Like 22:00-07:00. Only Needs you comes through then. Empty: none">
              <Field k="notifications.quiet_hours" v={notifications.quiet_hours} label="Quiet hours" onSave={save} mono allowEmpty />
            </Row>
            <Row label="Only when I'm away" hint="No pop-ups while Thrall's window is the one in front">
              <Toggle k="notifications.only_when_away" v={notifications.only_when_away} label="Only when Thrall isn't in front" onSave={save} />
            </Row>
          </Group>
        </>
      );
    }
    if (section === "models") {
      return (
        <>
          <Group title="Chat" note={saved}>
            <Row label="Provider"><Value v={provider.kind} /></Row>
            <Row
              label="Model"
              hint={shown && shown.model !== provider.model ? `In use now: ${shown.model} (picked with /model)` : "An OpenRouter id (xiaomi/mimo-v2.5-pro), chatgpt/<model> on your ChatGPT plan, or another supplier's (Providers)"}
            >
              <Field k="provider.model" v={provider.model} label="Model" onSave={save} mono list="th-plan-models" />
              <datalist id="th-plan-models">
                {(shown?.plan_models ?? []).map((m) => (
                  <option key={m} value={m} />
                ))}
              </datalist>
            </Row>
            <Row label="Longest reply" hint="Tokens">
              <Field k="provider.max_tokens" v={provider.max_tokens} label="Longest reply" onSave={save} number />
            </Row>
            {chatgptRow}
          </Group>
          <Group title="Profiles" note="Named models to switch between with /model @name.">
            {Object.keys(profiles).length === 0 ? <p className="th-set-empty">None yet.</p> : null}
            {Object.entries(profiles).map(([name, p]) => {
              const profile = obj(p);
              return (
                <Row key={name} label={`@${name}`} hint={text(profile.description) === "—" ? undefined : text(profile.description)}>
                  <span className="th-set-mono">{text(profile.model)}</span>
                  <button type="button" className="th-btn" onClick={() => useProfile(name)}>
                    Use
                  </button>
                  <button type="button" className="th-btn th-btn-quiet" aria-label={`Remove @${name}`} onClick={() => save(`profiles.${name}.model`, "")}>
                    Remove
                  </button>
                </Row>
              );
            })}
            <ProfileAdder onAdd={addProfile} />
          </Group>
          <Group title="Pictures, voice and video" note="Who reads and makes pictures, speaks and transcribes is picked in Providers.">
            <Row label="Backup picture reader" hint="Tried when the picture reader is busy or down">
              <Field k="provider.vision_backup_model" v={provider.vision_backup_model} label="Backup picture reader" onSave={save} mono />
            </Row>
            <Planned label="Music" hint="Songs and backing tracks" />
          </Group>
        </>
      );
    }
    if (section === "safety") {
      return (
        <>
          <Group title="Checks">
            <Row label="Check its work" hint="After changing files, Thrall checks before saying it's done">
              <Toggle k="agent.verify" v={agent.verify} label="Check its work" onSave={save} />
            </Row>
            <Row label="Claim check" hint="The final reply is checked against what Thrall actually did">
              <Toggle k="agent.claim_check" v={agent.claim_check} label="Claim check" onSave={save} />
            </Row>
          </Group>
          <Group title="Questions">
            <Row
              label="When Thrall asks"
              hint="Otherwise it decides, carries on, and lists what it assumed at the end. Full auto never asks."
            >
              <Choice
                k="agent.questions"
                v={agent.questions ?? "when_it_matters"}
                label="When Thrall asks"
                options={QUESTION_MODES}
                labels={QUESTION_MODE_LABELS}
                onSave={save}
              />
            </Row>
            <Row label="Wait for an answer" hint="Minutes before Thrall goes ahead with its recommended answers">
              <Field k="agent.question_wait_minutes" v={agent.question_wait_minutes} label="Wait for an answer" onSave={save} number />
            </Row>
          </Group>
          <Group title="Permissions">
            <Planned label="Mode for new sessions" hint="Ask, auto-accept edits, plan or full auto" />
            <Row label="Checkpoints" hint="Files are kept before each edit; Rewind on any of your messages puts them back">
              <span className="th-badge is-ok">On</span>
            </Row>
            <Planned label="Always-allowed commands" hint="Commands Thrall may run without asking" />
          </Group>
        </>
      );
    }
    if (section === "memory") {
      return (
        <>
          <Group title="Memory">
            <Row label="Remember conversations" hint="Later sessions can recall finished turns">
              <Toggle k="memory.remember_conversations" v={memory.remember_conversations} label="Remember conversations" onSave={save} />
            </Row>
            <Row label="Context before summarising" hint="Tokens">
              <Field k="memory.context_limit_tokens" v={memory.context_limit_tokens} label="Context before summarising" onSave={save} number />
            </Row>
            <Row label="Search model" hint="Finds memories by meaning">
              <Field k="memory.embedding_model" v={memory.embedding_model} label="Search model" onSave={save} mono />
            </Row>
            <Row label="Save facts without asking" hint="Thrall tells you each one, and you can forget it. Off: it asks first">
              <Toggle k="memory.save_facts" v={memory.save_facts} label="Save facts without asking" onSave={save} />
            </Row>
            <Row label="Trim old tool results (experimental)" hint="Big results from a few exchanges ago are sent shortened to save tokens. Nothing is deleted. If Thrall forgets something it read earlier, turn this off">
              <Toggle k="memory.trim_old_tool_results" v={memory.trim_old_tool_results} label="Trim old tool results" onSave={save} />
            </Row>
          </Group>
          <Group
            title="Lessons Thrall learned"
            note={
              lessons === null
                ? "Loading…"
                : lessons.length === 0
                  ? undefined
                  : "Noted when you corrected Thrall or it found what works. The newest are in every chat; delete any that are wrong."
            }
          >
            {lessons !== null && lessons.length === 0 ? (
              <p className="th-set-empty">None yet. When you correct Thrall, it notes the lesson here.</p>
            ) : null}
            {(lessons ?? []).map((l) => (
              <Row key={l.lesson} label={l.text} hint={new Date(l.at).toLocaleDateString()}>
                <button type="button" className="th-btn th-btn-quiet" aria-label={`Delete the lesson: ${l.text}`} onClick={() => forgetLesson(l.lesson)}>
                  Delete
                </button>
              </Row>
            ))}
          </Group>
          <Group
            title="Facts Thrall remembers"
            note={facts === null ? "Loading…" : facts.length === 0 ? undefined : `${facts.length} saved. Forget any that are wrong or out of date.`}
          >
            {facts !== null && facts.length === 0 ? (
              <p className="th-set-empty">None yet. Tell Thrall about yourself or your setup and it remembers.</p>
            ) : null}
            {(facts ?? []).map((f) => (
              <Row key={f.fact} label={f.topic} hint={f.content}>
                <button type="button" className="th-btn th-btn-quiet" aria-label={`Forget ${f.topic}`} onClick={() => forgetFact(f.fact)}>
                  Forget
                </button>
              </Row>
            ))}
          </Group>
          <Group
            title="Your skills"
            note="Recipes Thrall uses only when a task matches, or when you type /name. Thrall learns new ones by itself after working something out, each with a safety rating; ones rated High are put to you first. Turn any off or remove it here."
          >
            {(shown?.skills ?? []).length === 0 ? (
              <p className="th-set-empty">None yet. A project's own skills (its .thrall/skills folder) work in that folder.</p>
            ) : null}
            {(shown?.skills ?? []).map((s) => (
              <Row key={s.name} label={`/${s.name}`} hint={s.description || undefined}>
                {s.rating ? (
                  <span className={`th-badge is-rating-${s.rating}`} title="Safety rating, from where Thrall learned it and what it does">
                    {RATING_LABELS[s.rating] ?? s.rating}
                  </span>
                ) : null}
                <span className="th-badge" title={s.helper ? "A helper does the work: you see its steps, but only its result goes into Thrall's conversation" : "Thrall follows it in the conversation itself"}>
                  {s.helper ? "In a helper" : "Inline"}
                </span>
                <Toggle k={`skills.${s.name}.enabled`} v={s.enabled} label={`Use /${s.name}`} onSave={save} />
                {removing === s.name ? (
                  <>
                    <button type="button" className="th-btn th-btn-danger" onClick={() => removeSkill(s.name)}>
                      Remove
                    </button>
                    <button type="button" className="th-btn th-btn-quiet" onClick={() => setRemoving(null)}>
                      Keep
                    </button>
                  </>
                ) : (
                  <button type="button" className="th-btn th-btn-quiet" aria-label={`Remove /${s.name}`} onClick={() => setRemoving(s.name)}>
                    Remove
                  </button>
                )}
              </Row>
            ))}
          </Group>
        </>
      );
    }
    if (section === "writing") {
      return (
        <Group
          title="Writing style"
          note="The humaniser keeps Thrall clear of the habits that make writing read as machine-made: puffed-up phrases, chatbot talk, stock endings, overused words, flat rhythm."
        >
          <Row label="Humaniser" hint="Light: Thrall's replies. Full: also the documents it writes, each checked after saving and fixed">
            <Choice k="writing.humanise" v={writing.humanise ?? "off"} label="Humaniser" options={HUMANISE_LEVELS} onSave={save} />
          </Row>
          <Row
            label="Detector-aware /humanise"
            hint="/humanise also evens out what AI detectors measure: sentence rhythm and the most expected words. Detectors are unreliable; nothing is guaranteed to pass"
          >
            <Toggle k="writing.detector_aware" v={writing.detector_aware} label="Detector-aware /humanise" onSave={save} />
          </Row>
        </Group>
      );
    }
    if (section === "tools") {
      const names = Object.keys(servers);
      return (
        <>
          <Group
            title="MCP servers"
            note={
              names.length
                ? "Started only when Thrall needs them. Only you can use a server until you share it; everyone then uses it as you, with your sign-ins and keys. Values that could be keys are hidden."
                : undefined
            }
          >
            {names.length === 0 ? (
              <p className="th-set-empty">
                None yet. Add them under [mcp.servers.&lt;name&gt;] in config.toml: command = "…" for a program on this computer, or url = "https://…" for a server on the web.
              </p>
            ) : (
              names.map((n) => {
                const s = obj(servers[n]);
                const url = text(s.url);
                const who = s.shared === true ? "Shared with everyone." : "Only you.";
                const share = <Toggle k={`mcp.servers.${n}.shared`} v={s.shared} label={`Share ${n} with everyone`} onSave={save} />;
                if (url) {
                  const headers = Object.keys(obj(s.headers));
                  const signedIn = mcpSignedIn.includes(n);
                  const waiting = mcpSigningIn?.server === n;
                  const hint = [
                    "On the web.",
                    who,
                    headers.length ? `Headers: ${headers.join(", ")}.` : "",
                    signedIn ? "Signed in." : "Sign in only if it asks you to.",
                  ]
                    .filter(Boolean)
                    .join(" ");
                  return (
                    <Row key={n} label={n} hint={hint}>
                      <span className="th-set-mono">{url}</span>
                      {waiting && mcpSigningIn.url ? (
                        <a className="th-btn" href={mcpSigningIn.url} target="_blank" rel="noreferrer">
                          Open sign-in page
                        </a>
                      ) : null}
                      {signedIn ? (
                        <button type="button" className="th-btn" onClick={() => mcpSignOut(n)}>
                          Sign out
                        </button>
                      ) : (
                        <button type="button" className="th-btn" disabled={waiting} onClick={() => mcpSignIn(n)}>
                          {waiting ? "Waiting…" : "Sign in"}
                        </button>
                      )}
                      {share}
                    </Row>
                  );
                }
                const args = Array.isArray(s.args) ? (s.args as unknown[]).map(String).join(" ") : "";
                const env = Object.keys(obj(s.env));
                return (
                  <Row key={n} label={n} hint={`On this computer. ${who}${env.length ? ` Environment: ${env.join(", ")}` : ""}`}>
                    <span className="th-set-mono">{`${text(s.command)} ${args}`.trim()}</span>
                    {share}
                  </Row>
                );
              })
            )}
            <Planned label="Add a server" hint="Find, install and connect an MCP server from here" />
          </Group>
          <Group title="Hooks" note="Your own commands, run at points in a turn.">
            {hooks.length === 0 ? (
              <p className="th-set-empty">None.</p>
            ) : (
              hooks.map((h, i) => (
                <Row key={i} label={text(h.event)} hint={h.tool ? `Only for ${text(h.tool)}` : undefined}>
                  <span className="th-set-mono">{text(h.command)}</span>
                </Row>
              ))
            )}
          </Group>
          <Group title="Integrations">
            <Planned label="VS Code" hint="Thrall beside your editor, with its errors and warnings" />
            <Planned label="GitHub reviews" hint="Thrall reviews pull requests" />
            <Planned label="Telegram" hint="Talk to Thrall from Telegram" phase="On hold" />
          </Group>
        </>
      );
    }
    if (section === "personality") {
      return <Personality engine={engine} onStatus={setStatus} Group={Group} />;
    }
    if (section === "people") {
      return <People engine={engine} onStatus={setStatus} Row={Row} Group={Group} />;
    }
    if (section === "providers") {
      return (
        <Providers engine={engine} shown={shown} onSave={save} onStatus={setStatus} Row={Row} Group={Group} Field={Field} />
      );
    }
    if (section === "phones") {
      return (
        <Phones
          engine={engine}
          remote={obj(config.remote)}
          onSave={save}
          onStatus={setStatus}
          Row={Row}
          Group={Group}
          Toggle={Toggle}
          Field={Field}
        />
      );
    }
    if (section === "keys") {
      return (
        <Group title="Keys" note="Kept in this computer's keychain. They never pass through this screen; set or replace one in a terminal with the command shown.">
          {(shown?.keys ?? []).filter((k) => k.name !== "chatgpt").map((k) => (
            <Row key={k.name} label={k.name} hint={k.purpose}>
              <span className={`th-badge${k.set ? " is-ok" : " is-warn"}`}>{k.set ? "Set" : "Not set"}</span>
              <code className="th-set-mono">thrall key set {k.name}</code>
            </Row>
          ))}
        </Group>
      );
    }
    return (
      <Group title="Advanced" note="Change these only for a proxy or a compatible service.">
        <Row label="Config file"><Value v={shown?.config_file} mono /></Row>
        <Row label="OpenRouter address"><Field k="provider.base_url" v={provider.base_url} label="OpenRouter address" onSave={save} mono /></Row>
        <Row label="fal.ai address"><Field k="provider.fal_base_url" v={provider.fal_base_url} label="fal.ai address" onSave={save} mono /></Row>
        <Row label="Log detail">
          <Choice k="logging.level" v={logging.level} label="Log detail" options={LOG_LEVELS} onSave={save} />
        </Row>
      </Group>
    );
  };

  return (
    <div className="th-modal-scrim" onMouseDown={onClose}>
      <div
        className="th-settings"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="th-settings-head">
          <h3 id="settings-title">Settings</h3>
          <button ref={closeRef} type="button" className="th-btn th-btn-quiet" onClick={onClose} aria-label="Close settings">
            ✕
          </button>
        </header>
        <nav className="th-settings-nav" aria-label="Settings sections">
          {SETTINGS_SECTIONS.map((s) => (
            <button
              key={s.id}
              type="button"
              className={section === s.id ? "is-on" : undefined}
              aria-current={section === s.id ? "page" : undefined}
              onClick={() => setSection(s.id)}
            >
              {s.label}
            </button>
          ))}
        </nav>
        <div className="th-settings-body">
          {failed ? <p className="th-set-note is-warn">Couldn't load Thrall's settings: {failed}</p> : null}
          {status ? (
            <p className={`th-set-status${status.warn ? " is-warn" : ""}`} role="status">
              {status.text}
              {signInUrl ? (
                <>
                  {" "}
                  <a href={signInUrl} target="_blank" rel="noopener noreferrer">
                    Open the page
                  </a>
                </>
              ) : null}
            </p>
          ) : null}
          {body()}
        </div>
      </div>
    </div>
  );
}
