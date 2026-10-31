// Settings → Phones: remote access for paired phones, adding one (for someone) with a QR code, and
// the list of phones with whose each is and Remove. The engine does the pairing and the
// encryption; this only shows it.

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import type { Engine, ReplyOf } from "../engine";
import { PHONE_POLL_MS } from "../constants";
import { CopyButton } from "./CopyButton";

type Listed = ReplyOf<"phone.listed">;
type Pairing = ReplyOf<"phone.pairing">;
type Person = ReplyOf<"people.listed">["items"][number];
type Save = (key: string, value: boolean | number | string) => void;
type Status = (s: { text: string; warn?: boolean }) => void;

function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/** The QR as an SVG: dark modules on white, with the quiet zone scanners need, in either theme. */
function Qr({ rows }: { rows: string[] }) {
  const size = rows.length;
  const pad = 4;
  let d = "";
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length; x++) if (row[x] === "1") d += `M${x + pad} ${y + pad}h1v1h-1z`;
  });
  return (
    <svg className="th-qr" viewBox={`0 0 ${size + pad * 2} ${size + pad * 2}`} role="img" aria-label="Pairing QR code" shapeRendering="crispEdges">
      <rect width="100%" height="100%" fill="#ffffff" />
      <path d={d} fill="#000000" />
    </svg>
  );
}

