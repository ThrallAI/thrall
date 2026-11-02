// The connection to Thrall's engine: protocol frames over a WebSocket. Requests resolve with
// their reply; session events go to the listener. It reconnects by itself, and the app reopens
// its session from the last event it saw, so nothing stored is lost.

import {
  CLIENT_KIND,
  FILES_PATH,
  CLIENT_NAME,
  PING_INTERVAL_MS,
  PROTOCOL_VERSION,
  RECONNECT_FIRST_MS,
  RECONNECT_MAX_MS,
  REQUEST_TIMEOUT_MS,
  WS_PATH,
} from "./constants";
import type { EngineInfo, EventFrame, ReplyFrame, RequestFrame, UserInfo } from "./protocol";
import { forgetDevice, storedToken } from "./signin";

/** A request without the envelope (`v`, `id`), which this module adds. */
export type Request = RequestFrame extends infer F
  ? F extends { type: string }
    ? Omit<F, "v" | "id">
    : never
  : never;

/** A reply without the envelope. */
export type Reply = ReplyFrame extends infer F
  ? F extends { type: string }
    ? Omit<F, "v" | "re">
    : never
  : never;

/** The reply to a request of type `T`'s natural answer, picked by `type`. */
export type ReplyOf<T extends Reply["type"]> = Extract<Reply, { type: T }>;

/** The engine refused a request. */
export class EngineError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

export type ConnectionState = "connecting" | "open" | "closed";

export interface Listener {
  onEvent(frame: EventFrame): void;
  /** Called on every (re)connect with the engine's details and who this is, and when the
   * connection drops. */
  onState(state: ConnectionState, engine?: EngineInfo, user?: UserInfo): void;
}

/** Where the engine is and how to prove we may talk to it. */
export interface Endpoint {
  url: string;
  token: string;
  /** The token is this browser's own sign-in (not the page's access token). */
  device: boolean;
}

/** The engine that served this page, or the dev overrides. The page itself carries no token
 * (anyone on the computer can load it): a browser gets its own from a sign-in link. An empty
 * token means this browser isn't signed in. */
export function endpoint(): Endpoint {
  const env = import.meta.env;
  const devUrl = env.VITE_THRALL_WS as string | undefined;
  const devToken = env.VITE_THRALL_TOKEN as string | undefined;
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  const device = devToken ? null : storedToken();
  return {
    url: devUrl ?? `${scheme}//${location.host}${WS_PATH}`,
    token: devToken ?? device ?? "",
    device: device !== null,
  };
}

