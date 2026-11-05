// The web app: a client of Thrall's one engine. It holds no agent state of its own; it sends
// protocol requests and draws the events that come back.

import { useCallback, useEffect, useLayoutEffect, useReducer, useRef, useState } from "react";

import { CommandPalette, type PaletteItem } from "./components/CommandPalette";
import { Settings } from "./components/Settings";
import { Composer, type ComposerHandle } from "./components/Composer";
import { folderName } from "./components/Sidebar";
import { DecisionCard, type Choice } from "./components/DecisionCard";
import { LivingMark } from "./components/LivingMark";
import { ModeBanner } from "./components/ModeBanner";
import { RewindDialog, type Rewound } from "./components/RewindDialog";
import { Sidebar, type HomePages } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { TaskPanel } from "./components/TaskPanel";
import { TranscriptItem } from "./components/TranscriptItem";
import { FileLoader } from "./components/Attachments";
import {
  ARCHIVED_SESSIONS,
  ASK_NOTE_MS,
  CLIENT_COMMANDS,
  CLIENT_KIND,
  CLIENT_LABELS,
  DONE_HOLD_MS,
  GREETING,
  OPEN_SESSION_JS,
  HOME_GREETING,
  MODE_COVERS,
  MODE_CYCLE,
  MODE_LABELS,
  SIDEBAR_REFRESH_MS,
  SIDEBAR_SESSIONS,
  WORKSPACE_KEY,
} from "./constants";
import { Engine, EngineError, endpoint, type ConnectionState, type Request } from "./engine";
import { NewFolder } from "./components/NewFolder";
import { PcAsking } from "./components/PCs";
import type { CommandItem, EngineInfo, EventFrame, GroupInfo, PcRequestInfo, PersonInfo, UserInfo } from "./protocol";
import { markWelcomed, tourOn, welcomed } from "./onboarding";
import { Tour } from "./components/Tour";
import { Welcome } from "./components/Welcome";
import type { Session } from "./types";
import { applyTheme, flipTheme, readTheme } from "./theme";
import { apply, emptyView, localId, markForgotten, openedView, shownActivity, type Item, type View } from "./transcript";

type Action =
  | { type: "event"; frame: EventFrame }
  | { type: "reset"; view: View }
  | { type: "local"; item: Item }
  | { type: "patch"; patch: Partial<View> }
  | { type: "forgot"; fact: string };

function reducer(view: View, action: Action): View {
  switch (action.type) {
    case "event":
      return apply(view, action.frame, CLIENT_KIND, CLIENT_LABELS);
    case "reset":
      return action.view;
    case "local":
      return { ...view, items: [...view.items, action.item] };
    case "patch":
      return { ...view, ...action.patch };
    case "forgot":
      return markForgotten(view, action.fact);
  }
}

function readFolder(): string {
  try {
    return localStorage.getItem(WORKSPACE_KEY) ?? "";
  } catch {
    return "";
  }
}

function saveFolder(folder: string): void {
  try {
    localStorage.setItem(WORKSPACE_KEY, folder);
  } catch {
    // Remembering the folder is only a convenience.
  }
}

const SESSION_COMMANDS = new Set(["cost", "title", "tasks", "status", "compact"]);
const NEAR_BOTTOM_PX = 80;
const CARD_MARK_PX = 160;
const CONNECTING_MARK_PX = 96;

