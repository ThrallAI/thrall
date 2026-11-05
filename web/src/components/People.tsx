// Settings → People: who uses this Thrall, what each has spent this month, whose AI key their
// turns use and their monthly limit, sign-in links for their browsers, and the browsers signed in
// as them. The engine keeps people apart and enforces the limits; this only shows them. Phones
// are handed to people in Settings → Phones.

import { useCallback, useEffect, useState, type FormEvent, type ReactNode } from "react";

import type { Engine, ReplyOf } from "../engine";
import { dollars, monthStart, signInLink } from "../people";
import { CopyButton } from "./CopyButton";

type Person = ReplyOf<"people.listed">["items"][number];
type Devices = ReplyOf<"device.listed">;
type Status = (s: { text: string; warn?: boolean }) => void;

function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function said(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** A person's key, model and monthly limit, being edited. Keys are only ever written. */
function TermsEditor({
  person,
  onSave,
  onKey,
  onClose,
}: {
  person: Person;
  onSave: (ownKey: boolean, model: string, limit: string) => void;
  onKey: (key: string) => void;
  onClose: () => void;
}) {
  const owner = person.role === "owner";
  const [ownKey, setOwnKey] = useState(person.own_key ?? false);
  const [model, setModel] = useState(person.model ?? "");
  const [limit, setLimit] = useState(person.monthly_limit != null ? String(person.monthly_limit) : "");
  const [key, setKey] = useState("");
  const limitOk = limit.trim() === "" || (Number(limit) > 0 && Number.isFinite(Number(limit)));
  return (
    <form
      className="th-people-terms"
      onSubmit={(e) => {
        e.preventDefault();
        if (limitOk) onSave(ownKey, model, limit);
      }}
    >
      {owner ? (
        <p className="th-set-note">
          You use this Thrall's own key and model (Keys and Models). A limit on yourself stops runaway spending.
        </p>
      ) : (
        <>
          <fieldset className="th-people-choice">
            <legend>Whose AI key</legend>
            <label>
              <input type="radio" name={`key-${person.person}`} checked={!ownKey} onChange={() => setOwnKey(false)} /> Yours
            </label>
            <label>
              <input type="radio" name={`key-${person.person}`} checked={ownKey} onChange={() => setOwnKey(true)} /> Their own
            </label>
          </fieldset>
          {ownKey ? (
            <div className="th-set-add">
              <input
                className="th-input th-set-mono"
                type="password"
                autoComplete="off"
                aria-label={`${person.name}'s own key`}
                placeholder={person.key_set ? "Key set; paste a new one to replace it" : "Paste their key (sk-or-…)"}
                value={key}
                onChange={(e) => setKey(e.target.value)}
              />
              <button
                type="button"
                className="th-btn"
                disabled={!key.trim()}
                onClick={() => {
                  onKey(key);
                  setKey("");
                }}
              >
                Save key
              </button>
              {person.key_set ? (
                <button type="button" className="th-btn th-btn-quiet" onClick={() => onKey("")}>
                  Forget key
                </button>
              ) : null}
            </div>
          ) : null}
          <label className="th-people-field">
            <span>Their model</span>
            <input
              className="th-input th-set-mono"
              placeholder="the same as yours"
              value={model}
              onChange={(e) => setModel(e.target.value)}
            />
          </label>
        </>
      )}
      <label className="th-people-field">
        <span>Monthly limit, US$</span>
        <input
          className={`th-input is-number${limitOk ? "" : " is-bad"}`}
          inputMode="decimal"
          placeholder="no limit"
          value={limit}
          onChange={(e) => setLimit(e.target.value)}
        />
      </label>
      <div className="th-set-add">
        <button type="submit" className="th-btn th-btn-primary" disabled={!limitOk}>
          Save
        </button>
        <button type="button" className="th-btn th-btn-quiet" onClick={onClose}>
          Close
        </button>
      </div>
    </form>
  );
}

type Rules = NonNullable<Person["rules"]>;

/** How many of a person's controls are on (something turned off for them). */
function controlsOn(rules: Rules | undefined): number {
  if (!rules) return 0;
  return [rules.no_commands, rules.no_system_folders, rules.no_phones, rules.child].filter(Boolean).length;
}

/** What Thrall may do for someone, all allowed until switched off. Saved as each one changes. */
function ControlsEditor({ person, onSave, onClose }: { person: Person; onSave: (rules: Rules) => void; onClose: () => void }) {
  const rules: Rules = person.rules ?? {};
  const set = (change: Partial<Rules>) => onSave({ ...rules, ...change });
  return (
    <div className="th-people-terms">
      <p className="th-set-note">Everything is allowed until you switch it off. What's off shows beside {person.name}'s name.</p>
      <label className="th-people-check">
        <input type="checkbox" checked={!rules.no_commands} onChange={(e) => set({ no_commands: !e.target.checked })} /> Thrall can run
        commands for them
      </label>
      <label className="th-people-check">
        <input type="checkbox" checked={!rules.no_system_folders} onChange={(e) => set({ no_system_folders: !e.target.checked })} />{" "}
        Thrall can change files in system folders for them (Windows, programs, other people's folders)
      </label>
      <label className="th-people-check">
        <input type="checkbox" checked={!rules.no_phones} onChange={(e) => set({ no_phones: !e.target.checked })} /> Their phones can
        connect
      </label>
      <label className="th-people-check">
        <input type="checkbox" checked={Boolean(rules.child)} onChange={(e) => set({ child: e.target.checked ? "under_13" : undefined })} />{" "}
        Child's account: answers fit for their age
      </label>
      {rules.child ? (
        <label className="th-people-field">
          <span>Their age</span>
          <select className="th-input" value={rules.child} onChange={(e) => set({ child: e.target.value })}>
            <option value="under_13">Under 13</option>
            <option value="13_15">13 to 15</option>
            <option value="16_17">16 or 17</option>
          </select>
        </label>
      ) : null}
      <div className="th-set-add">
        <button type="button" className="th-btn th-btn-quiet" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  );
}

export function People({
  engine,
  onStatus,
  Row,
  Group,
}: {
  engine: Engine | null;
  onStatus: Status;
  Row: (p: { label: string; hint?: string; children: ReactNode }) => ReactNode;
  Group: (p: { title: string; note?: string; children: ReactNode }) => ReactNode;
}) {
  const [people, setPeople] = useState<Person[]>([]);
  const [adding, setAdding] = useState("");
  const [renaming, setRenaming] = useState<{ person: string; name: string } | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [link, setLink] = useState<{ person: string; url: string; expires: number } | null>(null);
  const [devices, setDevices] = useState<Devices | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [controlling, setControlling] = useState<string | null>(null);

  const load = useCallback(
    () =>
      engine
        ?.request<"people.listed">({ type: "people.list", since: monthStart(new Date()) })
        .then((r) => setPeople(r.items))
        .catch((e: unknown) => onStatus({ text: said(e), warn: true })),
    [engine, onStatus],
  );

  useEffect(() => {
    void load();
  }, [load]);

  /** Run a people request, then re-read the list (its reply counts spending from the start). */
  const change = (request: Parameters<Engine["request"]>[0], done: string) =>
    engine
      ?.request<"people.listed">(request)
      .then(() => {
        onStatus({ text: done });
        return load();
      })
      .catch((e: unknown) => onStatus({ text: said(e), warn: true }));

  const add = (e: FormEvent) => {
    e.preventDefault();
    const name = adding.trim();
    if (!name) return;
    void change({ type: "people.add", name }, `Added ${name}. Hand them a phone in Phones, or make them a sign-in link.`)?.then(() => setAdding(""));
  };

  const rename = (e: FormEvent) => {
    e.preventDefault();
    if (!renaming) return;
    const { person, name } = renaming;
    setRenaming(null);
    void change({ type: "people.rename", person, name }, "Renamed.");
  };

  const remove = (p: Person) => {
    setRemoving(null);
    if (link?.person === p.person) setLink(null);
    if (devices?.person === p.person) setDevices(null);
    void change(
      { type: "people.remove", person: p.person },
      `Removed ${p.name}. Their phones and browsers can't reach Thrall any more; their chats and files are kept.`,
    );
  };

  const saveTerms = (p: Person, ownKey: boolean, model: string, limit: string) => {
    const trimmed = limit.trim();
    void change(
      {
        type: "people.terms",
        person: p.person,
        own_key: ownKey,
        model: model.trim() || undefined,
        monthly_limit: trimmed === "" ? undefined : Number(trimmed),
      },
      ownKey && !p.key_set ? `Saved. ${p.name}'s turns wait until their key is set.` : "Saved.",
    );
  };

  const saveRules = (p: Person, rules: Rules) => {
    void change({ type: "people.rules", person: p.person, rules }, `Saved. It applies to ${p.name} from their next step.`);
  };

  const saveKey = (p: Person, key: string) => {
    engine
      ?.request<"ack">({ type: "people.key", person: p.person, key: key.trim() || undefined })
      .then(() => {
        onStatus({ text: key.trim() ? `${p.name}'s key is kept. It's never shown again.` : `${p.name}'s key is forgotten.` });
        return load();
      })
      .catch((e: unknown) => onStatus({ text: said(e), warn: true }));
  };

  const makeLink = (p: Person) => {
    engine
      ?.request<"signin.code">({ type: "signin.create", person: p.person })
      .then((r) => {
        setLink({ person: p.person, url: signInLink(location, r.code), expires: r.expires });
        onStatus({ text: `Sign-in link for ${p.name} ready. Send it only to them.` });
      })
      .catch((e: unknown) => onStatus({ text: said(e), warn: true }));
  };

  const showDevices = (p: Person) => {
    if (devices?.person === p.person) {
      setDevices(null);
      return;
    }
    engine
      ?.request<"device.listed">({ type: "device.list", person: p.person })
      .then(setDevices)
      .catch((e: unknown) => onStatus({ text: said(e), warn: true }));
  };

  const signOut = (person: string, device: string) => {
    engine
      ?.request<"device.listed">({ type: "device.remove", device, person })
      .then((r) => {
        setDevices(r);
        onStatus({ text: "Signed out. That browser can't reach Thrall until it gets a new link." });
      })
      .catch((e: unknown) => onStatus({ text: said(e), warn: true }));
  };

  const linkFor = (p: Person) =>
    link?.person === p.person ? (
      <div className="th-pairing">
        <p>
          Open this in {p.name}'s browser; that browser is them from then on. It works once, until{" "}
          <strong>{new Date(link.expires).toLocaleTimeString(undefined, { timeStyle: "short" })}</strong>. For now it only opens on
          this computer; reaching Thrall from other computers comes later.
        </p>
        <p className="th-set-mono th-people-link">{link.url}</p>
        <p>
          <CopyButton text={() => link.url} label="Copy link" />{" "}
          <button type="button" className="th-btn th-btn-quiet" onClick={() => setLink(null)}>
            Done
          </button>
        </p>
      </div>
    ) : null;

  const devicesFor = (p: Person) =>
    devices?.person === p.person ? (
      <div className="th-people-devices">
        {devices.items.length === 0 ? (
          <p className="th-set-empty">No browsers signed in as {p.name}.</p>
        ) : (
          devices.items.map((d) => (
            <Row
              key={d.device}
              label={d.current ? `${d.name} (this one)` : d.name}
              hint={[`Signed in ${when(d.added)}`, d.last_seen ? `last used ${when(d.last_seen)}` : "not used since"].join(" · ")}
            >
              <button type="button" className="th-btn th-btn-quiet" onClick={() => signOut(p.person, d.device)}>
                Sign out
              </button>
            </Row>
          ))
        )}
      </div>
    ) : null;

  return (
    <>
      <Group
        title="People"
        note="Everyone gets their own chats, files and memory. They use your key and model unless you give them their own. Their work runs on this computer, or on their own PC once it joins (Settings → PCs). Spending is this month's, from the first; a limit stops their turns when it's reached."
      >
        {people.map((p) => {
          const owner = p.role === "owner";
          const spent = p.spent ?? 0;
          const hint = [
            owner ? "Runs this Thrall" : null,
            `${p.phones} ${p.phones === 1 ? "phone" : "phones"}`,
            p.monthly_limit != null ? `${dollars(spent)} of ${dollars(p.monthly_limit)} this month` : `${dollars(spent)} this month`,
            p.own_key ? (p.key_set ? "their own key" : "their own key (not set)") : null,
            p.model ? p.model : null,
            p.rules?.no_commands ? "no commands" : null,
            p.rules?.no_system_folders ? "no system folders" : null,
            p.rules?.no_phones ? "no phones" : null,
            p.rules?.child ? "child's account" : null,
          ]
            .filter(Boolean)
            .join(" · ");
          return (
            <div key={p.person} className="th-people-person">
              {renaming?.person === p.person ? (
                <form className="th-set-add" onSubmit={rename}>
                  <input
                    className="th-input"
                    aria-label={`New name for ${p.name}`}
                    value={renaming.name}
                    autoFocus
                    onChange={(e) => setRenaming({ person: p.person, name: e.target.value })}
                  />
                  <button type="submit" className="th-btn" disabled={!renaming.name.trim()}>
                    Save
                  </button>
                  <button type="button" className="th-btn th-btn-quiet" onClick={() => setRenaming(null)}>
                    Cancel
                  </button>
                </form>
              ) : (
                <Row label={p.name} hint={hint}>
                  {removing === p.person ? (
                    <>
                      <button type="button" className="th-btn th-btn-danger" onClick={() => remove(p)}>
                        Remove
                      </button>
                      <button type="button" className="th-btn th-btn-quiet" onClick={() => setRemoving(null)}>
                        Keep
                      </button>
                    </>
                  ) : (
                    <>
                      {p.own_key && !p.key_set ? <span className="th-badge is-warn">Needs key</span> : null}
                      {p.monthly_limit != null && spent >= p.monthly_limit ? <span className="th-badge is-warn">At limit</span> : null}
                      <button
                        type="button"
                        className="th-btn th-btn-quiet"
                        aria-expanded={editing === p.person}
                        onClick={() => setEditing(editing === p.person ? null : p.person)}
                      >
                        Key & limit
                      </button>
                      {owner ? null : (
                        <button
                          type="button"
                          className="th-btn th-btn-quiet"
                          aria-expanded={controlling === p.person}
                          onClick={() => setControlling(controlling === p.person ? null : p.person)}
                        >
                          {controlsOn(p.rules) > 0 ? `Controls (${controlsOn(p.rules)} on)` : "Controls"}
                        </button>
                      )}
                      <button type="button" className="th-btn th-btn-quiet" onClick={() => makeLink(p)}>
                        Sign-in link
                      </button>
                      <button
                        type="button"
                        className="th-btn th-btn-quiet"
                        aria-expanded={devices?.person === p.person}
                        onClick={() => showDevices(p)}
                      >
                        Browsers
                      </button>
                      <button type="button" className="th-btn th-btn-quiet" onClick={() => setRenaming({ person: p.person, name: p.name })}>
                        Rename
                      </button>
                      {owner ? null : (
                        <button type="button" className="th-btn th-btn-quiet" onClick={() => setRemoving(p.person)}>
                          Remove…
                        </button>
                      )}
                    </>
                  )}
                </Row>
              )}
              {editing === p.person ? (
                <TermsEditor
                  key={`${p.person}-${p.own_key}-${p.model ?? ""}-${p.monthly_limit ?? ""}`}
                  person={p}
                  onSave={(ownKey, model, limit) => saveTerms(p, ownKey, model, limit)}
                  onKey={(key) => saveKey(p, key)}
                  onClose={() => setEditing(null)}
                />
              ) : null}
              {controlling === p.person && !owner ? (
                <ControlsEditor person={p} onSave={(rules) => saveRules(p, rules)} onClose={() => setControlling(null)} />
              ) : null}
              {linkFor(p)}
              {devicesFor(p)}
            </div>
          );
        })}
        <form className="th-set-add" onSubmit={add}>
          <input className="th-input" aria-label="New person's name" placeholder="Name, e.g. Sam" value={adding} onChange={(e) => setAdding(e.target.value)} />
          <button type="submit" className="th-btn" disabled={!adding.trim()}>
            Add person
          </button>
        </form>
      </Group>
    </>
  );
}
