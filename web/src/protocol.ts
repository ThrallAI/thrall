/* Generated from docs/protocol.schema.json by npm run gen:types. Do not edit. */

/**
 * Frames clients send.
 */
export type RequestFrame = {
  /**
   * Request id, unique per connection; the reply quotes it as `re`.
   */
  id: string;
  /**
   * Protocol major version.
   */
  v: number;
} & (
  | {
      client: ClientInfo;
      /**
       * Protocol majors the client speaks.
       */
      protocols: number[];
      /**
       * Access token (local mode: read from the data folder).
       */
      token: string;
      type: "hello";
    }
  | {
      type: "ping";
    }
  | {
      /**
       * Starting permission mode (`ask` · `auto_edits` · `plan` · `full_auto`).
       */
      mode?: string | null;
      /**
       * Title (Thrall names it after the first turn otherwise).
       */
      title?: string | null;
      type: "session.create";
      /**
       * Folder the session works in.
       */
      workspace?: string | null;
    }
  | {
      /**
       * Last `seq` the client already has.
       */
      after_seq?: number | null;
      /**
       * Session id.
       */
      session: string;
      type: "session.open";
    }
  | {
      /**
       * Session id.
       */
      session: string;
      type: "session.detach";
    }
  | {
      /**
       * List archived sessions instead (most recently archived first).
       */
      archived?: boolean;
      /**
       * How many.
       */
      limit?: number | null;
      type: "session.list";
    }
  | {
      /**
       * True to archive, false to bring it back.
       */
      archived: boolean;
      /**
       * Session id.
       */
      session: string;
      type: "session.archive";
    }
  | {
      /**
       * Session id.
       */
      session: string;
      /**
       * New title.
       */
      title: string;
      type: "session.rename";
    }
  | {
      /**
       * Minutes, for `until: time`.
       */
      minutes?: number | null;
      /**
       * `ask` · `auto_edits` · `plan` · `full_auto`.
       */
      mode: string;
      /**
       * Session id.
       */
      session: string;
      type: "session.set_mode";
      /**
       * For `full_auto` only, how long it lasts: `task_end` (the default: back to Ask when the
       * task finishes) · `time` (after `minutes`) · `manual` (until switched off). Full auto
       * never lasts past a restart of Thrall.
       */
      until?: string | null;
    }
  | {
      /**
       * True to pin.
       */
      pinned: boolean;
      /**
       * Session id.
       */
      session: string;
      type: "session.pin";
    }
  | {
      /**
       * Group id; absent to take it out of its group.
       */
      group?: string | null;
      /**
       * Session id.
       */
      session: string;
      type: "session.move";
    }
  | {
      /**
       * The folder's full path.
       */
      folder: string;
      /**
       * Session id.
       */
      session: string;
      type: "session.set_folder";
    }
  | {
      type: "group.list";
    }
  | {
      /**
       * Its name.
       */
      name: string;
      type: "group.create";
    }
  | {
      /**
       * Group id.
       */
      group: string;
      /**
       * New name.
       */
      name: string;
      type: "group.rename";
    }
  | {
      /**
       * Group id.
       */
      group: string;
      type: "group.delete";
    }
  | {
      /**
       * Group ids, first to last.
       */
      groups: string[];
      type: "group.reorder";
    }
  | {
      /**
       * Session id.
       */
      session: string;
      type: "session.delete";
    }
  | {
      type: "home.get";
    }
  | {
      type: "home.clear";
    }
  | {
      type: "heartbeat.get";
    }
  | {
      /**
       * The checklist, as markdown.
       */
      checklist: string;
      type: "heartbeat.set";
    }
  | {
      type: "heartbeat.run";
    }
  | {
      type: "startup.get";
    }
  | {
      /**
       * Start at sign-in.
       */
      on: boolean;
      type: "startup.set";
    }
  | {
      /**
       * Ids of files uploaded with `POST /v1/files` to attach.
       */
      files?: string[];
      /**
       * Session id.
       */
      session: string;
      /**
       * What the user typed (`@path` mentions included).
       */
      text: string;
      type: "turn.send";
    }
  | {
      /**
       * Session id.
       */
      session: string;
      /**
       * The turn to go back to before.
       */
      turn: string;
      type: "checkpoint.preview";
    }
  | {
      /**
       * Take the conversation back.
       */
      conversation: boolean;
      /**
       * Put files back.
       */
      files: boolean;
      /**
       * Files (as `checkpoint.shown` names them) to leave as they are.
       */
      keep?: string[];
      /**
       * Session id.
       */
      session: string;
      /**
       * The turn to go back to before.
       */
      turn: string;
      type: "turn.rewind";
    }
  | {
      /**
       * Session id.
       */
      session: string;
      type: "turn.retry";
    }
  | {
      /**
       * Session id.
       */
      session: string;
      /**
       * The new message.
       */
      text: string;
      type: "turn.edit";
    }
  | {
      /**
       * Session id.
       */
      session: string;
      type: "turn.interrupt";
    }
  | {
      /**
       * The answer.
       */
      choice: "yes" | "no" | "always";
      /**
       * Request id.
       */
      request: string;
      type: "permission.answer";
    }
  | {
      /**
       * Whether to go ahead.
       */
      approve: boolean;
      /**
       * Approve and let file edits go ahead without asking.
       */
      auto_edits?: boolean;
      /**
       * Plan id.
       */
      plan: string;
      type: "plan.decide";
    }
  | {
      type: "session.watch_all";
    }
  | {
      type: "engine.stop";
    }
  | {
      /**
       * The upload's id.
       */
      file: string;
      /**
       * Session the cost belongs to, if any.
       */
      session?: string | null;
      type: "audio.transcribe";
    }
  | {
      type: "command.list";
    }
  | {
      /**
       * Everything after the name.
       */
      args?: string;
      /**
       * Name without the slash.
       */
      name: string;
      /**
       * Session it applies to, if any.
       */
      session?: string | null;
      type: "command.run";
    }
  | {
      type: "memory.list";
    }
  | {
      /**
       * The fact's id.
       */
      fact: string;
      type: "memory.forget";
    }
  | {
      /**
       * The skill's name.
       */
      name: string;
      type: "skill.remove";
    }
  | {
      type: "settings.get";
    }
  | {
      /**
       * Dotted key, e.g. `agent.verify` or `provider.model`.
       */
      key: string;
      type: "settings.set";
      /**
       * The new value.
       */
      value: boolean | number | string;
    }
  | {
      type: "phone.list";
    }
  | {
      /**
       * Who the phone is for (a person's id); absent: the owner.
       */
      person?: string | null;
      type: "phone.pair";
    }
  | {
      /**
       * The person's id.
       */
      person: string;
      /**
       * The phone's id.
       */
      phone: string;
      type: "phone.assign";
    }
  | {
      /**
       * The phone's id.
       */
      phone: string;
      type: "phone.remove";
    }
  | {
      /**
       * Count each person's spending from this moment (Unix ms), e.g. the start of this month
       * where the asker is; absent: all of it.
       */
      since?: number | null;
      type: "people.list";
    }
  | {
      /**
       * Their name.
       */
      name: string;
      type: "people.add";
    }
  | {
      /**
       * Their new name.
       */
      name: string;
      /**
       * Their id.
       */
      person: string;
      type: "people.rename";
    }
  | {
      /**
       * Their own model; absent: the engine's. Not for the owner (who changes the engine's).
       */
      model?: string | null;
      /**
       * The most their turns may cost in a calendar month, in US dollars; absent: no limit.
       */
      monthly_limit?: number | null;
      /**
       * Their turns use their own AI key (not the owner's). Not for the owner.
       */
      own_key?: boolean;
      /**
       * Their id.
       */
      person: string;
      type: "people.terms";
    }
  | {
      /**
       * The key; absent or empty: forget it.
       */
      key?: string | null;
      /**
       * Whose; absent: yours.
       */
      person?: string | null;
      type: "people.key";
    }
  | {
      /**
       * Their id.
       */
      person: string;
      type: "people.remove";
    }
  | {
      /**
       * Their id.
       */
      person: string;
      type: "signin.create";
    }
  | {
      /**
       * Whose; absent: yours.
       */
      person?: string | null;
      type: "device.list";
    }
  | {
      /**
       * The device's id.
       */
      device: string;
      /**
       * Whose; absent: yours.
       */
      person?: string | null;
      type: "device.remove";
    }
  | {
      /**
       * This piece, base64.
       */
      data: string;
      /**
       * The file's name (used with the first piece).
       */
      name?: string;
      type: "file.put";
      /**
       * The upload to add to (from an earlier `file.saved`); absent for the first piece.
       */
      upload?: string | null;
    }
  | {
      type: "folder.places";
    }
  | {
      /**
       * Path inside it; empty for the folder itself.
       */
      path?: string;
      /**
       * The session whose folder it is.
       */
      session: string;
      type: "folder.list";
    }
  | {
      /**
       * Where to start, in bytes.
       */
      offset?: number;
      /**
       * Path inside it.
       */
      path: string;
      /**
       * The session whose folder it is.
       */
      session: string;
      type: "folder.read";
    }
  | {
      /**
       * The file's id.
       */
      file: string;
      /**
       * Where to start, in bytes.
       */
      offset?: number;
      type: "file.get";
    }
  | {
      /**
       * Its name.
       */
      name: string;
      /**
       * Where to make it (from `folder.places`).
       */
      parent: string;
      type: "folder.create";
    }
  | {
      type: "app.latest";
    }
  | {
      /**
       * Where to start, in bytes.
       */
      offset?: number;
      type: "app.get";
    }
);
/**
 * Answers a request.
 */
