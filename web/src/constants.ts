// Named constants for the web app. Every limit, timeout and repeated string lives here.

/** Protocol major this app speaks. */
export const PROTOCOL_VERSION = 1;

/** How the app introduces itself to the engine. */
export const CLIENT_NAME = "Thrall Web";

/** This client's kind, as `answered_by` names it: the desktop app opens the page with
 * `?client=desktop`; anywhere else it's a browser. */
export const CLIENT_KIND: "web" | "desktop" =
  typeof location !== "undefined" && new URLSearchParams(location.search).get("client") === "desktop"
    ? "desktop"
    : "web";

/** How many archived sessions the sidebar lists. */
export const ARCHIVED_SESSIONS = 100;

/** Upload path on the engine. */
export const FILES_PATH = "/v1/files";

/** Largest file the engine accepts (25 MiB). */
export const MAX_UPLOAD_BYTES = 25 * 1024 * 1024;

/** WebSocket endpoint path on the engine. */
export const WS_PATH = "/v1/ws";

/** Heartbeat interval (protocol §2). */
export const PING_INTERVAL_MS = 20_000;

/** How long a request may wait for its reply. */
export const REQUEST_TIMEOUT_MS = 30_000;

/** Reconnect delays: first try, then doubling up to the cap. */
export const RECONNECT_FIRST_MS = 500;
export const RECONNECT_MAX_MS = 5_000;

/** Sessions shown in the sidebar. */
export const SIDEBAR_SESSIONS = 50;

/** How often the sidebar refreshes (other sessions' state isn't pushed). */
export const SIDEBAR_REFRESH_MS = 15_000;

/** How long the living mark shows Done before going back to Ready. */
export const DONE_HOLD_MS = 1_600;

/** Most results the Ctrl+K palette shows. */
export const PALETTE_MAX = 12;

/** Most slash-command suggestions shown at once. */
export const SUGGESTIONS_MAX = 8;

/** Tallest the composer grows, in pixels. */
export const COMPOSER_MAX_HEIGHT = 240;

/** Key holding the folder new sessions start in (a per-browser convenience). */
export const WORKSPACE_KEY = "thrall.workspace";

/** Key holding which folder groups are collapsed in the sidebar. */
export const COLLAPSED_KEY = "thrall.collapsed";

/** Key holding this browser's device token, once it signed in with a sign-in link. */
export const DEVICE_TOKEN_KEY = "thrall.device";

/** Where a browser trades a sign-in code for its device token. */
export const SIGNIN_PATH = "/v1/signin";

/** A sign-in link is the web app's address with this, then the code. */
export const SIGNIN_HASH = "#signin=";

/** What the desktop app's window is called in a person's list of signed-in devices. */
export const DESKTOP_DEVICE_NAME = "Thrall desktop app";

/** Key holding the theme choice (system, dark or light). */
export const THEME_KEY = "thrall.theme";

/** Placeholder in the message box. */
export const COMPOSER_PLACEHOLDER = "Message Thrall · / for commands";

/** Shown on an empty session. */
export const GREETING = "What are we working on?";
/** What an empty Home page says. */
export const HOME_GREETING = "What's on your mind?";

/** Commands this app handles itself; the engine's come from `command.list`. */
export const CLIENT_COMMANDS: ReadonlyArray<{ name: string; summary: string }> = [
  { name: "new", summary: "Start a new session" },
];

/** Permission modes in Shift+Tab order (full auto is never reached by toggling). */
export const MODE_CYCLE = ["ask", "auto_edits", "plan"] as const;

/** Plain names for permission modes. */
export const MODE_LABELS: Record<string, string> = {
  ask: "Ask",
  auto_edits: "Auto-accept edits",
  plan: "Plan",
  full_auto: "Full auto",
};

/** What each mode lets Thrall do without asking, shown when it's picked (the engine's wording). */
export const MODE_COVERS: Record<string, string> = {
  ask: "Thrall asks before every file edit and every command.",
  auto_edits: "File edits in the folder go ahead without asking; commands still ask.",
  plan: "Thrall only reads and plans; nothing changes until you approve a plan.",
  full_auto:
    "Edits and commands go ahead without asking, risky ones too (restarts, sending and deleting outside the folder still ask).",
};

