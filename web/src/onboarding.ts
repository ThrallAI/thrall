// First steps for each new person: a welcome, then a one-minute tour of the real screen. Both
// can be switched off in Settings and the tour taken again from the command palette. Kept in
// this browser (a convenience: nothing breaks if it's forgotten).

import { TOUR_OFF_KEY, WELCOMED_KEY_PREFIX } from "./constants";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string | null): void {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // Only a convenience.
  }
}

/** Whether new people get the welcome and the tour offered (Settings switch). */
export function tourOn(): boolean {
  return read(TOUR_OFF_KEY) !== "1";
}

export function setTourOn(on: boolean): void {
  write(TOUR_OFF_KEY, on ? null : "1");
}

/** Whether this person has had the welcome in this browser. */
export function welcomed(person: string): boolean {
  return read(WELCOMED_KEY_PREFIX + person) === "1";
}

export function markWelcomed(person: string): void {
  write(WELCOMED_KEY_PREFIX + person, "1");
}

/** One stop of the tour: what it points at (a `data-tour` name; none: the middle of the screen). */
export interface TourStep {
  target?: string;
  title: string;
  body: string;
}

export const TOUR: TourStep[] = [
  {
    target: "composer",
    title: "Ask here",
    body: "Type what you want done, in plain words. Thrall can read and change the files in this session's folder, run commands and use the web. Start with / for commands.",
  },
  {
    target: "mode",
    title: "How much Thrall does on its own",
    body: "Ask (the default) checks with you before every edit and command. Auto-accept edits lets file edits go ahead but still asks before commands. Plan only reads and plans. Shift+Tab switches between them.",
  },
  {
    title: "You approve the changes",
    body: "When Thrall wants to change something, a card asks first: Allow, Always (for this session) or No. Risky commands are never covered by Always, and restarts always ask.",
  },
  {
    target: "new",
    title: "Sessions",
    body: "Each session is one conversation with its own folder and history. Start a new one for a new job, and keep them together in groups.",
  },
  {
    target: "cost",
    title: "What it costs",
    body: "This is what the session has cost on your own OpenRouter account so far. Thrall uses your key; nothing goes anywhere else.",
  },
  {
    target: "settings",
    title: "Settings",
    body: "The model, people in your household, pairing your phone, and starting Thrall when you sign in. Take this tour again any time: Ctrl+K, then \"Take the tour\".",
  },
];