interface Waiting {
  resolve: (reply: Reply) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export class Engine {
  private socket: WebSocket | null = null;
  private waiting = new Map<string, Waiting>();
  private nextId = 0;
  private ping: ReturnType<typeof setInterval> | null = null;
  private retryDelay = RECONNECT_FIRST_MS;
  private stopped = false;
  private ready = false;

  constructor(
    private readonly where: Endpoint,
    private readonly listener: Listener,
  ) {}

  start(): void {
    this.stopped = false;
    this.open();
  }

  stop(): void {
    this.stopped = true;
    this.socket?.close();
  }

  /** Upload a file to attach to a message; resolves with its id. */
  async upload(file: File): Promise<{ file: string; name: string; size: number }> {
    const base = this.where.url.replace(/^ws/, "http").replace(/\/v1\/ws$/, "");
    const res = await fetch(`${base}${FILES_PATH}`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${this.where.token}`,
        "X-File-Name": encodeURIComponent(file.name),
      },
      body: file,
    });
    const body = (await res.json().catch(() => ({}))) as { file?: string; name?: string; size?: number; error?: string };
    if (!res.ok || !body.file) {
      throw new EngineError("upload", body.error ?? `Couldn't upload ${file.name}.`);
    }
    return { file: body.file, name: body.name ?? file.name, size: body.size ?? file.size };
  }

  /** Fetch a file attached to a message, as a blob (the request needs the access token). */
  async download(file: string): Promise<Blob> {
    const base = this.where.url.replace(/^ws/, "http").replace(/\/v1\/ws$/, "");
    const res = await fetch(`${base}${FILES_PATH}/${encodeURIComponent(file)}`, {
      headers: { Authorization: `Bearer ${this.where.token}` },
    });
    if (!res.ok) throw new EngineError("download", "Couldn't load this file.");
    return res.blob();
  }

  /** Send a request; resolves with its reply, rejects with an EngineError on `error`. */
  request<T extends Reply["type"] = Reply["type"]>(request: Request): Promise<ReplyOf<T>> {
    if (!this.ready && request.type !== "hello") {
      return Promise.reject(new EngineError("closed", "Not connected to Thrall's engine yet."));
    }
    return this.send(request) as Promise<ReplyOf<T>>;
  }

  private send(request: Request): Promise<Reply> {
    const id = `w-${++this.nextId}`;
    const frame = { v: PROTOCOL_VERSION, id, ...request };
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiting.delete(id);
        reject(new EngineError("timeout", "Thrall's engine didn't answer in time."));
      }, REQUEST_TIMEOUT_MS);
      this.waiting.set(id, { resolve, reject, timer });
      try {
        this.socket?.send(JSON.stringify(frame));
      } catch {
        clearTimeout(timer);
        this.waiting.delete(id);
        reject(new EngineError("closed", "Lost the connection to Thrall's engine."));
      }
    });
  }

  private open(): void {
    this.listener.onState("connecting");
    let socket: WebSocket;
    try {
      socket = new WebSocket(this.where.url);
    } catch {
      this.retry();
      return;
    }
    this.socket = socket;
    socket.onopen = () => {
      this.send({
        type: "hello",
        protocols: [PROTOCOL_VERSION],
        client: { kind: CLIENT_KIND, name: CLIENT_NAME, version: "0.1.0" },
        token: this.where.token,
      })
        .then((reply) => {
          if (reply.type !== "welcome") throw new EngineError("invalid", "Unexpected answer.");
          this.ready = true;
          this.retryDelay = RECONNECT_FIRST_MS;
          this.ping = setInterval(() => {
            this.send({ type: "ping" }).catch(() => undefined);
          }, PING_INTERVAL_MS);
          this.listener.onState("open", reply.engine, reply.user);
        })
        .catch((e: unknown) => {
          // This browser was signed out: forget its sign-in and show how to sign in again.
          if (this.where.device && e instanceof EngineError && e.code === "unauthorized") {
            forgetDevice();
            location.reload();
            return;
          }
          socket.close();
        });
    };
    socket.onmessage = (msg) => this.receive(String(msg.data));
    socket.onclose = () => {
      this.ready = false;
      if (this.ping) clearInterval(this.ping);
      this.ping = null;
      for (const [, w] of this.waiting) {
        clearTimeout(w.timer);
        w.reject(new EngineError("closed", "Lost the connection to Thrall's engine."));
      }
      this.waiting.clear();
      this.listener.onState("closed");
      this.retry();
    };
  }

  private retry(): void {
    if (this.stopped) return;
    const delay = this.retryDelay;
    this.retryDelay = Math.min(this.retryDelay * 2, RECONNECT_MAX_MS);
    setTimeout(() => this.open(), delay);
  }

  private receive(text: string): void {
    let frame: Record<string, unknown>;
    try {
      frame = JSON.parse(text) as Record<string, unknown>;
    } catch {
      return;
    }
    if (typeof frame.re === "string") {
      const w = this.waiting.get(frame.re);
      if (!w) return;
      this.waiting.delete(frame.re);
      clearTimeout(w.timer);
      const reply = frame as unknown as Reply;
      if (reply.type === "error") w.reject(new EngineError(reply.code, reply.message));
      else w.resolve(reply);
      return;
    }
    if (typeof frame.session === "string" && typeof frame.type === "string") {
      this.listener.onEvent(frame as unknown as EventFrame);
    }
  }
}