export function App() {
  const engine = useRef<Engine | null>(null);
  const loadFile = useCallback((file: string) => {
    const e = engine.current;
    if (!e) return Promise.reject(new EngineError("closed", "Not connected to Thrall's engine yet."));
    return e.download(file);
  }, []);  const [conn, setConn] = useState<ConnectionState>("connecting");
  const [info, setInfo] = useState<EngineInfo | null>(null);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [commands, setCommands] = useState<CommandItem[]>([]);
  const [current, setCurrent] = useState<Session | null>(null);
  const [view, dispatch] = useReducer(reducer, undefined, () => emptyView());
  const [folder, setFolder] = useState(readFolder);
  const [editingFolder, setEditingFolder] = useState(false);
  const [makingFolder, setMakingFolder] = useState(false);
  const requestEngine = useCallback((r: Request) => {
    const e = engine.current;
    if (!e) return Promise.reject(new EngineError("closed", "Not connected to Thrall's engine yet."));
    return e.request(r);
  }, []);
  const [editingTitle, setEditingTitle] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [composerEmpty, setComposerEmpty] = useState(true);
  const [, setTick] = useState(0);
  const [theme, setTheme] = useState(readTheme);
  const [unread, setUnread] = useState<Set<string>>(() => new Set());
  const [groups, setGroups] = useState<GroupInfo[]>([]);
  const [palette, setPalette] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // PCs asking to join (the owner, on this computer, sees them wherever they are).
  const [asks, setAsks] = useState<PcRequestInfo[]>([]);
  const [askPeople, setAskPeople] = useState<PersonInfo[]>([]);
  const [askNote, setAskNote] = useState<string | null>(null);
  // Who's signed in, and their first steps: the welcome, then (if they want it) the tour.
  const [me, setMe] = useState<UserInfo | null>(null);
  const [welcoming, setWelcoming] = useState(false);
  const [touring, setTouring] = useState(false);
  const [editing, setEditing] = useState(false);
  const [home, setHome] = useState<HomePages | null>(null);
  // The person is in Home (not an earlier page): when a fresh page starts, they move to it.
  const inHome = useRef(false);
  const openSessionRef = useRef<(id: string) => Promise<void>>(async () => undefined);
  const editingRef = useRef(false);
  editingRef.current = editing;

  useEffect(() => applyTheme(theme), [theme]);

  const currentId = useRef<string | null>(null);
  const viewRef = useRef(view);
  viewRef.current = view;
  const composer = useRef<ComposerHandle>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);

  const note = useCallback((text: string, level: "info" | "warning" = "info") => {
    dispatch({ type: "local", item: { kind: "notice", id: localId("n"), level, text } });
  }, []);
  const fail = useCallback((e: unknown) => {
    const text = e instanceof Error ? e.message : String(e);
    dispatch({ type: "local", item: { kind: "error", id: localId("e"), text } });
  }, []);

  const refreshSessions = useCallback(async () => {
    const e = engine.current;
    if (!e) return;
    try {
      const reply = await e.request<"session.listed">({ type: "session.list", limit: SIDEBAR_SESSIONS });
      setSessions(reply.items);
      // The engine names a session after its first turn; pick that up for the header too.
      const mine = reply.items.find((s) => s.session === currentId.current);
      if (mine && mine.title && mine.title !== viewRef.current.title) {
        dispatch({ type: "patch", patch: { title: mine.title } });
      }
      setFolder((f) => f || reply.items.find((s) => s.workspace)?.workspace || "");
    } catch {
      // The sidebar catches up on the next refresh.
    }
  }, []);

  // Home's current page (a fresh one starts each day) and its earlier pages.
  const refreshHome = useCallback(async () => {
    const e = engine.current;
    if (!e) return;
    try {
      const reply = await e.request<"home.shown">({ type: "home.get" });
      setHome({ page: reply.page, earlier: reply.earlier });
      const id = currentId.current;
      if (inHome.current && id && id !== reply.page.session && !viewRef.current.busy) {
        void openSessionRef.current(reply.page.session);
      }
    } catch {
      // Home catches up on the next refresh.
    }
  }, []);

  // The slash commands, with the person's skills (which change when one is saved or removed).
  const refreshCommands = useCallback(() => {
    engine.current
      ?.request<"command.listed">({ type: "command.list" })
      .then((r) => setCommands(r.items))
      .catch(() => undefined);
  }, []);

  const refreshAsks = useCallback(() => {
    const e = engine.current;
    if (!e) return;
    e.request<"pc.requests">({ type: "pc.requests" })
      .then((r) => {
        setAsks(r.items);
        if (r.items.length > 0)
          e.request<"people.listed">({ type: "people.list" })
            .then((p) => setAskPeople(p.items))
            .catch(() => undefined);
      })
      // Only the owner on this computer is asked.
      .catch(() => setAsks([]));
  }, []);

  useEffect(() => {
    if (!askNote) return;
    const t = window.setTimeout(() => setAskNote(null), ASK_NOTE_MS);
    return () => window.clearTimeout(t);
  }, [askNote]);

  // Connect once; reconnects reopen the current session from the last event seen.
  useEffect(() => {
    const e = new Engine(endpoint(), {
      onEvent(frame) {
        if (frame.type === "pc.asked") {
          refreshAsks();
          return;
        }
        // Every session's row follows its state (working, needs you, idle), the open one too.
        if (frame.type === "status") {
          setSessions((list) => list.map((s) => (s.session === frame.session ? { ...s, state: frame.state } : s)));
          setHome((h) =>
            h && h.page.session === frame.session ? { ...h, page: { ...h.page, state: frame.state } } : h,
          );
        }
        // A turn may have saved a new skill: its /name joins the list.
        if (frame.type === "turn.completed") refreshCommands();
        if (frame.session === currentId.current) {
          dispatch({ type: "event", frame });
          if (frame.type === "session.updated" || (frame.type === "status" && frame.state === "idle"))
            void refreshSessions();
        } else {
          // Other sessions (the engine sends them because this page watches them all): keep the
          // sidebar live and mark sessions with news you haven't seen.
          if (frame.type === "session.updated" && (frame.title || frame.workspace)) {
            const { title, workspace } = frame;
            setSessions((list) =>
              list.map((s) =>
                s.session === frame.session
                  ? { ...s, title: title ?? s.title, workspace: workspace ?? s.workspace }
                  : s,
              ),
            );
          } else if (frame.type === "turn.completed" || frame.type === "agent.completed") {
            setUnread((u) => new Set(u).add(frame.session));
            void refreshSessions();
          } else if (frame.type === "turn.started") {
            void refreshSessions();
          }
        }
      },
      onState(state, engineInfo, user) {
        setConn(state);
        if (state !== "open" || !engineInfo) return;
        setInfo(engineInfo);
        if (user) {
          setMe(user);
          if (tourOn() && !welcomed(user.id)) setWelcoming(true);
        }
        if (!viewRef.current.model) dispatch({ type: "patch", patch: { model: engineInfo.model } });
        refreshCommands();
        refreshAsks();
        e.request({ type: "session.watch_all" }).catch(() => undefined);
        e.request<"group.listed">({ type: "group.list" })
          .then((r) => setGroups(r.items))
          .catch(() => undefined);
        void refreshSessions();
        void refreshHome();
        const id = currentId.current;
        if (id) {
          e.request({ type: "session.open", session: id, after_seq: viewRef.current.lastSeq }).catch(() => undefined);
        }
      },
    });
    engine.current = e;
    e.start();
    const timer = setInterval(() => {
      void refreshSessions();
      void refreshHome();
    }, SIDEBAR_REFRESH_MS);
    // Coming back to the window may be a new day: Home moves to a fresh page.
    const onVisible = () => {
      if (document.visibilityState === "visible") void refreshHome();
    };
    document.addEventListener("visibilitychange", onVisible);
    window.addEventListener("focus", onVisible);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
      window.removeEventListener("focus", onVisible);
      e.stop();
    };
  }, [refreshSessions, refreshHome]);

  // Done shows for a moment, then the mark rests.
  useEffect(() => {
    if (view.activity !== "done") return;
    const t = setTimeout(() => setTick((n) => n + 1), DONE_HOLD_MS + 50);
    return () => clearTimeout(t);
  }, [view.activity, view.activityAt]);

  // Follow the conversation unless the person has scrolled up.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [view.items, view.card]);

  // Ctrl+O shows or hides every step's details, as in the terminal.
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      if (ev.ctrlKey && ev.key.toLowerCase() === "o") {
        setExpanded((x) => !x);
        ev.preventDefault();
      }
      // Ctrl+K: jump to anything.
      if (ev.ctrlKey && !ev.shiftKey && !ev.altKey && ev.key.toLowerCase() === "k") {
        ev.preventDefault();
        setPalette((p) => !p);
      }
      // Ctrl+N: a new session (in a browser, Ctrl+N may open a new window instead).
      if (ev.ctrlKey && !ev.shiftKey && !ev.altKey && ev.key.toLowerCase() === "n") {
        ev.preventDefault();
        newSessionRef.current();
      }
      // Ctrl+, : settings, as in most apps.
      if (ev.ctrlKey && !ev.shiftKey && !ev.altKey && ev.key === ",") {
        ev.preventDefault();
        setSettingsOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // The desktop app shows Thrall's own menus, not the browser's (Back, Refresh, Print…). Text
  // boxes and selected text keep theirs, for cut, copy and paste.
  useEffect(() => {
    if (CLIENT_KIND !== "desktop") return;
    const onMenu = (ev: MouseEvent) => {
      const t = ev.target as HTMLElement | null;
      const editable = t?.closest("input, textarea, [contenteditable='true']");
      const selected = (window.getSelection()?.toString() ?? "") !== "";
      if (!editable && !selected) ev.preventDefault();
    };
    window.addEventListener("contextmenu", onMenu);
    return () => window.removeEventListener("contextmenu", onMenu);
  }, []);

  const detach = useCallback(() => {
    const id = currentId.current;
    if (id) engine.current?.request({ type: "session.detach", session: id }).catch(() => undefined);
  }, []);

  const newSession = useCallback(() => {
    if (viewRef.current.busy) {
      note("Thrall is still working here. It carries on in the background; your new session starts now.");
    }
    detach();
    inHome.current = false;
    currentId.current = null;
    setCurrent(null);
    dispatch({ type: "reset", view: emptyView(viewRef.current.model) });
    setSidebarOpen(false);
    composer.current?.focus();
  }, [detach, note]);
  const newSessionRef = useRef(newSession);
  newSessionRef.current = newSession;

  const openSession = useCallback(
    async (id: string) => {
      const e = engine.current;
      if (!e || id === currentId.current) {
        setSidebarOpen(false);
        return;
      }
      detach();
      currentId.current = id;
      atBottom.current = true;
      setUnread((u) => {
        if (!u.has(id)) return u;
        const next = new Set(u);
        next.delete(id);
        return next;
      });
      setSidebarOpen(false);
      try {
        const opened = (await e.request({ type: "session.open", session: id })) as unknown as Session;
        setCurrent(opened);
        dispatch({ type: "reset", view: openedView(opened, viewRef.current.model) });
      } catch (err) {
        fail(err);
      }
    },
    [detach, fail],
  );
  openSessionRef.current = openSession;

  /** Open a session the person picked (not Home). */
  const pickSession = useCallback(
    (id: string) => {
      inHome.current = false;
      void openSession(id);
    },
    [openSession],
  );

  const openHome = useCallback(async () => {
    const e = engine.current;
    if (!e) return;
    try {
      const reply = await e.request<"home.shown">({ type: "home.get" });
      setHome({ page: reply.page, earlier: reply.earlier });
      inHome.current = true;
      await openSession(reply.page.session);
      composer.current?.focus();
    } catch (err) {
      fail(err);
    }
  }, [fail, openSession]);

  // The desktop app opens the session a clicked notification is about (Home's current page
  // opens as Home, so a fresh day moves on with it).
  const homeRef = useRef(home);
  homeRef.current = home;
  useEffect(() => {
    const w = window as unknown as Record<string, unknown>;
    w[OPEN_SESSION_JS] = (id: string) => {
      if (homeRef.current?.page.session === id) void openHome();
      else pickSession(id);
    };
    return () => {
      delete w[OPEN_SESSION_JS];
    };
  }, [openHome, pickSession]);

  const ensureSession = useCallback(async (): Promise<string> => {
    const e = engine.current;
    if (!e) throw new EngineError("closed", "Not connected to Thrall's engine yet.");
    if (currentId.current) return currentId.current;
    const opened = (await e.request({
      type: "session.create",
      workspace: folder || null,
      mode: viewRef.current.mode,
    })) as unknown as Session;
    currentId.current = opened.session;
    setCurrent(opened);
    dispatch({ type: "patch", patch: { title: opened.title, mode: opened.mode } });
    void refreshSessions();
    return opened.session;
  }, [folder, refreshSessions]);

  const send = useCallback(
    async (text: string, files: string[] = []) => {
      const e = engine.current;
      if (!e) return;
      atBottom.current = true;
      const busy = viewRef.current.busy;
      try {
        const session = await ensureSession();
        if (editingRef.current) {
          setEditing(false);
          await e.request({ type: "turn.edit", session, text });
          dispatch({ type: "patch", patch: { busy: true } });
          return;
        }
        await e.request({ type: "turn.send", session, text, files });
        if (busy) note("Added: Thrall will see it at its next step.");
        else dispatch({ type: "patch", patch: { busy: true } });
      } catch (err) {
        fail(err);
      }
    },
    [ensureSession, fail, note],
  );

  const setMode = useCallback(
    async (mode: string) => {
      const before = viewRef.current.mode;
      dispatch({ type: "patch", patch: { mode, offerAsk: false } });
      if (mode !== before) note(`${MODE_LABELS[mode] ?? mode}: ${MODE_COVERS[mode] ?? ""}`);
      const id = currentId.current;
      if (!id) return;
      try {
        await engine.current?.request({ type: "session.set_mode", session: id, mode });
      } catch (err) {
        dispatch({ type: "patch", patch: { mode: before } });
        fail(err);
      }
    },
    [fail, note],
  );

  const cycleMode = useCallback(() => {
    const i = MODE_CYCLE.indexOf(viewRef.current.mode as (typeof MODE_CYCLE)[number]);
    const next = MODE_CYCLE[(i + 1) % MODE_CYCLE.length] ?? "ask";
    void setMode(next);
  }, [setMode]);

  const command = useCallback(
    async (name: string, args: string) => {
      const e = engine.current;
      if (!e) return;
      atBottom.current = true;
      if (name === "new") {
        newSession();
        return;
      }
      const session = currentId.current;
      if (SESSION_COMMANDS.has(name) && !session) {
        note("Send a message first; that starts the session.");
        return;
      }
      if (name === "mode" && !session) {
        const wanted = Object.keys(MODE_LABELS).find((m) => m === args || m.startsWith(args));
        if (args && wanted) await setMode(wanted);
        else note(`Mode: ${MODE_LABELS[viewRef.current.mode]}. Change it with Shift+Tab or the mode chip.`);
        return;
      }
      try {
        const reply = await e.request<"command.result">({ type: "command.run", session, name, args });
        let markdown = reply.markdown;
        if (name === "help") {
          markdown += CLIENT_COMMANDS.map((c) => `- \`/${c.name}\`: ${c.summary}\n`).join("");
          markdown += "\n**Keys:** Shift+Tab mode · Esc stop · Ctrl+O step details · y / a / n answer cards";
        }
        dispatch({ type: "local", item: { kind: "command", id: localId("c"), name, markdown } });
      } catch (err) {
        fail(err);
      }
    },
    [fail, newSession, note, setMode],
  );

  const stop = useCallback(() => {
    const id = currentId.current;
    if (!id) return;
    engine.current
      ?.request({ type: "turn.interrupt", session: id })
      .then(() => note("Stopping…"))
      .catch(fail);
  }, [fail, note]);

  const choose = useCallback(
    async (choice: Choice) => {
      const card = viewRef.current.card;
      const e = engine.current;
      if (!card || !e) return;
      dispatch({ type: "patch", patch: { card: null } });
      try {
        if (choice.kind === "permission")
          await e.request({ type: "permission.answer", request: card.request, choice: choice.choice });
        else if (choice.kind === "question")
          await e.request({ type: "question.answer", question: card.request, answers: choice.answers });
        else
          await e.request({
            type: "plan.decide",
            plan: card.request,
            approve: choice.approve,
            auto_edits: choice.autoEdits,
          });
      } catch (err) {
        if (!(err instanceof EngineError && err.code === "already_answered")) fail(err);
      }
    },
    [fail],
  );

  const rename = useCallback(
    async (title: string) => {
      setEditingTitle(false);
      const id = currentId.current;
      const t = title.trim();
      if (!id || !t || t === viewRef.current.title) return;
      try {
        await engine.current?.request({ type: "session.rename", session: id, title: t });
      } catch (err) {
        fail(err);
      }
    },
    [fail],
  );

  const renameSession = useCallback(
    async (id: string, title: string) => {
      const t = title.trim();
      if (!t) return;
      try {
        await engine.current?.request({ type: "session.rename", session: id, title: t });
        setSessions((list) => list.map((s) => (s.session === id ? { ...s, title: t } : s)));
        if (id === currentId.current) dispatch({ type: "patch", patch: { title: t } });
      } catch (err) {
        fail(err);
      }
    },
    [fail],
  );

  const archiveSession = useCallback(
    async (id: string, archived: boolean) => {
      try {
        await engine.current?.request({ type: "session.archive", session: id, archived });
        if (archived && id === currentId.current) newSession();
        void refreshSessions();
      } catch (err) {
        fail(err);
      }
    },
    [fail, newSession, refreshSessions],
  );

  const deleteSession = useCallback(
    async (id: string) => {
      try {
        await engine.current?.request({ type: "session.delete", session: id });
        if (id === currentId.current) {
          currentId.current = null;
          setCurrent(null);
          dispatch({ type: "reset", view: emptyView(viewRef.current.model) });
        }
        setSessions((list) => list.filter((s) => s.session !== id));
        void refreshSessions();
      } catch (err) {
        fail(err);
      }
    },
    [fail, refreshSessions],
  );

  const pinSession = useCallback(
    async (id: string, pinned: boolean) => {
      setSessions((list) => list.map((s) => (s.session === id ? { ...s, pinned } : s)));
      try {
        await engine.current?.request({ type: "session.pin", session: id, pinned });
      } catch (err) {
        fail(err);
      }
      void refreshSessions();
    },
    [fail, refreshSessions],
  );

  const moveSession = useCallback(
    async (id: string, group: string | null) => {
      setSessions((list) => list.map((s) => (s.session === id ? { ...s, group } : s)));
      try {
        await engine.current?.request({ type: "session.move", session: id, group });
      } catch (err) {
        fail(err);
      }
      void refreshSessions();
    },
    [fail, refreshSessions],
  );

  const moveSessionFolder = useCallback(
    async (id: string, folder: string) => {
      try {
        await engine.current?.request({ type: "session.set_folder", session: id, folder });
        setSessions((list) => list.map((s) => (s.session === id ? { ...s, workspace: folder } : s)));
      } catch (err) {
        fail(err);
      }
      void refreshSessions();
    },
    [fail, refreshSessions],
  );

  /** Run a group request; every one answers with the new list. */
  const changeGroups = useCallback(
    async (request: Parameters<Engine["request"]>[0]) => {
      try {
        const reply = await engine.current?.request<"group.listed">(request);
        if (reply) setGroups(reply.items);
        void refreshSessions();
      } catch (err) {
        fail(err);
      }
    },
    [fail, refreshSessions],
  );

  const loadArchived = useCallback(async (): Promise<Session[]> => {
    const e = engine.current;
    if (!e) return [];
    const reply = await e.request<"session.listed">({
      type: "session.list",
      limit: ARCHIVED_SESSIONS,
      archived: true,
    });
    return reply.items;
  }, []);

  const retry = useCallback(() => {
    const id = currentId.current;
    if (!id) return;
    atBottom.current = true;
    engine.current
      ?.request({ type: "turn.retry", session: id })
      .then(() => dispatch({ type: "patch", patch: { busy: true } }))
      .catch(fail);
  }, [fail]);

  const [rewinding, setRewinding] = useState<string | null>(null);
  const openRewind = useCallback((turn: string) => setRewinding(turn), []);
  const forget = useCallback(
    (fact: string) => {
      engine.current
        ?.request({ type: "memory.forget", fact })
        .then(() => dispatch({ type: "forgot", fact }))
        .catch(fail);
    },
    [fail],
  );

  // The engine posts what the rewind did as a notice; the message goes back in the box.
  const rewound = useCallback((r: Rewound) => {
    setRewinding(null);
    if (r.conversation && r.text) {
      setEditing(false);
      composer.current?.fill(r.text);
    }
  }, []);

  // Edit and Retry belong to the last message and reply, and only while Thrall is idle.
  const lastUser = view.busy ? undefined : view.items.findLast((i) => i.kind === "user");
  const lastReply = view.busy ? undefined : view.items.findLast((i) => i.kind === "assistant" && i.text);

  const paletteItems = (): PaletteItem[] => {
    const items: PaletteItem[] = [
      { id: "a-home", kind: "Action", label: "Home", hint: "Your own conversation", run: () => void openHome() },
      { id: "a-new", kind: "Action", label: "New session", hint: "Ctrl+N", run: newSession },
      {
        id: "a-theme",
        kind: "Action",
        label: "Switch theme",
        hint: "Light · Dark",
        run: () => setTheme(flipTheme),
      },
      { id: "a-settings", kind: "Action", label: "Settings", hint: "Ctrl+,", run: () => setSettingsOpen(true) },
      { id: "a-tour", kind: "Action", label: "Take the tour", hint: "One minute", run: () => setTouring(true) },
      {
        id: "a-steps",
        kind: "Action",
        label: expanded ? "Hide step details" : "Show step details",
        hint: "Ctrl+O",
        run: () => setExpanded((x) => !x),
      },
      { id: "a-mode", kind: "Action", label: "Change mode", hint: "Shift+Tab", run: cycleMode },
    ];
    for (const s of sessions) {
      items.push({
        id: `s-${s.session}`,
        kind: "Session",
        label: s.title || "New session",
        hint: folderName(s.workspace),
        run: () => pickSession(s.session),
      });
    }
    const all = [...commands.map((c) => ({ name: c.name, summary: c.summary, hint: c.args_hint })), ...CLIENT_COMMANDS.map((c) => ({ ...c, hint: "" }))];
    for (const c of all) {
      items.push({
        id: `c-${c.name}`,
        kind: "Command",
        label: `/${c.name}`,
        hint: c.summary,
        // Commands that take something after them go into the message box to finish.
        run: () => (c.hint ? composer.current?.fill(`/${c.name} `) : void command(c.name, "")),
      });
    }
    return items;
  };

  const activity = shownActivity(view, Date.now(), DONE_HOLD_MS);

  if (!info) {
    return (
      <div className="connecting">
        <LivingMark activity={conn === "closed" ? "failed" : "thinking"} size={CONNECTING_MARK_PX} />
        <p className="th-readout">{conn === "closed" ? "Can't reach Thrall" : "Connecting"}</p>
        {conn === "closed" ? (
          <p>
            Thrall's engine isn't answering. Start it with <code>thrall engine</code> (or open Thrall in a
            terminal); this page connects by itself once it's up.
          </p>
        ) : null}
      </div>
    );
  }

  const workspace = current?.workspace ?? (current ? "" : folder);
  const empty = view.items.length === 0 && !view.card;
  const title = view.title || (current ? "New session" : "");

  return (
    <div className={`shell${view.tasks.length > 0 ? " has-panel" : ""}`}>
      <Sidebar
        home={home}
        onHome={() => void openHome()}
        sessions={sessions}
        groups={groups}
        current={current?.session ?? null}
        unread={unread}
        activity={activity}
        open={sidebarOpen}
        onNew={newSession}
        theme={theme}
        onTheme={() => setTheme(flipTheme)}
        onSettings={() => setSettingsOpen(true)}
        actions={{
          onPick: pickSession,
          onRename: (id, title) => void renameSession(id, title),
          onArchive: (id, archived) => void archiveSession(id, archived),
          onDelete: (id) => void deleteSession(id),
          onPin: (id, pinned) => void pinSession(id, pinned),
          onMove: (id, group) => void moveSession(id, group),
          onMoveFolder: (id, folder) => void moveSessionFolder(id, folder),
          loadArchived,
          onCreateGroup: (name) => void changeGroups({ type: "group.create", name }),
          onRenameGroup: (group, name) => void changeGroups({ type: "group.rename", group, name }),
          onDeleteGroup: (group) => void changeGroups({ type: "group.delete", group }),
          onReorderGroups: (ids) => {
            setGroups((list) => ids.map((id) => list.find((g) => g.group === id)).filter((g): g is GroupInfo => Boolean(g)));
            void changeGroups({ type: "group.reorder", groups: ids });
          },
        }}
      />
      <div className={`scrim${sidebarOpen ? " is-open" : ""}`} onClick={() => setSidebarOpen(false)} />
      <main className="main">
        {conn !== "open" ? <div className="banner">Reconnecting to Thrall… your session is saved.</div> : null}
        <header className="topbar">
          <button
            type="button"
            className="th-btn th-btn-quiet menu"
            onClick={() => setSidebarOpen(true)}
            aria-label="Show sessions"
          >
            Sessions
          </button>
          <div className="topbar-title">
            {editingTitle ? (
              <input
                id="session-title"
                autoFocus
                defaultValue={view.title}
                aria-label="Session title"
                onBlur={(ev) => void rename(ev.target.value)}
                onKeyDown={(ev) => {
                  if (ev.key === "Enter") void rename(ev.currentTarget.value);
                  if (ev.key === "Escape") setEditingTitle(false);
                }}
              />
            ) : (
              <h1
                title={current && !current.home ? "Click to rename" : undefined}
                onClick={() => current && !current.home && setEditingTitle(true)}
                style={{ cursor: current && !current.home ? "text" : "default" }}
              >
                {title || "New session"}
              </h1>
            )}
            {workspace ? <span className="path">{workspace}</span> : null}
          </div>
        </header>
        <div
          className="scroller"
          ref={scroller}
          onScroll={(ev) => {
            const el = ev.currentTarget;
            atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX;
          }}
        >
          <div className="column">
            {empty ? (
              <div className="empty">
                <LivingMark activity={activity} size={CARD_MARK_PX} />
                <h2 className="th-greeting">{current?.home ? HOME_GREETING : GREETING}</h2>
                {!current ? (
                  <div className="folder">
                    {makingFolder ? (
                      <NewFolder
                        request={requestEngine}
                        onCreated={(path) => {
                          setFolder(path);
                          saveFolder(path);
                          setMakingFolder(false);
                        }}
                        onCancel={() => setMakingFolder(false)}
                      />
                    ) : editingFolder ? (
                      <input
                        id="folder"
                        autoFocus
                        defaultValue={folder}
                        placeholder="C:\Users\you\Projects\something"
                        aria-label="Folder Thrall works in"
                        onBlur={(ev) => {
                          setFolder(ev.target.value.trim());
                          saveFolder(ev.target.value.trim());
                          setEditingFolder(false);
                        }}
                        onKeyDown={(ev) => {
                          if (ev.key === "Enter") ev.currentTarget.blur();
                          if (ev.key === "Escape") setEditingFolder(false);
                        }}
                      />
                    ) : (
                      <>
                        <span>{folder ? "Working in" : "No folder chosen: Thrall works in its own folder."}</span>
                        {folder ? <code>{folder}</code> : null}
                        <button type="button" className="th-btn th-btn-quiet" onClick={() => setEditingFolder(true)}>
                          {folder ? "Change" : "Choose a folder"}
                        </button>
                        <button type="button" className="th-btn th-btn-quiet" onClick={() => setMakingFolder(true)}>
                          ＋ New folder
                        </button>
                      </>
                    )}
                  </div>
                ) : null}
              </div>
            ) : (
              <FileLoader.Provider value={loadFile}>
              <div className="th-transcript">
                {view.items.map((item) => (
                  <TranscriptItem
                    key={`${item.kind}-${item.id}`}
                    item={item}
                    expanded={expanded}
                    onEdit={
                      item === lastUser && item.kind === "user"
                        ? () => {
                            setEditing(true);
                            composer.current?.fill(item.text);
                          }
                        : undefined
                    }
                    onRetry={item === lastReply ? retry : undefined}
                    onRewind={view.busy ? undefined : openRewind}
                    onForget={forget}
                  />
                ))}
              </div>
              </FileLoader.Provider>
            )}
          </div>
        </div>
        <div className="bottom">
          <div className="column">
            {view.card ? (
              <DecisionCard key={view.card.request} card={view.card} onChoose={(c) => void choose(c)} keysActive={composerEmpty} />
            ) : null}
            {me?.oversight ? (
              <p className="th-oversight" role="note">
                {me.overseer ?? "This Thrall's owner"}{" "}
                {me.oversight === "full"
                  ? "can read your chats with Thrall."
                  : me.oversight === "flagged"
                    ? "can see your chats' titles, and read chats where something wasn't allowed."
                    : "can see your chats' titles and when you had them."}
              </p>
            ) : null}
            <ModeBanner
              mode={view.mode}
              limit={view.modeLimit}
              offerAsk={view.offerAsk}
              onAsk={() => void setMode("ask")}
              onKeep={() => dispatch({ type: "patch", patch: { offerAsk: false } })}
            />
            <Composer
              ref={composer}
              commands={commands}
              busy={view.busy}
              disabled={conn !== "open"}
              onSend={(t, files) => void send(t, files)}
              onUpload={(file) => {
                const e = engine.current;
                if (!e) return Promise.reject(new EngineError("closed", "Not connected to Thrall's engine yet."));
                return e.upload(file);
              }}
              onDictate={async (recording) => {
                const e = engine.current;
                if (!e) throw new EngineError("closed", "Not connected to Thrall's engine yet.");
                const saved = await e.upload(recording);
                const reply = await e.request<"audio.transcribed">({
                  type: "audio.transcribe",
                  file: saved.file,
                  session: currentId.current ?? undefined,
                });
                return reply.text;
              }}
              onCommand={(n, a) => void command(n, a)}
              onStop={stop}
              onCycleMode={cycleMode}
              onEmptyChange={setComposerEmpty}
              editing={editing}
              onCancelEdit={() => setEditing(false)}
            />
            <StatusBar mode={view.mode} workspace={workspace} model={view.model} cost={view.cost} onCycleMode={cycleMode} />
          </div>
        </div>
      </main>
      {view.tasks.length > 0 ? <TaskPanel tasks={view.tasks} /> : null}
      {palette ? <CommandPalette items={paletteItems()} onClose={() => setPalette(false)} /> : null}
      {welcoming && me ? (
        <Welcome
          name={me.name}
          owner={(me.role ?? "owner") === "owner"}
          engine={engine.current}
          onTour={() => {
            markWelcomed(me.id);
            setWelcoming(false);
            setTouring(true);
          }}
          onDone={() => {
            markWelcomed(me.id);
            setWelcoming(false);
          }}
        />
      ) : null}
      {touring ? <Tour onDone={() => setTouring(false)} /> : null}
      {(asks.length > 0 && !settingsOpen) || askNote ? (
        <div className="th-asks" role="status">
          {settingsOpen
            ? null
            : asks.map((r) => (
                <PcAsking
                  key={r.request}
                  engine={engine.current}
                  request={r}
                  people={askPeople}
                  onDone={(left) => setAsks(left.items)}
                  onStatus={(s) => setAskNote(s.text)}
                />
              ))}
          {askNote ? <p className="th-asks-note">{askNote}</p> : null}
        </div>
      ) : null}
      {rewinding && current && engine.current ? (
        <RewindDialog
          engine={engine.current}
          session={current.session}
          turn={rewinding}
          onDone={rewound}
          onClose={() => setRewinding(null)}
        />
      ) : null}
      {settingsOpen ? (
        <Settings
          engine={engine.current}
          theme={theme}
          onTheme={setTheme}
          version={info?.version ?? ""}
          onTour={() => {
            setSettingsOpen(false);
            setTouring(true);
          }}
          onClose={() => {
            setSettingsOpen(false);
            refreshCommands();
          }}
          onHomeCleared={(page) => {
            setHome({ page, earlier: [] });
            if (current?.home) {
              inHome.current = true;
              currentId.current = null;
              void openSession(page.session);
            }
          }}
        />
      ) : null}
    </div>
  );
}