/** The tour's note: its width (as in the CSS) and the gap it keeps from the window's edges. */
export const TOUR_NOTE_PX = 360;
export const TOUR_EDGE_PX = 16;

/** Remembered in this browser: the welcome and tour are switched off for new people here. */
export const TOUR_OFF_KEY = "thrall.tourOff";

/** Remembered in this browser, per person (add the person's id): they've had the welcome. */
export const WELCOMED_KEY_PREFIX = "thrall.welcomed.";

/** Where new versions are published (GitHub's releases for the repository). */
export const RELEASES_API = "https://api.github.com/repos/ThrallAI/thrall/releases/latest";

/** The releases page people download new versions from. */
export const RELEASES_PAGE = "https://github.com/ThrallAI/thrall/releases";

/** The start-at-sign-in setting's name when an older engine doesn't say (it was Windows-only). */
export const STARTUP_LABEL = "Start with Windows";

/** How long Full auto lasts (`ModeLimit.until`). */
export const MODE_UNTIL_TASK_END = "task_end";
export const MODE_UNTIL_TIME = "time";
export const MODE_UNTIL_MANUAL = "manual";

/** Full auto's lengths in the mode chip's menu. */
export const FULL_AUTO_LENGTHS: { label: string; until: string; minutes?: number }[] = [
  { label: "Until this task ends", until: MODE_UNTIL_TASK_END },
  { label: "For 30 minutes", until: MODE_UNTIL_TIME, minutes: 30 },
  { label: "For 1 hour", until: MODE_UNTIL_TIME, minutes: 60 },
  { label: "For 2 hours", until: MODE_UNTIL_TIME, minutes: 120 },
  { label: "Until I switch it off", until: MODE_UNTIL_MANUAL },
];

/** Who answered, in plain words. */
export const CLIENT_LABELS: Record<string, string> = {
  tui: "in the terminal",
  web: "in a browser",
  desktop: "in the desktop app",
  android: "on your phone",
  telegram: "in Telegram",
  headless: "by a script",
};

/** Sections of the settings screen, in order. */
export const SETTINGS_SECTIONS = [
  { id: "general", label: "General" },
  { id: "personality", label: "Personality" },
  { id: "models", label: "Models" },
  { id: "providers", label: "Providers" },
  { id: "safety", label: "Permissions & safety" },
  { id: "memory", label: "Learning" },
  { id: "writing", label: "Writing" },
  { id: "tools", label: "Tools & integrations" },
  { id: "people", label: "People" },
  { id: "phones", label: "Phones" },
  { id: "pcs", label: "PCs" },
  { id: "keys", label: "Keys" },
  { id: "advanced", label: "Advanced" },
] as const;

/** One settings section's id. */
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]["id"];

/** What a learned skill's safety rating is called on screen. */
export const RATING_LABELS: Readonly<Record<string, string>> = {
  low: "Low risk",
  medium: "Medium risk",
  high: "High risk",
};

/** The personality written in the person's own words (`personality.set {preset}`). */
export const PERSONALITY_CUSTOM = "custom";

/** Longest own-words personality the engine keeps. */
export const PERSONALITY_MAX_CHARS = 1500;

/** Longest description a skill may have (the engine's SKILL_DESCRIPTION_MAX_CHARS). */
export const SKILL_DESCRIPTION_MAX_CHARS = 300;

/** Most helpers that may work at once (the engine's MAX_HELPERS_AT_ONCE). */
export const MAX_HELPERS_AT_ONCE = 8;

/** Log detail levels, least to most. */
export const LOG_LEVELS = ["error", "warn", "info", "debug", "trace"] as const;

/** When Home starts a fresh page (config `home.fresh_start`). */
export const FRESH_STARTS = ["daily", "weekly", "never"] as const;

/** Humaniser levels (config `writing.humanise`), least to most. */
export const HUMANISE_LEVELS = ["off", "light", "full"] as const;

/** `answered_by` on a question card nobody answered in time. */
export const QUESTION_TIMEOUT = "timeout";

/** When Thrall asks instead of deciding (config `agent.questions`). */
export const QUESTION_MODES = ["never", "when_it_matters", "when_unsure"] as const;

