// Settings → PCs: other PCs at home or work join this Thrall as someone's companion. The switch
// lets them find it and ask; each one that asks is let in (as someone's), turned away or blocked
// here or from the card that pops up. The engine does the handshake and the encryption; this only
// shows it.

import { useCallback, useEffect, useState, type ReactNode } from "react";

import type { Engine, ReplyOf } from "../engine";
import { PHONE_POLL_MS } from "../constants";

type Requests = ReplyOf<"pc.requests">;
type Listed = ReplyOf<"phone.listed">;
type Person = ReplyOf<"people.listed">["items"][number];
type Save = (key: string, value: boolean | number | string) => void;
type Status = (s: { text: string; warn?: boolean }) => void;

function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** One PC asking to join: its name and code, whose it'll be, and Let in / Decline / Block. */
export function PcAsking({
  engine,
  request,
  people,
  onDone,
  onStatus,
}: {
  engine: Engine | null;
  request: Requests["items"][number];
  people: Person[];
  onDone: (r: Requests) => void;
  onStatus: Status;
}) {
  const owner = people.find((p) => p.role === "owner")?.person ?? "";
  const [whose, setWhose] = useState(owner);
  useEffect(() => {
    if (!whose && owner) setWhose(owner);
  }, [owner, whose]);
  const name = (person: string) => people.find((p) => p.person === person)?.name ?? "theirs";

  const accept = () =>
    engine
      ?.request<"pc.requests">({ type: "pc.accept", request: request.request, person: whose || owner })
      .then((r) => {
        onDone(r);
        onStatus({ text: `${request.name} joined as ${name(whose || owner)}'s PC.` });
      })
      .catch((e: unknown) => onStatus({ text: message(e), warn: true }));
  const decline = (block: boolean) =>
    engine
      ?.request<"pc.requests">({ type: "pc.decline", request: request.request, block })
      .then((r) => {
        onDone(r);
        onStatus({ text: block ? `Blocked ${request.name}. It can't ask again.` : `Turned ${request.name} away.` });
      })
      .catch((e: unknown) => onStatus({ text: message(e), warn: true }));

  return (
    <div className="th-ask">
      <p className="th-ask-title">
        <strong>{request.name}</strong> wants to join
      </p>
      <p className="th-ask-code">
        Code <span>{request.code}</span>
      </p>
      <p className="th-ask-hint">Check it's the code on that PC's screen.</p>
      <div className="th-ask-actions">
        {people.length > 1 ? (
          <select className="th-input" aria-label="Whose PC it is" value={whose || owner} onChange={(e) => setWhose(e.target.value)}>
            {people.map((p) => (
              <option key={p.person} value={p.person}>
                {p.name}'s PC
              </option>
            ))}
          </select>
        ) : null}
        <button type="button" className="th-btn th-btn-primary" onClick={() => void accept()}>
          Let in
        </button>
        <button type="button" className="th-btn th-btn-quiet" onClick={() => void decline(false)}>
          Decline
        </button>
        <button type="button" className="th-btn th-btn-quiet" onClick={() => void decline(true)}>
          Block
        </button>
      </div>
    </div>
  );
}