export type ReplyFrame = {
  /**
   * The request id this answers.
   */
  re: string;
  /**
   * Protocol major version.
   */
  v: number;
} & (
  | {
      /**
       * Features this engine has, so clients don't guess.
       */
      capabilities: string[];
      engine: EngineInfo;
      limits: Limits;
      /**
       * The protocol major both sides will speak.
       */
      protocol: number;
      type: "welcome";
      user: UserInfo;
    }
  | {
      type: "pong";
    }
  | {
      type: "ack";
    }
  | {
      /**
       * Stable code clients switch on.
       */
      code:
        | "unauthorized"
        | "unsupported"
        | "invalid"
        | "not_found"
        | "busy"
        | "already_answered"
        | "unknown_command"
        | "rate_limited"
        | "provider_unavailable"
        | "provider_key_missing"
        | "provider_key_rejected"
        | "provider_rejected"
        | "too_large"
        | "internal";
      /**
       * Plain-language explanation and what to do.
       */
      message: string;
      /**
       * Whether trying again may work.
       */
      retry: boolean;
      type: "error";
    }
  | SessionInfo
  | {
      /**
       * Newest first.
       */
      items: SessionInfo1[];
      type: "session.listed";
    }
  | {
      /**
       * The groups.
       */
      items: GroupInfo[];
      type: "group.listed";
    }
  | {
      /**
       * Earlier pages, newest first.
       */
      earlier: SessionInfo1[];
      page: SessionInfo2;
      type: "home.shown";
    }
  | {
      /**
       * The checklist ("" when there isn't one).
       */
      checklist: string;
      /**
       * When the next check-in is due, Unix ms (absent while check-ins are off).
       */
      next_run?: number | null;
      type: "heartbeat.shown";
    }
  | {
      /**
       * This system can start Thrall at sign-in (the desktop app is installed beside the engine).
       */
      available: boolean;
      /**
       * What the setting is called on the engine's system ("Start with Windows", "Open at
       * login", "Start when I log in"); empty from an older engine.
       */
      label?: string;
      /**
       * Thrall starts at sign-in.
       */
      on: boolean;
      type: "startup.shown";
    }
  | {
      /**
       * The commands.
       */
      items: CommandItem[];
      type: "command.listed";
    }
  | {
      /**
       * What to show.
       */
      markdown: string;
      type: "command.result";
    }
  | {
      /**
       * The words.
       */
      text: string;
      type: "audio.transcribed";
    }
  | {
      /**
       * The files a rewind would put back, in the order they were first changed.
       */
      files: RewindFile[];
      /**
       * What the user typed to start it (their messages in it, joined).
       */
      text: string;
      /**
       * The turn.
       */
      turn: string;
      type: "checkpoint.shown";
    }
  | {
      /**
       * Files that couldn't be put back, each with the reason.
       */
      failed: string[];
      /**
       * Files put back (or removed, if the turn made them).
       */
      restored: string[];
      /**
       * What the user typed to start the turn, to put back in the message box.
       */
      text: string;
      type: "turn.rewound";
    }
  | {
      /**
       * Newest first.
       */
      facts: FactInfo[];
      type: "memory.listed";
    }
  | {
      /**
       * `config.toml` as JSON, with values that could be secrets (MCP server environment
       * variables that aren't keychain references) replaced by `"••••"`.
       */
      config: {
        [k: string]: unknown;
      };
      /**
       * Where the file is, if the engine has a data folder.
       */
      config_file?: string | null;
      /**
       * Keys Thrall knows about and whether each is stored.
       */
      keys: KeyStatus[];
      /**
       * MCP servers on the web that Thrall is signed in to (`/mcp login <name>`).
       */
      mcp_signed_in?: string[];
      /**
       * The model turns use now (`/model` can differ from the file until restart).
       */
      model: string;
      /**
       * Saved settings not in use yet: they apply when Thrall finishes what it's doing.
       */
      pending?: boolean;
      /**
       * Chat models the person's ChatGPT plan offers (`chatgpt/<model>`), when signed in.
       */
      plan_models?: string[];
      /**
       * Skills Thrall can use (in the session folder `settings.get` was asked about, or only
       * the person's own), including ones turned off.
       */
      skills?: SkillInfo[];
      type: "settings.shown";
    }
  | {
      /**
       * Paired phones, newest first.
       */
      items: PhoneInfo[];
      remote: RemoteInfo;
      type: "phone.listed";
    }
  | {
      /**
       * When the code stops working, Unix ms.
       */
      expires: number;
      /**
       * The pairing link the QR holds (`thrall://pair?…`); it contains a secret, so it's shown
       * only on this computer.
       */
      link: string;
      /**
       * The QR code's rows, top to bottom: `1` dark, `0` light (no quiet zone).
       */
      qr: string[];
      type: "phone.pairing";
    }
  | {
      /**
       * Everyone, the owner first, then oldest first.
       */
      items: PersonInfo[];
      type: "people.listed";
    }
  | {
      /**
       * The one-time code (it signs someone in: show it only to them).
       */
      code: string;
      /**
       * When it stops working, Unix ms.
       */
      expires: number;
      /**
       * Who it signs in as.
       */
      name: string;
      type: "signin.code";
    }
  | {
      /**
       * Signed-in browsers and PCs, newest first.
       */
      items: DeviceInfo[];
      /**
       * Whose devices.
       */
      person: string;
      type: "device.listed";
    }
  | {
      /**
       * Its id, for `turn.send {files}` and further pieces.
       */
      file: string;
      /**
       * The name it was saved under.
       */
      name: string;
      /**
       * Bytes so far.
       */
      size: number;
      type: "file.saved";
    }
  | {
      /**
       * Thrall's own folder for sessions started without one picked (`Documents\Thrall`).
       */
      default?: string | null;
      /**
       * Where a new folder goes unless another is picked (`folders.new_in`, else `default`).
       */
      new_in?: string | null;
      /**
       * Folders sessions use, most recently active first.
       */
      recent: string[];
      type: "folder.places";
    }
  | {
      /**
       * The new folder's full path.
       */
      path: string;
      type: "folder.created";
    }
  | {
      /**
       * The phone app this engine holds; absent when it holds none.
       */
      release?: AppRelease | null;
      type: "app.latest";
    }
  | {
      /**
       * Folders first, then files, each newest first.
       */
      items: FolderEntry[];
      /**
       * Some entries were left out (a very full folder).
       */
      more?: boolean;
      /**
       * The folder, relative to the session's folder ("" for the folder itself).
       */
      path: string;
      /**
       * The session.
       */
      session: string;
      type: "folder.listed";
    }
  | {
      /**
       * This piece, base64.
       */
      data: string;
      /**
       * Where this piece starts.
       */
      offset: number;
      /**
       * The file, relative to the session's folder.
       */
      path: string;
      /**
       * The session.
       */
      session: string;
      /**
       * The whole file's size; the last piece ends there.
       */
      size: number;
      type: "folder.part";
    }
  | {
      /**
       * This piece, base64.
       */
      data: string;
      /**
       * The file's id.
       */
      file: string;
      /**
       * Its name.
       */
      name: string;
      /**
       * Where this piece starts.
       */
      offset: number;
      /**
       * The whole file's size; the last piece ends there.
       */
      size: number;
      type: "file.part";
    }
);
/**
 * Something that happened in a session.
 */