/** What each question mode is called on the settings screen. */
export const QUESTION_MODE_LABELS: Readonly<Record<string, string>> = {
  never: "Never: decide and note it",
  when_it_matters: "Only when it matters",
  when_unsure: "When it's unsure",
};

/** A company or server that can do Thrall's jobs. `prefix` starts its model ids (OpenRouter's have
 * none); `key` is its keychain name (`thrall key set <key>`), absent when it needs none. */
export interface Supplier {
  id: string;
  name: string;
  prefix: string;
  key?: string;
  about: string;
}

/** Every supplier, in the order the Providers page lists them. */
export const SUPPLIERS: readonly Supplier[] = [
  { id: "openrouter", name: "OpenRouter", prefix: "", key: "openrouter", about: "One key for models from many companies" },
  { id: "chatgpt", name: "ChatGPT plan", prefix: "chatgpt/", about: "Your ChatGPT Plus or Pro plan (sign in under Models)" },
  { id: "openai", name: "OpenAI", prefix: "direct/openai/", key: "openai", about: "OpenAI directly, with your own OpenAI key" },
  { id: "anthropic", name: "Anthropic", prefix: "direct/anthropic/", key: "anthropic", about: "Anthropic's models directly, with your own Anthropic key" },
  { id: "google", name: "Google", prefix: "direct/google/", key: "google", about: "Gemini models directly, with your own Google AI key" },
  { id: "elevenlabs", name: "ElevenLabs", prefix: "direct/elevenlabs/", key: "elevenlabs", about: "Natural voices; the voice is an ElevenLabs voice id" },
  { id: "local", name: "This computer (local)", prefix: "local/", about: "Ollama or LM Studio: free, and nothing leaves your network" },
  { id: "program", name: "Programs on this computer", prefix: "program/", about: "whisper.cpp, Piper or a Stable Diffusion web UI you've installed: free" },
  { id: "fal", name: "fal.ai", prefix: "", key: "fal", about: "Makes videos" },
];

/** Thrall's jobs, the setting holding each one's model, and the suppliers that can do it. */
export const JOBS: ReadonlyArray<{ key: string; label: string; suppliers: readonly string[] }> = [
  { key: "provider.model", label: "Chat", suppliers: ["openrouter", "chatgpt", "openai", "anthropic", "google", "local"] },
  { key: "provider.vision_model", label: "Reads pictures", suppliers: ["openrouter", "openai", "anthropic", "google", "local"] },
  { key: "provider.image_model", label: "Makes pictures", suppliers: ["openrouter", "program"] },
  { key: "provider.speech_model", label: "Speaks", suppliers: ["openrouter", "openai", "elevenlabs", "program"] },
  { key: "provider.transcribe_model", label: "Transcribes", suppliers: ["openrouter", "openai", "google", "program"] },
  { key: "memory.embedding_model", label: "Memory search", suppliers: ["openrouter", "openai", "google"] },
];

/** Which setting each job `local.found` reports on is saved to when its model is picked. */
export const LOCAL_JOB_SETTINGS: Readonly<Record<string, string>> = {
  chat: "provider.model",
  transcribe: "provider.transcribe_model",
  speak: "provider.speech_model",
  pictures: "provider.image_model",
};

/** What each of those jobs is called on screen. */
export const LOCAL_JOB_LABELS: Readonly<Record<string, string>> = {
  chat: "Chat",
  transcribe: "Transcribes",
  speak: "Speaks",
  pictures: "Makes pictures",
};

/** How often the settings screen checks whether a ChatGPT sign-in has landed. */
export const SIGN_IN_POLL_MS = 2000;

/** How often Settings → Phones re-reads the phone list while a pairing code is showing. */
export const PHONE_POLL_MS = 2000;

/** How long the line saying what came of a PC asking to join stays up. */
export const ASK_NOTE_MS = 6000;

/** How long it keeps checking (the engine waits five minutes for the browser). */
export const SIGN_IN_WATCH_MS = 5 * 60 * 1000 + 10_000;

/** The `agent` of a check-in's `agent.completed` (its message is already in Home). */
export const CHECK_IN_AGENT = "check-in";

/** The function the desktop app calls to open the session a clicked notification is about. */
export const OPEN_SESSION_JS = "thrallOpen";
