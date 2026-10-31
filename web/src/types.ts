// Names for generated protocol types the app uses everywhere.

import type { SessionInfo1 } from "./protocol";

/**
 * A session as `session.opened` and `session.listed` describe it. (The generator flattens the
 * `session.opened` reply to just its `type`, so the app reads that reply through this type.)
 */
export type Session = SessionInfo1;
