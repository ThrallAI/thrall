// Typed `/name` commands this app can't run: the terminal app's own ones.

import type { CommandItem } from "./protocol";
import { TERMINAL_ONLY } from "./constants";

/**
 * What to say for a command only the terminal app has (`/settings`, `/quit`…), instead of the
 * engine's "Unknown command". Null for anything else, or a name the engine knows (a skill too).
 */
export function terminalOnly(name: string, known: readonly CommandItem[]): string | null {
  if (known.some((c) => c.name.toLowerCase() === name)) return null;
  if (!Object.prototype.hasOwnProperty.call(TERMINAL_ONLY, name)) return null;
  return `/${name} is a terminal command. ${TERMINAL_ONLY[name]}`;
}
