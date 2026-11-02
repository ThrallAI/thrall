// Signing this browser in as a person. A sign-in link is the web app's address with
// #signin=<code>: the owner makes one for someone in Settings → People, the desktop app makes one
// for its own window, and `thrall open` makes one for the browser. Opening it trades the code for
// this browser's own device token, kept in localStorage and shown in hello.

import { CLIENT_KIND, DESKTOP_DEVICE_NAME, DEVICE_TOKEN_KEY, SIGNIN_HASH, SIGNIN_PATH } from "./constants";

/** This browser's device token, if it was signed in with a link. */
export function storedToken(): string | null {
  try {
    return localStorage.getItem(DEVICE_TOKEN_KEY);
  } catch {
    return null;
  }
}

/** Forget this browser's sign-in (it was signed out). */
export function forgetDevice(): void {
  try {
    localStorage.removeItem(DEVICE_TOKEN_KEY);
  } catch {
    // Nothing kept, nothing to forget.
  }
}

/** A short name for this browser ("Edge on Windows"), so its person can tell their devices apart. */
export function deviceName(agent = navigator.userAgent): string {
  const browser = /Edg\//.test(agent)
    ? "Edge"
    : /OPR\//.test(agent)
      ? "Opera"
      : /Firefox\//.test(agent)
        ? "Firefox"
        : /Chrome\//.test(agent)
          ? "Chrome"
          : /Safari\//.test(agent)
            ? "Safari"
            : "A browser";
  const system = /Windows/.test(agent)
    ? "Windows"
    : /Android/.test(agent)
      ? "Android"
      : /iPhone|iPad/.test(agent)
        ? "iOS"
        : /Mac OS X/.test(agent)
          ? "macOS"
          : /Linux/.test(agent)
            ? "Linux"
            : "";
  return system ? `${browser} on ${system}` : browser;
}

/**
 * If this page was opened from a sign-in link, trade its code for a device token and take the code
 * out of the address. Resolves with why it failed, or null (signed in, or there was no link).
 */
export async function finishSignIn(): Promise<string | null> {
  if (!location.hash.startsWith(SIGNIN_HASH)) return null;
  const code = decodeURIComponent(location.hash.slice(SIGNIN_HASH.length));
  // The code is single-use; keep it out of the address bar and history either way.
  history.replaceState(null, "", location.pathname + location.search);
  // The desktop app brings a fresh link every time it starts; its window keeps the sign-in it
  // already has rather than adding a new device each time.
  if (CLIENT_KIND === "desktop" && storedToken()) return null;
  try {
    const res = await fetch(SIGNIN_PATH, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ code, name: CLIENT_KIND === "desktop" ? DESKTOP_DEVICE_NAME : deviceName() }),
    });
    const body = (await res.json().catch(() => ({}))) as { token?: string; error?: string };
    if (!res.ok || !body.token) return body.error ?? "Couldn't sign in. Ask for a new link.";
    localStorage.setItem(DEVICE_TOKEN_KEY, body.token);
    return null;
  } catch {
    return "Couldn't reach Thrall to sign in. Try the link again.";
  }
}