export type EventFrame = {
  /**
   * When it happened, Unix milliseconds.
   */
  at: number;
  /**
   * Position in the session's event log. Absent on live-only events (deltas), which are never
   * stored: after a reconnect the client gets the matching `message.completed` instead.
   */
  seq?: number | null;
  /**
   * Session the event belongs to.
   */
  session: string;
  /**
   * Protocol major version.
   */
  v: number;
} & (
  | {
      /**
       * Nobody started it: a scheduled job or a check-in (clients don't announce it as the
       * person's own work finishing).
       */
      scheduled?: boolean;
      /**
       * Turn id.
       */
      turn: string;
      type: "turn.started";
    }
  | {
      /**
       * How it ended.
       */
      status: "done" | "interrupted" | "failed";
      /**
       * Turn id.
       */
      turn: string;
      type: "turn.completed";
      usage: Usage;
    }
  | {
      /**
       * Turn id.
       */
      turn: string;
      type: "turn.retracted";
    }
  | {
      /**
       * Message id.
       */
      message: string;
      /**
       * Who wrote it.
       */
      role: "user" | "assistant";
      /**
       * Turn id.
       */
      turn: string;
      type: "message.started";
    }
  | {
      /**
       * Message id.
       */
      message: string;
      /**
       * Text to append.
       */
      text: string;
      type: "message.delta";
    }
  | {
      /**
       * Message id.
       */
      message: string;
      /**
       * Text to append.
       */
      text: string;
      type: "reasoning.delta";
    }
  | {
      /**
       * Files attached to it (user messages).
       */
      files?: FileRef[];
      /**
       * Message id.
       */
      message: string;
      /**
       * Full text; clients replace what they streamed with this.
       */
      text: string;
      type: "message.completed";
    }
  | {
      /**
       * Step id.
       */
      step: string;
      /**
       * Plain-language title: "Reading src/main.rs".
       */
      title: string;
      /**
       * Tool name.
       */
      tool: string;
      /**
       * Turn id.
       */
      turn: string;
      type: "step.started";
    }
  | {
      /**
       * Step id.
       */
      step: string;
      /**
       * Text to append.
       */
      text: string;
      type: "step.output";
    }
  | {
      /**
       * The change, for file edits.
       */
      diff?: Diff | null;
      /**
       * Files the step made (pictures), fetched with `GET /v1/files/{file}`.
       */
      files?: FileRef[];
      /**
       * How it went.
       */
      status: "ok" | "error" | "warn" | "denied" | "cancelled";
      /**
       * Step id.
       */
      step: string;
      /**
       * One-line plain-language result.
       */
      summary: string;
      type: "step.completed";
    }
  | {
      /**
       * The tasks, in order.
       */
      items: TaskItem[];
      type: "tasks.updated";
    }
  | {
      /**
       * The answers on offer.
       */
      choices: PermissionChoice[];
      /**
       * The change, for file edits (accept is `yes`, reject is `no`).
       */
      diff?: Diff | null;
      /**
       * Request id; answers quote it.
       */
      request: string;
      /**
       * How risky.
       */
      risk: "low" | "medium" | "high";
      /**
       * The step waiting on the answer.
       */
      step: string;
      /**
       * Plain language: "Run `cargo test`".
       */
      summary: string;
      /**
       * Tool name.
       */
      tool: string;
      /**
       * Turn id.
       */
      turn: string;
      type: "permission.request";
    }
  | {
      /**
       * The plan.
       */
      markdown: string;
      /**
       * Plan id; decisions quote it.
       */
      plan: string;
      type: "plan.proposed";
    }
  | {
      /**
       * Which client decided.
       */
      answered_by: string;
      /**
       * Whether it was approved.
       */
      approved: boolean;
      /**
       * Plan id.
       */
      plan: string;
      type: "plan.resolved";
    }
  | {
      /**
       * Which client answered.
       */
      answered_by: string;
      /**
       * An answer to a permission request.
       */
      choice: "yes" | "no" | "always";
      /**
       * Request id.
       */
      request: string;
      type: "permission.resolved";
    }
  | {
      /**
       * New permission mode (`ask` · `auto_edits` · `plan` · `full_auto`).
       */
      mode?: string | null;
      /**
       * How long Full auto lasts (sent with `mode: full_auto`).
       */
      mode_limit?: ModeLimit | null;
      /**
       * New title.
       */
      title?: string | null;
      type: "session.updated";
      /**
       * New folder the session works in.
       */
      workspace?: string | null;
    }
  | {
      /**
       * Helper id.
       */
      agent: string;
      /**
       * What it's doing.
       */
      title: string;
      type: "agent.started";
    }
  | {
      /**
       * Helper id.
       */
      agent: string;
      /**
       * `done` · `failed`.
       */
      status: string;
      /**
       * One line about the result.
       */
      summary: string;
      /**
       * What it was doing.
       */
      title: string;
      type: "agent.completed";
    }
  | {
      /**
       * Model the session's turns use.
       */
      model: string;
      /**
       * Idle, working, or waiting for an answer.
       */
      state: "idle" | "working" | "waiting_for_you";
      type: "status";
    }
  | {
      /**
       * The fact.
       */
      content: string;
      /**
       * The fact's id.
       */
      fact: string;
      /**
       * Its topic.
       */
      topic: string;
      type: "memory.saved";
    }
  | {
      /**
       * How prominent.
       */
      level: "info" | "warning";
      /**
       * Plain-language text.
       */
      text: string;
      type: "notice";
    }
  | {
      /**
       * Stable code clients switch on.
       */
      code:
        | "unauthorized"
        | "unsupported"
        | "invalid"
        | "not_found"
        | "busy"
        | "already_answered"
        | "unknown_command"
        | "rate_limited"
        | "provider_unavailable"
        | "provider_key_missing"
        | "provider_key_rejected"
        | "provider_rejected"
        | "too_large"
        | "internal";
      /**
       * Plain-language explanation and what to do.
       */
      message: string;
      /**
       * Whether trying again may work.
       */
      retry: boolean;
      type: "error";
    }
);
/**
 * An answer to a permission request.
 */