export function Phones({
  engine,
  remote,
  onSave,
  onStatus,
  Row,
  Group,
  Toggle,
  Field,
}: {
  engine: Engine | null;
  /** `[remote]` from the config. */
  remote: Record<string, unknown>;
  onSave: Save;
  onStatus: Status;
  Row: (p: { label: string; hint?: string; children: ReactNode }) => ReactNode;
  Group: (p: { title: string; note?: string; children: ReactNode }) => ReactNode;
  Toggle: (p: { k: string; v: unknown; label: string; onSave: Save }) => ReactNode;
  Field: (p: { k: string; v: unknown; label: string; onSave: Save; mono?: boolean; allowEmpty?: boolean }) => ReactNode;
}) {
  const [listed, setListed] = useState<Listed | null>(null);
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [now, setNow] = useState(Date.now());
  const [removing, setRemoving] = useState<string | null>(null);
  const [people, setPeople] = useState<Person[]>([]);
  const [forWhom, setForWhom] = useState("");
  const paired = useRef<Set<string>>(new Set());

  useEffect(() => {
    engine
      ?.request<"people.listed">({ type: "people.list" })
      .then((r) => setPeople(r.items))
      .catch(() => setPeople([]));
  }, [engine]);
  const owner = people.find((p) => p.role === "owner")?.person ?? "";
  const nameOf = (person: string | undefined) => people.find((p) => p.person === (person || owner))?.name;

  const load = useCallback(
    () =>
      engine
        ?.request<"phone.listed">({ type: "phone.list" })
        .then((r) => {
          setListed(r);
          return r;
        })
        .catch((e: unknown) => {
          onStatus({ text: e instanceof Error ? e.message : String(e), warn: true });
          return undefined;
        }),
    [engine, onStatus],
  );

  // Reload when remote access or its address changes (the listener follows within seconds).
  useEffect(() => {
    void load();
    const t = window.setTimeout(() => void load(), PHONE_POLL_MS);
    return () => window.clearTimeout(t);
  }, [load, remote.enabled, remote.address]);

  // While a QR is showing: count down, and close it when a new phone appears.
  useEffect(() => {
    if (!pairing) return;
    const tick = window.setInterval(() => setNow(Date.now()), 1000);
    const watch = window.setInterval(() => {
      void load()?.then((r) => {
        const fresh = r?.items.find((p) => !paired.current.has(p.phone));
        if (fresh) {
          setPairing(null);
          const whose = nameOf(fresh.person);
          onStatus({ text: `Paired ${fresh.name}${whose ? ` for ${whose}` : ""}. It can reach Thrall now.` });
        }
      });
    }, PHONE_POLL_MS);
    return () => {
      window.clearInterval(tick);
      window.clearInterval(watch);
    };
  }, [pairing, load, onStatus]);

  useEffect(() => {
    if (pairing && now >= pairing.expires) {
      setPairing(null);
      onStatus({ text: "The pairing code ran out. Add the phone again for a new one.", warn: true });
    }
  }, [now, pairing, onStatus]);

  const addPhone = () => {
    paired.current = new Set((listed?.items ?? []).map((p) => p.phone));
    engine
      ?.request<"phone.pairing">({ type: "phone.pair", person: forWhom && forWhom !== owner ? forWhom : undefined })
      .then((r) => {
        setNow(Date.now());
        setPairing(r);
        onStatus({ text: "Scan the code with the Thrall app on your phone." });
      })
      .catch((e: unknown) => onStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  const assign = (phone: string, person: string) => {
    engine
      ?.request<"phone.listed">({ type: "phone.assign", phone, person })
      .then((r) => {
        setListed(r);
        onStatus({ text: `That phone is ${nameOf(person) ?? "theirs"}'s now. It reconnects as them.` });
      })
      .catch((e: unknown) => onStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  const remove = (phone: string) => {
    setRemoving(null);
    engine
      ?.request<"phone.listed">({ type: "phone.remove", phone })
      .then((r) => {
        setListed(r);
        onStatus({ text: "Removed. That phone can't reach Thrall any more." });
      })
      .catch((e: unknown) => onStatus({ text: e instanceof Error ? e.message : String(e), warn: true }));
  };

  const link = listed?.remote;
  const enabled = remote.enabled === true;
  const address = typeof remote.address === "string" ? remote.address : "";
  const left = pairing ? Math.max(0, Math.round((pairing.expires - now) / 1000)) : 0;
  const phones = listed?.items ?? [];

  return (
    <>
      <Group
        title="Remote access"
        note="Paired phones reach Thrall through a tunnel such as Tailscale Funnel, with no VPN on the phone. Everything on the link is encrypted end to end between the phone and this computer, and only phones you add here get in."
      >
        <Row label="Let paired phones connect" hint="Thrall stays running while this is on">
          <Toggle k="remote.enabled" v={remote.enabled} label="Let paired phones connect" onSave={onSave} />
        </Row>
        <Row label="Address" hint="The public address phones use, like https://your-pc.your-tailnet.ts.net">
          <Field k="remote.address" v={address} label="Phone address" onSave={onSave} mono allowEmpty />
        </Row>
        <Row
          label="Phone link"
          hint={
            link?.listening
              ? `Point the tunnel at port ${link.port}: tailscale funnel --bg ${link.port}`
              : enabled
                ? "Starting…"
                : "Off"
          }
        >
          <span className={`th-badge${link?.listening ? " is-ok" : ""}`}>
            {link?.listening ? `Listening on 127.0.0.1:${link.port}` : "Not listening"}
          </span>
        </Row>
        <Row
          label="Add a phone"
          hint={!enabled ? "Turn on remote access first" : !address ? "Set the address first" : "Shows a one-time code for the Thrall app to scan"}
        >
          {pairing ? (
            <button type="button" className="th-btn th-btn-quiet" onClick={() => setPairing(null)}>
              Cancel
            </button>
          ) : (
            <>
              {people.length > 1 ? (
                <select className="th-input" aria-label="Who the phone is for" value={forWhom || owner} onChange={(e) => setForWhom(e.target.value)}>
                  {people.map((p) => (
                    <option key={p.person} value={p.person}>
                      For {p.name}
                    </option>
                  ))}
                </select>
              ) : null}
              <button type="button" className="th-btn th-btn-primary" disabled={!enabled || !address} onClick={addPhone}>
                Add a phone
              </button>
            </>
          )}
        </Row>
        {pairing ? (
          <div className="th-pairing">
            <Qr rows={pairing.qr} />
            <p>
              Open Thrall on your phone and scan this. The code works once and runs out in{" "}
              <strong>
                {Math.floor(left / 60)}:{String(left % 60).padStart(2, "0")}
              </strong>
              . Only share it with the person you're adding: whoever uses it first is paired.
            </p>
            <p>
              Not next to the phone? Copy the link and send it privately; it's pasted in the app's pairing
              screen. <CopyButton text={() => pairing.link} label="Copy link" />
            </p>
          </div>
        ) : null}
      </Group>
      <Group title="Paired phones">
        {phones.length === 0 ? (
          <p className="th-set-empty">None yet.</p>
        ) : (
          phones.map((p) => (
            <Row
              key={p.phone}
              label={p.name}
              hint={[`Added ${when(p.added)}`, p.last_seen ? `last connected ${when(p.last_seen)}` : "not connected since"].join(" · ")}
            >
              {p.connected ? <span className="th-badge is-ok">Connected</span> : null}
              {people.length > 1 ? (
                <select
                  className="th-input"
                  aria-label={`Whose ${p.name} is`}
                  value={p.person || owner}
                  onChange={(e) => assign(p.phone, e.target.value)}
                >
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
    </>
  );
}