export function PCs({
  engine,
  remote,
  onSave,
  onStatus,
  Row,
  Group,
  Toggle,
}: {
  engine: Engine | null;
  /** `[remote]` from the config. */
  remote: Record<string, unknown>;
  onSave: Save;
  onStatus: Status;
  Row: (p: { label: string; hint?: string; children: ReactNode }) => ReactNode;
  Group: (p: { title: string; note?: string; children: ReactNode }) => ReactNode;
  Toggle: (p: { k: string; v: unknown; label: string; onSave: Save }) => ReactNode;
}) {
  const [requests, setRequests] = useState<Requests | null>(null);
  const [listed, setListed] = useState<Listed | null>(null);
  const [people, setPeople] = useState<Person[]>([]);
  const [removing, setRemoving] = useState<string | null>(null);

  useEffect(() => {
    engine
      ?.request<"people.listed">({ type: "people.list" })
      .then((r) => setPeople(r.items))
      .catch(() => setPeople([]));
  }, [engine]);
  const owner = people.find((p) => p.role === "owner")?.person ?? "";

  const load = useCallback(() => {
    engine
      ?.request<"pc.requests">({ type: "pc.requests" })
      .then(setRequests)
      .catch((e: unknown) => onStatus({ text: message(e), warn: true }));
    engine
      ?.request<"phone.listed">({ type: "phone.list" })
      .then(setListed)
      .catch(() => undefined);
  }, [engine, onStatus]);

  // PCs ask at any moment, and the listener follows the switch within seconds.
  useEffect(() => {
    load();
    const t = window.setInterval(load, PHONE_POLL_MS);
    return () => window.clearInterval(t);
  }, [load, remote.pcs]);

  const assign = (phone: string, person: string) =>
    engine
      ?.request<"phone.listed">({ type: "phone.assign", phone, person })
      .then((r) => {
        setListed(r);
        onStatus({ text: "Done. That PC reconnects as them." });
      })
      .catch((e: unknown) => onStatus({ text: message(e), warn: true }));
  const remove = (phone: string) => {
    setRemoving(null);
    engine
      ?.request<"phone.listed">({ type: "phone.remove", phone })
      .then((r) => {
        setListed(r);
        onStatus({ text: "Removed. That PC can't reach Thrall any more." });
      })
      .catch((e: unknown) => onStatus({ text: message(e), warn: true }));
  };
  const unblock = (blocked: string) =>
    engine
      ?.request<"pc.requests">({ type: "pc.unblock", blocked })
      .then((r) => {
        setRequests(r);
        onStatus({ text: "Unblocked. That PC may ask to join again." });
      })
      .catch((e: unknown) => onStatus({ text: message(e), warn: true }));

  const link = requests?.pcs;
  const enabled = remote.pcs === true;
  const asking = requests?.items ?? [];
  const blocked = requests?.blocked ?? [];
  const pcs = (listed?.items ?? []).filter((p) => p.kind === "pc");

  return (
    <>
      <Group
        title="Companion PCs"
        note="Other PCs at home or at work can join this Thrall. Each one is someone's: they get their own chats, and their work happens on their own PC. Everything between the PCs is encrypted."
      >
        <Row label="Let PCs on this network connect" hint="A PC asks to join and you choose. Thrall stays running while this is on">
          <Toggle k="remote.pcs" v={remote.pcs} label="Let PCs on this network connect" onSave={onSave} />
        </Row>
        <Row
          label="This PC"
          hint={link?.listening ? "Companions on this network find it by itself; over Tailscale, type this PC's name" : enabled ? "Starting…" : "Off"}
        >
          <span className={`th-badge${link?.listening ? " is-ok" : ""}`}>
            {link?.listening ? `${link.name} · port ${link.port}` : "Not listening"}
          </span>
        </Row>
      </Group>
      {asking.length > 0 ? (
        <Group title="Asking to join">
          {asking.map((r) => (
            <PcAsking key={r.request} engine={engine} request={r} people={people} onDone={setRequests} onStatus={onStatus} />
          ))}
        </Group>
      ) : null}
      <Group title="Joined PCs">
        {pcs.length === 0 ? (
          <p className="th-set-empty">None yet.</p>
        ) : (
          pcs.map((p) => (
            <Row
              key={p.phone}
              label={p.name}
              hint={[`Joined ${when(p.added)}`, p.last_seen ? `last connected ${when(p.last_seen)}` : "not connected since"].join(" · ")}
            >
              {p.connected ? <span className="th-badge is-ok">Connected</span> : null}
              {people.length > 1 ? (
                <select className="th-input" aria-label={`Whose ${p.name} is`} value={p.person || owner} onChange={(e) => void assign(p.phone, e.target.value)}>
                  {people.map((who) => (
                    <option key={who.person} value={who.person}>
                      {who.name}'s
                    </option>
                  ))}
                </select>
              ) : null}
              {removing === p.phone ? (
                <>
                  <button type="button" className="th-btn th-btn-danger" onClick={() => remove(p.phone)}>
                    Remove
                  </button>
                  <button type="button" className="th-btn th-btn-quiet" onClick={() => setRemoving(null)}>
                    Keep
                  </button>
                </>
              ) : (
                <button type="button" className="th-btn th-btn-quiet" onClick={() => setRemoving(p.phone)}>
                  Remove…
                </button>
              )}
            </Row>
          ))
        )}
      </Group>
      {blocked.length > 0 ? (
        <Group title="Blocked PCs" note="These can't ask to join.">
          {blocked.map((b) => (
            <Row key={b.blocked} label={b.name} hint={`Blocked ${when(b.at)}`}>
              <button type="button" className="th-btn th-btn-quiet" onClick={() => void unblock(b.blocked)}>
                Unblock
              </button>
            </Row>
          ))}
        </Group>
      ) : null}
    </>
  );
}
