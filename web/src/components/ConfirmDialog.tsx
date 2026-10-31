// A yes/no question inside the app (never the browser's own dialog). Enter confirms, Esc cancels.

import { useEffect, useRef } from "react";

export function ConfirmDialog({
  title,
  body,
  confirm,
  danger,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: string;
  confirm: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const yes = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    yes.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div className="th-modal-scrim" onMouseDown={onCancel}>
      <div
        className="th-modal"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <h3 id="confirm-title">{title}</h3>
        <p>{body}</p>
        <div className="th-choices">
          <button ref={yes} type="button" className={`th-btn ${danger ? "th-btn-danger" : "th-btn-primary"}`} onClick={onConfirm}>
            {confirm}
          </button>
          <button type="button" className="th-btn" onClick={onCancel}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