export type PermissionChoice = "yes" | "no" | "always";

/**
 * Every frame in both directions, as one schema root (for generating client types).
 */
export interface Protocol {
  client: RequestFrame;
  /**
   * Frames the engine sends.
   */
  server: ReplyFrame | EventFrame;
}
/**
 * Who is connecting.
 */
export interface ClientInfo {
  /**
   * What sort of client.
   */
  kind: "tui" | "web" | "desktop" | "android" | "telegram" | "headless";
  /**
   * Display name ("Thrall Terminal").
   */
  name: string;
  /**
   * Client version.
   */
  version: string;
}
/**
 * About the engine.
 */
export interface EngineInfo {
  /**
   * `home` (free, for a household). Other editions may come later.
   */
  edition?: string;
  /**
   * The model turns use now (`status` events report changes).
   */
  model?: string;
  /**
   * Engine version; a client built for another version may offer to restart the engine.
   */
  version: string;
}
/**
 * Limits the client must respect.
 */
export interface Limits {
  /**
   * Largest frame, in bytes.
   */
  frame_bytes: number;
}
/**
 * Who the engine acts for.
 */
export interface UserInfo {
  /**
   * User id.
   */
  id: string;
  /**
   * Display name.
   */
  name: string;
  /**
   * `owner` (runs this engine) or `member`.
   */
  role?: string;
}
/**
 * A session is open on this connection.
 */
