// Copies text to the clipboard and says so for a moment.

import { useEffect, useState } from "react";

const COPIED_MS = 1500;

export function CopyButton({ text, label = "Copy" }: { text: () => string; label?: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), COPIED_MS);
    return () => clearTimeout(t);
  }, [copied]);
  return (
    <button
      type="button"
      className="th-copy"
      onClick={() => {
        navigator.clipboard
          .writeText(text())
          .then(() => setCopied(true))
          .catch(() => setCopied(false));
      }}
    >
      {copied ? "Copied" : label}
    </button>
  );
}
