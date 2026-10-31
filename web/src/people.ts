// Small helpers for Settings → People: the month spending is counted from, sign-in link
// addresses, and how money reads.

import { SIGNIN_HASH } from "./constants";

/** Midnight on the first of `now`'s month, local time, as Unix ms. */
export function monthStart(now: Date): number {
  return new Date(now.getFullYear(), now.getMonth(), 1).getTime();
}

/** The address a sign-in code is opened at: this web app's, with the code after the hash. */
export function signInLink(page: { origin: string; pathname: string }, code: string): string {
  return `${page.origin}${page.pathname}${SIGNIN_HASH}${code}`;
}

/** Dollars, to the cent (a hint of anything spent under a cent). */
export function dollars(usd: number): string {
  if (usd > 0 && usd < 0.01) return "under $0.01";
  return `$${usd.toFixed(2)}`;
}
