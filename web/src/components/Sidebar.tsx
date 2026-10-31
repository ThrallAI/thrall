// Sessions, newest first: pinned ones on top, then the person's own groups, then the rest grouped
// by the folder they work in, then the archive. Drag a session onto a section to pin it or move
// it; drag a group's heading to reorder groups. Right-click (or ⋯) does the same without a mouse
// drag, plus rename, archive and delete.

import { useEffect, useMemo, useRef, useState, type DragEvent } from "react";

import { COLLAPSED_KEY } from "../constants";
import type { GroupInfo } from "../protocol";
import { shownTheme, type Theme } from "../theme";
import type { Activity } from "../transcript";
import type { Session } from "../types";
import { ConfirmDialog } from "./ConfirmDialog";
import { ACTIVITY_LABELS, LivingMark } from "./LivingMark";

const SIDEBAR_MARK_PX = 48;
const NO_FOLDER = "";
const MENU_WIDTH_PX = 220;
const MENU_HEIGHT_PX = 320;
const DRAG_SESSION = "application/x-thrall-session";
const DRAG_GROUP = "application/x-thrall-group";
const PINNED_KEY = "pinned";
// Kept in the "collapsed" set with the opposite meaning: Earlier starts closed.
const EARLIER_OPEN_KEY = "home-earlier-open";
const HOME_PREFIX = /^Home · /;

/** Home: Thrall's built-in conversation, its current page and earlier ones. */
export interface HomePages {
  page: Session;
  earlier: Session[];
}

export function folderName(path: string | null | undefined): string {
  if (!path) return "";
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) ?? path;
}

function when(ms: number, now: number): string {
  const d = new Date(ms);
  const today = new Date(now);
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
  if (sameDay(d, today)) return "Today";
  if (sameDay(d, new Date(now - 86_400_000))) return "Yesterday";
  return d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

function meta(s: Session, now: number, showFolder: boolean): string {
  if (s.state === "working") return "Working…";
  // Waiting sessions show a Needs you badge instead.
  const folder = showFolder ? folderName(s.workspace) : "";
  return folder ? `${when(s.updated, now)} · ${folder}` : when(s.updated, now);
}

function readCollapsed(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

function saveCollapsed(set: Set<string>): void {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...set]));
  } catch {
    // Remembering open sections is only a convenience.
  }
}

export interface Group {
  key: string;
  label: string;
  sessions: Session[];
}

/** Sessions grouped by folder, keeping their order; groups ordered by their newest session. */
export function groupByFolder(sessions: Session[]): Group[] {
  const groups = new Map<string, Group>();
  for (const s of sessions) {
    const key = s.workspace ?? NO_FOLDER;
    let g = groups.get(key);
    if (!g) {
      g = { key, label: key ? folderName(key) : "No folder", sessions: [] };
      groups.set(key, g);
    }
    g.sessions.push(s);
  }
  return [...groups.values()];
}

/** Where each session shows: pinned first, then its own group, then its folder. */
export function arrange(sessions: Session[], groups: GroupInfo[]) {
  const known = new Set(groups.map((g) => g.group));
  const pinned = sessions.filter((s) => s.pinned);
  const inGroup = new Map<string, Session[]>(groups.map((g) => [g.group, []]));
  const loose: Session[] = [];
  for (const s of sessions) {
    if (s.pinned) continue;
    if (s.group && known.has(s.group)) inGroup.get(s.group)?.push(s);
    else loose.push(s);
  }
  return { pinned, inGroup, folders: groupByFolder(loose) };
}

type Menu =
  | { kind: "session"; session: Session; x: number; y: number; archived: boolean }
  | { kind: "group"; group: GroupInfo; x: number; y: number };

type Confirm =
  | { kind: "session"; session: Session }
  | { kind: "group"; group: GroupInfo }
  | { kind: "folder"; session: Session; folder: string; label: string };

