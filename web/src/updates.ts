// "Check for updates": asks GitHub for the newest published release and compares versions.
// Nothing about the person or their sessions is sent; it runs only when they ask.

import { RELEASES_API } from "./constants";

/** A version's numbers (`v0.1.0-preview` → [0, 1, 0]); a pre-release tag doesn't count. */
function numbers(version: string): number[] {
  const plain = version.trim().replace(/^v/i, "").split(/[-+]/)[0] ?? "";
  return plain.split(".").map((n) => Number.parseInt(n, 10) || 0);
}

/** Whether `candidate` is a newer version than `current`. */
export function isNewer(candidate: string, current: string): boolean {
  const a = numbers(candidate);
  const b = numbers(current);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const d = (a[i] ?? 0) - (b[i] ?? 0);
    if (d !== 0) return d > 0;
  }
  return false;
}

/** The newest release's version when it's newer than `current`, else null. */
export async function newerRelease(current: string): Promise<string | null> {
  const r = await fetch(RELEASES_API, { headers: { Accept: "application/vnd.github+json" } });
  if (!r.ok) throw new Error(`GitHub answered ${r.status}`);
  const release = (await r.json()) as { tag_name?: string };
  const tag = release.tag_name ?? "";
  return tag && isNewer(tag, current) ? tag.replace(/^v/i, "") : null;
}
