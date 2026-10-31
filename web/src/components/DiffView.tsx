// A unified diff with added and removed lines marked (the +/- signs stay, so colour isn't the
// only cue).

import { memo } from "react";

import type { Diff } from "../protocol";

function lineClass(line: string): string {
  if (line.startsWith("@@")) return "line hunk";
  if (line.startsWith("+++") || line.startsWith("---")) return "line hunk";
  if (line.startsWith("+")) return "line add";
  if (line.startsWith("-")) return "line del";
  return "line";
}

export const DiffView = memo(function DiffView({ diff }: { diff: Diff }) {
  const lines = diff.unified.replace(/\n$/, "").split("\n");
  const verb = diff.kind === "create" ? "New file" : diff.kind === "delete" ? "Deleted" : "Changed";
  return (
    <div className="th-diff" role="region" aria-label={`${verb}: ${diff.path}`}>
      <div className="th-diff-path">
        {verb} {diff.path} · +{diff.added} −{diff.removed}
      </div>
      {lines.map((line, i) => (
        <div key={i} className={lineClass(line)}>
          {line || " "}
        </div>
      ))}
    </div>
  );
});