export interface SessionActions {
  onPick: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onArchive: (id: string, archived: boolean) => void;
  onDelete: (id: string) => void;
  onPin: (id: string, pinned: boolean) => void;
  onMove: (id: string, group: string | null) => void;
  onMoveFolder: (id: string, folder: string) => void;
  loadArchived: () => Promise<Session[]>;
  onCreateGroup: (name: string) => void;
  onRenameGroup: (group: string, name: string) => void;
  onDeleteGroup: (group: string) => void;
  onReorderGroups: (groups: string[]) => void;
}

export function Sidebar({
  home,
  onHome,
  sessions,
  groups,
  current,
  unread,
  activity,
  open,
  onNew,
  theme,
  onTheme,
  onSettings,
  actions,
}: {
  home: HomePages | null;
  onHome: () => void;
  sessions: Session[];
  groups: GroupInfo[];
  current: string | null;
  unread: Set<string>;
  activity: Activity;
  open: boolean;
  onNew: () => void;
  theme: Theme;
  onTheme: () => void;
  onSettings: () => void;
  actions: SessionActions;
}) {
  const now = Date.now();
  const busy = activity !== "idle" && activity !== "done";
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const [menu, setMenu] = useState<Menu | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [renamingGroup, setRenamingGroup] = useState<string | null>(null);
  const [addingGroup, setAddingGroup] = useState(false);
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [showArchived, setShowArchived] = useState(false);
  const [archived, setArchived] = useState<Session[]>([]);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  const searching = query.trim() !== "";
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return sessions;
    return sessions.filter(
      (s) => s.title.toLowerCase().includes(q) || (s.workspace ?? "").toLowerCase().includes(q),
    );
  }, [sessions, query]);
  const layout = useMemo(() => arrange(shown, groups), [shown, groups]);

  // Close the menu on a click elsewhere, Esc, or a resize.
  useEffect(() => {
    if (!menu) return;
    const close = (e: Event) => {
      if (e instanceof MouseEvent && menuRef.current?.contains(e.target as Node)) return;
      setMenu(null);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenu(null);
    };
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", onKey);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("resize", close);
    };
  }, [menu]);

  const loadArchived = () => {
    actions
      .loadArchived()
      .then(setArchived)
      .catch(() => setArchived([]));
  };

  const toggle = (key: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      saveCollapsed(next);
      return next;
    });
  };

  const place = (x: number, y: number) => ({
    x: Math.max(4, Math.min(x, window.innerWidth - MENU_WIDTH_PX)),
    y: Math.max(4, Math.min(y, window.innerHeight - MENU_HEIGHT_PX)),
  });

  // ---------- drag and drop ----------

  const dropZone = (key: string, onSession: (id: string) => void, onGroup?: (id: string) => void) => ({
    onDragOver: (e: DragEvent) => {
      const types = e.dataTransfer.types;
      if (types.includes(DRAG_SESSION) || (onGroup && types.includes(DRAG_GROUP))) {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        if (dropTarget !== key) setDropTarget(key);
      }
    },
    onDragLeave: (e: DragEvent) => {
      if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node | null)) setDropTarget(null);
    },
    onDrop: (e: DragEvent) => {
      e.preventDefault();
      setDropTarget(null);
      // The dragged row may have moved to another section already, so its dragend never comes.
      setDragging(false);
      const session = e.dataTransfer.getData(DRAG_SESSION);
      const group = e.dataTransfer.getData(DRAG_GROUP);
      if (session) onSession(session);
      else if (group && onGroup) onGroup(group);
    },
  });

  const moveGroupBefore = (dragged: string, target: string) => {
    if (dragged === target) return;
    const ids = groups.map((g) => g.group).filter((id) => id !== dragged);
    const at = ids.indexOf(target);
    ids.splice(at < 0 ? ids.length : at, 0, dragged);
    actions.onReorderGroups(ids);
  };

  const sessionOf = (id: string) => sessions.find((s) => s.session === id);

  const pinDropped = (id: string) => {
    const s = sessionOf(id);
    if (s && !s.pinned) actions.onPin(id, true);
  };

  const dropIntoGroup = (group: string | null) => (id: string) => {
    const s = sessionOf(id);
    if (!s) return;
    if (s.pinned) actions.onPin(id, false);
    if ((s.group ?? null) !== group) actions.onMove(id, group);
  };

  // Onto a folder: out of its group (as before) and, after a confirm, into that folder, since
  // Thrall then works there.
  const dropOnFolder = (folder: string, label: string) => (id: string) => {
    const s = sessionOf(id);
    if (!s) return;
    dropIntoGroup(null)(id);
    if (folder && (s.workspace ?? "") !== folder) setConfirm({ kind: "folder", session: s, folder, label });
  };

  // ---------- pieces ----------

  const row = (s: Session, isArchived: boolean, showFolder: boolean) => {
    if (renaming === s.session) {
      return (
        <li key={s.session}>
          <input
            id={`rename-${s.session}`}
            className="th-rename"
            autoFocus
            defaultValue={s.title}
            aria-label="Session title"
            onBlur={(e) => {
              setRenaming(null);
              actions.onRename(s.session, e.target.value);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") setRenaming(null);
            }}
          />
        </li>
      );
    }
    const cls = [
      s.state === "working" ? "is-working" : s.state === "waiting_for_you" ? "is-waiting" : "",
      unread.has(s.session) ? "is-unread" : "",
    ]
      .filter(Boolean)
      .join(" ");
    return (
      // The row is what drags: Chromium (and so WebView2) won't start a drag from a <button>.
      <li
        key={s.session}
        className="th-session"
        draggable={!isArchived}
        onDragStart={(e) => {
          e.dataTransfer.setData(DRAG_SESSION, s.session);
          e.dataTransfer.effectAllowed = "move";
          setDragging(true);
        }}
        onDragEnd={() => {
          setDragging(false);
          setDropTarget(null);
        }}
      >
        <button
          type="button"
          aria-current={s.session === current ? "page" : undefined}
          className={cls}
          onClick={() => {
            if (!isArchived) actions.onPick(s.session);
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu({ kind: "session", session: s, archived: isArchived, ...place(e.clientX, e.clientY) });
          }}
          title={s.workspace ?? undefined}
        >
          <b>
            {unread.has(s.session) ? <span className="th-unread" aria-label="New activity" /> : null}
            {s.title || "New session"}
          </b>
          {s.state === "waiting_for_you" ? <span className="th-needs-you">Needs you</span> : null}
          <span>{meta(s, now, showFolder)}</span>
        </button>
        <button
          type="button"
          className="th-more"
          aria-label={`More for ${s.title || "this session"}`}
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            setMenu({ kind: "session", session: s, archived: isArchived, ...place(r.left, r.bottom + 4) });
          }}
        >
          ⋯
        </button>
      </li>
    );
  };

  const waitingIn = (list: Session[]) => list.filter((s) => s.state === "waiting_for_you").length;

  const section = (
    key: string,
    label: string,
    list: Session[],
    opts: {
      showFolder: boolean;
      title?: string;
      onDropSession?: (id: string) => void;
      group?: GroupInfo;
      emptyHint?: string;
    },
  ) => {
    const closed = collapsed.has(key) && !searching;
    const g = opts.group;
    const zone = opts.onDropSession
      ? dropZone(key, opts.onDropSession, g ? (dragged) => moveGroupBefore(dragged, g.group) : undefined)
      : {};
    return (
      <section key={key} className={`th-group${dropTarget === key ? " is-drop-target" : ""}`} {...zone}>
        {g && renamingGroup === g.group ? (
          <input
            id={`rename-group-${g.group}`}
            className="th-rename th-rename-group"
            autoFocus
            defaultValue={g.name}
            aria-label="Group name"
            onBlur={(e) => {
              setRenamingGroup(null);
              if (e.target.value.trim() && e.target.value.trim() !== g.name) actions.onRenameGroup(g.group, e.target.value);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") setRenamingGroup(null);
            }}
          />
        ) : (
          <div
            className="th-group-row"
            draggable={Boolean(g)}
            onDragStart={(e) => {
              if (!g) return;
              e.dataTransfer.setData(DRAG_GROUP, g.group);
              e.dataTransfer.effectAllowed = "move";
            }}
          >
            <button
              type="button"
              className="th-group-head"
              aria-expanded={!closed}
              onClick={() => toggle(key)}
              title={opts.title}
              onContextMenu={(e) => {
                if (!g) return;
                e.preventDefault();
                setMenu({ kind: "group", group: g, ...place(e.clientX, e.clientY) });
              }}
            >
              <span aria-hidden="true">{closed ? "▸" : "▾"}</span>
              <span className="th-group-name">{label}</span>
              {waitingIn(list) > 0 ? (
                <span className="th-needs-you" title="Sessions in here are waiting for your answer">
                  {waitingIn(list)} need{waitingIn(list) === 1 ? "s" : ""} you
                </span>
              ) : null}
              <span className="th-group-count">{list.length}</span>
            </button>
            {g ? (
              <button
                type="button"
                className="th-more th-group-more"
                aria-label={`More for the ${g.name} group`}
                onClick={(e) => {
                  const r = e.currentTarget.getBoundingClientRect();
                  setMenu({ kind: "group", group: g, ...place(r.left, r.bottom + 4) });
                }}
              >
                ⋯
              </button>
            ) : null}
          </div>
        )}
        {closed ? null : list.length === 0 ? (
          opts.emptyHint ? <p className="sidebar-empty th-drop-hint">{opts.emptyHint}</p> : null
        ) : (
          <ul className="th-sessions">{list.map((s) => row(s, false, opts.showFolder))}</ul>
        )}
      </section>
    );
  };

  return (
    <nav className={`sidebar${open ? " is-open" : ""}`} aria-label="Sessions">
      <div className="brand">
        <LivingMark activity={activity} size={SIDEBAR_MARK_PX} />
        <div className="brand-words">
          <span className="th-wordmark">Thrall</span>
          <span className={`th-readout${busy ? " is-busy" : ""}`} aria-live="polite">
            {ACTIVITY_LABELS[activity]}
          </span>
        </div>
      </div>
      <div className="th-home">
        {(() => {
          const page = home?.page;
          const inHome = Boolean(page && page.session === current);
          const state = page?.state;
          return (
            <button
              type="button"
              className={`th-home-btn${page && unread.has(page.session) ? " is-unread" : ""}`}
              aria-current={inHome ? "page" : undefined}
              onClick={onHome}
              title="Your own conversation with Thrall, apart from your projects"
            >
              <svg className="th-home-icon" viewBox="0 0 16 16" aria-hidden="true">
                <path d="M2.5 7.2 8 2.5l5.5 4.7V13a.5.5 0 0 1-.5.5H9.6V9.8H6.4v3.7H3a.5.5 0 0 1-.5-.5Z" />
              </svg>
              <b>
                {page && unread.has(page.session) ? <span className="th-unread" aria-label="New activity" /> : null}
                Home
              </b>
              {state === "waiting_for_you" ? (
                <span className="th-needs-you">Needs you</span>
              ) : state === "working" ? (
                <span className="th-home-meta">Working…</span>
              ) : null}
            </button>
          );
        })()}
        {home && home.earlier.length > 0 ? (
          <>
            <button
              type="button"
              className="th-group-head"
              aria-expanded={collapsed.has(EARLIER_OPEN_KEY)}
              onClick={() => toggle(EARLIER_OPEN_KEY)}
              title="Earlier Home pages"
            >
              <span aria-hidden="true">{collapsed.has(EARLIER_OPEN_KEY) ? "▾" : "▸"}</span>
              <span className="th-group-name">Earlier</span>
              <span className="th-group-count">{home.earlier.length}</span>
            </button>
            {collapsed.has(EARLIER_OPEN_KEY) ? (
              <ul className="th-sessions th-home-earlier">
                {home.earlier.map((s) => (
                  <li key={s.session}>
                    <button
                      type="button"
                      aria-current={s.session === current ? "page" : undefined}
                      className={unread.has(s.session) ? "is-unread" : undefined}
                      onClick={() => actions.onPick(s.session)}
                    >
                      <b>{s.title.replace(HOME_PREFIX, "") || "Home"}</b>
                    </button>
                  </li>
                ))}
              </ul>
            ) : null}
          </>
        ) : null}
      </div>
      <div className="sidebar-actions">
        <button type="button" className="th-btn" onClick={onNew} title="New session (Ctrl+N)" data-tour="new">
          New session
          <span className="th-key">Ctrl+N</span>
        </button>
        {dragging && layout.pinned.length === 0 ? (
          // Over the search box, so nothing in the list moves under the pointer mid-drag.
          <div
            className={`th-pin-overlay${dropTarget === PINNED_KEY ? " is-drop-target" : ""}`}
            {...dropZone(PINNED_KEY, pinDropped)}
          >
            Drop here to pin
          </div>
        ) : null}
        <input
          id="session-search"
          className="th-search"
          type="search"
          placeholder="Search sessions"
          aria-label="Search sessions"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      <div className="th-session-scroll">
        {sessions.length === 0 ? <p className="sidebar-empty">Your sessions appear here.</p> : null}
        {sessions.length > 0 && shown.length === 0 ? (
          <p className="sidebar-empty">No session matches “{query.trim()}”.</p>
        ) : null}

        {layout.pinned.length > 0
          ? section(PINNED_KEY, "Pinned", layout.pinned, {
              showFolder: true,
              onDropSession: pinDropped,
            })
          : null}

        {/* Groups and folders look alike, so each kind gets its own labelled part of the list. */}
        <div className="th-list-label">
          <span>Groups</span>
          <button type="button" className="th-list-add" onClick={() => setAddingGroup(true)} title="New group">
            + New
          </button>
        </div>
        {addingGroup ? (
          <input
            id="new-group"
            className="th-rename th-rename-group"
            autoFocus
            placeholder="Group name"
            aria-label="New group name"
            onBlur={(e) => {
              setAddingGroup(false);
              if (e.target.value.trim()) actions.onCreateGroup(e.target.value);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") {
                e.currentTarget.value = "";
                setAddingGroup(false);
              }
            }}
          />
        ) : null}
        {groups.length === 0 && !addingGroup ? <p className="sidebar-empty th-drop-hint">Make a group to keep sessions together.</p> : null}
        {groups.map((g) =>
          section(`group:${g.group}`, g.name, layout.inGroup.get(g.group) ?? [], {
            showFolder: true,
            group: g,
            onDropSession: dropIntoGroup(g.group),
            emptyHint: searching ? undefined : "Drag sessions here.",
          }),
        )}

        {layout.folders.length > 0 ? (
          <div className="th-list-label">
            <span>Folders</span>
          </div>
        ) : null}
        {layout.folders.map((f) =>
          section(`folder:${f.key}`, f.label, f.sessions, {
            showFolder: false,
            title: f.key || undefined,
            onDropSession: dropOnFolder(f.key, f.label),
          }),
        )}

        <section className="th-group th-archived">
          <div className="th-group-row">
            <button
              type="button"
              className="th-group-head"
              aria-expanded={showArchived}
              onClick={() => {
                const next = !showArchived;
                setShowArchived(next);
                if (next) loadArchived();
              }}
            >
              <span aria-hidden="true">{showArchived ? "▾" : "▸"}</span>
              <span className="th-group-name">Archived</span>
            </button>
          </div>
          {showArchived ? (
            archived.length === 0 ? (
              <p className="sidebar-empty">Nothing archived.</p>
            ) : (
              <ul className="th-sessions is-archived">{archived.map((s) => row(s, true, true))}</ul>
            )
          ) : null}
        </section>
      </div>
      <div className="sidebar-foot">
        <button type="button" className="th-btn th-btn-quiet" onClick={onSettings} title="Settings (Ctrl+,)" data-tour="settings">
          <svg className="th-icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z" />
            <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" />
          </svg>
          Settings
        </button>
        <button
          type="button"
          role="switch"
          aria-checked={shownTheme(theme) === "dark"}
          aria-label="Dark theme"
          className={`th-switch th-theme-switch${shownTheme(theme) === "dark" ? " is-on" : ""}`}
          onClick={onTheme}
          title={theme === "system" ? "Following Windows. Click to pick light or dark." : "Light or dark (Settings → General to follow Windows again)"}
        >
          <span />
        </button>
      </div>

      {menu?.kind === "session" ? (
        <div ref={menuRef} className="th-menu" role="menu" style={{ left: menu.x, top: menu.y }}>
          {menu.archived ? (
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                actions.onArchive(menu.session.session, false);
                setArchived((list) => list.filter((s) => s.session !== menu.session.session));
                setMenu(null);
              }}
            >
              Restore
            </button>
          ) : (
            <>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  actions.onPin(menu.session.session, !menu.session.pinned);
                  setMenu(null);
                }}
              >
                {menu.session.pinned ? "Unpin" : "Pin to top"}
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setRenaming(menu.session.session);
                  setMenu(null);
                }}
              >
                Rename
              </button>
              {groups.length > 0 ? (
                <>
                  <p className="th-menu-label">Move to group</p>
                  {groups.map((g) => (
                    <button
                      key={g.group}
                      type="button"
                      role="menuitemradio"
                      aria-checked={menu.session.group === g.group}
                      className="th-menu-indent"
                      onClick={() => {
                        dropIntoGroup(g.group)(menu.session.session);
                        setMenu(null);
                      }}
                    >
                      {menu.session.group === g.group ? "✓ " : ""}
                      {g.name}
                    </button>
                  ))}
                  {menu.session.group ? (
                    <button
                      type="button"
                      role="menuitem"
                      className="th-menu-indent"
                      onClick={() => {
                        actions.onMove(menu.session.session, null);
                        setMenu(null);
                      }}
                    >
                      Take out of group
                    </button>
                  ) : null}
                </>
              ) : null}
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  actions.onArchive(menu.session.session, true);
                  setMenu(null);
                }}
              >
                Archive
              </button>
            </>
          )}
          <button
            type="button"
            role="menuitem"
            className="is-danger"
            onClick={() => {
              setConfirm({ kind: "session", session: menu.session });
              setMenu(null);
            }}
          >
            Delete…
          </button>
        </div>
      ) : null}

      {menu?.kind === "group" ? (
        <div ref={menuRef} className="th-menu" role="menu" style={{ left: menu.x, top: menu.y }}>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setRenamingGroup(menu.group.group);
              setMenu(null);
            }}
          >
            Rename group
          </button>
          <button
            type="button"
            role="menuitem"
            className="is-danger"
            onClick={() => {
              setConfirm({ kind: "group", group: menu.group });
              setMenu(null);
            }}
          >
            Delete group…
          </button>
        </div>
      ) : null}

      {confirm?.kind === "session" ? (
        <ConfirmDialog
          title={`Delete “${confirm.session.title || "New session"}”?`}
          body="Its conversation is removed for good. Facts Thrall saved to memory stay, and so does what it cost. To keep it out of the way without losing it, archive it instead."
          confirm="Delete"
          danger
          onCancel={() => setConfirm(null)}
          onConfirm={() => {
            actions.onDelete(confirm.session.session);
            setArchived((list) => list.filter((s) => s.session !== confirm.session.session));
            setConfirm(null);
          }}
        />
      ) : null}
      {confirm?.kind === "folder" ? (
        <ConfirmDialog
          title={`Move “${confirm.session.title || "New session"}” to ${confirm.label}?`}
          body={`Thrall will work in ${confirm.folder} from now on. Files from earlier turns stay where they were made.`}
          confirm="Move"
          onCancel={() => setConfirm(null)}
          onConfirm={() => {
            actions.onMoveFolder(confirm.session.session, confirm.folder);
            setConfirm(null);
          }}
        />
      ) : null}
      {confirm?.kind === "group" ? (
        <ConfirmDialog
          title={`Delete the “${confirm.group.name}” group?`}
          body="Only the group goes. Its sessions stay, back under their folders."
          confirm="Delete group"
          danger
          onCancel={() => setConfirm(null)}
          onConfirm={() => {
            actions.onDeleteGroup(confirm.group.group);
            setConfirm(null);
          }}
        />
      ) : null}
    </nav>
  );
}