export interface SessionInfo {
  type: "session.opened";
}
/**
 * A session, as `session.opened` and `session.listed` describe it.
 */
export interface SessionInfo1 {
  /**
   * Money spent in this session so far, in US dollars.
   */
  cost_usd?: number;
  /**
   * The person's group it's in (a group id), if any.
   */
  group?: string | null;
  /**
   * A page of Home (built in: it can't be renamed, archived, pinned, moved or deleted).
   */
  home?: boolean;
  /**
   * Last event `seq` stored for it.
   */
  last_seq: number;
  /**
   * Permission mode (`ask` · `auto_edits` · `plan` · `full_auto`).
   */
  mode: string;
  /**
   * How long Full auto lasts, when `mode` is `full_auto`.
   */
  mode_limit?: ModeLimit | null;
  /**
   * Pinned to the top of the list.
   */
  pinned?: boolean;
  /**
   * Session id.
   */
  session: string;
  /**
   * What it's doing now.
   */
  state: "idle" | "working" | "waiting_for_you";
  /**
   * Title.
   */
  title: string;
  /**
   * Last activity, Unix milliseconds.
   */
  updated: number;
  /**
   * Folder it works in.
   */
  workspace?: string | null;
}
/**
 * How long Full auto lasts in a session.
 */
export interface ModeLimit {
  /**
   * When it goes back to Ask, Unix milliseconds (`time` only).
   */
  ends_at?: number | null;
  /**
   * `task_end` (back to Ask when the task finishes) · `time` (back to Ask at `ends_at`) ·
   * `manual` (until switched off, or Thrall restarts).
   */
  until: string;
}
/**
 * One of the person's session groups.
 */
