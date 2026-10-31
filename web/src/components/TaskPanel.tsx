// The session's task list (side panel on wide screens).

import type { TaskItem } from "../protocol";

const MARK = { pending: "☐", in_progress: "◐", done: "☑" } as const;
const CLASS = { pending: "", in_progress: "is-doing", done: "is-done" } as const;

export function TaskPanel({ tasks }: { tasks: TaskItem[] }) {
  return (
    <aside className="panel" aria-label="Tasks">
      <p className="th-label" style={{ margin: "0 0 8px" }}>
        Tasks
      </p>
      <ul className="th-tasks">
        {tasks.map((t) => (
          <li key={t.id} className={CLASS[t.status]}>
            <span aria-hidden="true">{MARK[t.status]}</span>
            <span>{t.text}</span>
          </li>
        ))}
      </ul>
    </aside>
  );
}
