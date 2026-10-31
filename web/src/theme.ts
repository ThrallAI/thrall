// The person's theme choice: follow the system, or always dark, or always light. Kept in this
// browser (or the desktop app); applied as data-theme on <html>, which tokens.css reads.

import { THEME_KEY } from "./constants";

export type Theme = "system" | "dark" | "light";

const ORDER: Theme[] = ["system", "dark", "light"];

export const THEME_LABELS: Record<Theme, string> = {
  system: "System",
  dark: "Dark",
  light: "Light",
};

export function readTheme(): Theme {
  try {
    const saved = localStorage.getItem(THEME_KEY);
    return saved === "dark" || saved === "light" ? saved : "system";
  } catch {
    return "system";
  }
}

export function applyTheme(theme: Theme): void {
  const root = document.documentElement;
  if (theme === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", theme);
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch {
    // Remembering the choice is only a convenience.
  }
}

export function nextTheme(theme: Theme): Theme {
  return ORDER[(ORDER.indexOf(theme) + 1) % ORDER.length] ?? "system";
}

/** What the screen shows now: the choice, or Windows' own setting while following the system. */
export function shownTheme(theme: Theme): "dark" | "light" {
  if (theme !== "system") return theme;
  return matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

/** The sidebar switch: the opposite of what's showing, kept from then on. */
export function flipTheme(theme: Theme): Theme {
  return shownTheme(theme) === "dark" ? "light" : "dark";
}