export interface GroupInfo {
  /**
   * Group id.
   */
  group: string;
  /**
   * Name.
   */
  name: string;
}
/**
 * A session, as `session.opened` and `session.listed` describe it.
 */
export interface SessionInfo2 {
  /**
   * Money spent in this session so far, in US dollars.
   */
  cost_usd?: number;
  /**
   * The person's group it's in (a group id), if any.
   */
  group?: string | null;
  /**
   * A page of Home (built in: it can't be renamed, archived, pinned, moved or deleted).
   */
  home?: boolean;
  /**
   * Last event `seq` stored for it.
   */
  last_seq: number;
  /**
   * Permission mode (`ask` · `auto_edits` · `plan` · `full_auto`).
   */
  mode: string;
  /**
   * How long Full auto lasts, when `mode` is `full_auto`.
   */
  mode_limit?: ModeLimit | null;
  /**
   * Pinned to the top of the list.
   */
  pinned?: boolean;
  /**
   * Session id.
   */
  session: string;
  /**
   * What it's doing now.
   */
  state: "idle" | "working" | "waiting_for_you";
  /**
   * Title.
   */
  title: string;
  /**
   * Last activity, Unix milliseconds.
   */
  updated: number;
  /**
   * Folder it works in.
   */
  workspace?: string | null;
}
/**
 * A slash command, for autocomplete.
 */
export interface CommandItem {
  /**
   * Arguments hint ("[model id]").
   */
  args_hint: string;
  /**
   * Name without the slash.
   */
  name: string;
  /**
   * It's a skill or a built-in Thrall carries out itself (`/humanise`): clients send
   * `/name …` as a message, and Thrall does it.
   */
  skill?: boolean;
  /**
   * What it does.
   */
  summary: string;
}
/**
 * A file a rewind would put back.
 */
export interface RewindFile {
  /**
   * What happens to it.
   */
  action: "restore" | "delete" | "unavailable";
  /**
   * Someone changed it after Thrall last did (or it's gone): putting it back loses that.
   */
  changed?: boolean;
  /**
   * The file, relative to the session folder when inside it.
   */
  path: string;
}
/**
 * A fact Thrall remembers.
 */
export interface FactInfo {
  /**
   * When it was saved, Unix ms.
   */
  at: number;
  /**
   * The fact.
   */
  content: string;
  /**
   * Its id.
   */
  fact: string;
  /**
   * Its topic.
   */
  topic: string;
}
/**
 * Whether a key is stored (`thrall key set <name>`). Never the key itself.
 */
export interface KeyStatus {
  /**
   * Key name (`openrouter`, `fal`).
   */
  name: string;
  /**
   * What it's for.
   */
  purpose: string;
  /**
   * Whether it's stored.
   */
  set: boolean;
}
/**
 * A skill, for the settings screen.
 */
export interface SkillInfo {
  /**
   * When to use it.
   */
  description: string;
  /**
   * It's turned on.
   */
  enabled: boolean;
  /**
   * It runs in a helper.
   */
  helper: boolean;
  /**
   * Its name (`/name` runs it).
   */
  name: string;
  /**
   * One of the person's own (`false`: the project's).
   */
  personal: boolean;
  /**
   * Safety rating of a skill Thrall learned: `low`, `medium` or `high` (none for ones
   * written by hand).
   */
  rating?: string | null;
}
/**
 * A paired phone.
 */
export interface PhoneInfo {
  /**
   * When it was paired, Unix ms.
   */
  added: number;
  /**
   * Connected right now.
   */
  connected?: boolean;
  /**
   * When it last connected, Unix ms (absent if never since pairing).
   */
  last_seen?: number | null;
  /**
   * The name it gave when pairing.
   */
  name: string;
  /**
   * Whose phone it is (a person's id).
   */
  person?: string;
  /**
   * Its id.
   */
  phone: string;
}
/**
 * The phone link.
 */
export interface RemoteInfo {
  /**
   * The public address phones use (`remote.address`; "" when not set).
   */
  address: string;
  /**
   * Switched on in settings (`remote.enabled`).
   */
  enabled: boolean;
  /**
   * The listener is running.
   */
  listening: boolean;
  /**
   * Its port on 127.0.0.1 (what the tunnel forwards to).
   */
  port: number;
}
/**
 * A person who uses this Thrall.
 */
export interface PersonInfo {
  /**
   * When they were added, Unix ms.
   */
  added: number;
  /**
   * Their own AI key is set (it's never shown).
   */
  key_set?: boolean;
  /**
   * Their own model, if they have one.
   */
  model?: string | null;
  /**
   * Their monthly spending limit in US dollars, if they have one.
   */
  monthly_limit?: number | null;
  /**
   * Their name.
   */
  name: string;
  /**
   * Their turns use their own AI key.
   */
  own_key?: boolean;
  /**
   * Their id.
   */
  person: string;
  /**
   * How many phones are paired to them.
   */
  phones?: number;
  /**
   * `owner` (runs this engine) or `member`.
   */
  role: string;
  /**
   * What their turns have cost (USD) since `people.list`'s `since`. Only what was recorded
   * with a price counts.
   */
  spent?: number;
}
/**
 * A browser or PC signed in as a person.
 */
export interface DeviceInfo {
  /**
   * When it signed in, Unix ms.
   */
  added: number;
  /**
   * The one asking.
   */
  current?: boolean;
  /**
   * Its id.
   */
  device: string;
  /**
   * When it last connected, Unix ms.
   */
  last_seen?: number | null;
  /**
   * The name it signed in under.
   */
  name: string;
}
/**
 * A build of the phone app, offered as an update.
 */
export interface AppRelease {
  /**
   * What's new, a line or a few.
   */
  notes?: string;
  /**
   * The APK's SHA-256, hex, to check the download.
   */
  sha256: string;
  /**
   * The APK's size in bytes.
   */
  size: number;
  /**
   * Android's version number: higher is newer.
   */
  version_code: number;
  /**
   * The version people see ("0.2.0").
   */
  version_name: string;
}
/**
 * One entry in a folder listing.
 */
export interface FolderEntry {
  /**
   * It's a folder.
   */
  folder?: boolean;
  /**
   * Last changed, Unix ms.
   */
  modified?: number;
  /**
   * Its name.
   */
  name: string;
  /**
   * Size in bytes (0 for folders).
   */
  size?: number;
}
/**
 * Tokens and cost.
 */
export interface Usage {
  /**
   * Cost in US dollars, when the provider reports it.
   */
  cost_usd?: number | null;
  /**
   * Tokens sent to the model.
   */
  input_tokens: number;
  /**
   * Tokens the model wrote.
   */
  output_tokens: number;
}
/**
 * A file attached to a message. Clients fetch it with `GET /v1/files/{file}`.
 */
export interface FileRef {
  /**
   * Upload id.
   */
  file: string;
  /**
   * `image` · `document` · `text` · `other`, so clients know how to show it.
   */
  kind: string;
  /**
   * File name.
   */
  name: string;
  /**
   * Size in bytes.
   */
  size: number;
}
/**
 * A file change (protocol §8).
 */
export interface Diff {
  /**
   * Lines added.
   */
  added: number;
  /**
   * What happened to the file.
   */
  kind: "create" | "modify" | "delete";
  /**
   * File path as shown to the user.
   */
  path: string;
  /**
   * Lines removed.
   */
  removed: number;
  /**
   * Standard unified diff.
   */
  unified: string;
}
/**
 * One entry in the task list.
 */
export interface TaskItem {
  /**
   * Task id.
   */
  id: string;
  /**
   * Where it's at.
   */
  status: "pending" | "in_progress" | "done";
  /**
   * What it is.
   */
  text: string;
}
