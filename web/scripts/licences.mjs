// Every npm package that ends up in the web app or its build must have a licence on this list
// (the web side's equivalent of `cargo deny`). Reads package-lock.json, so it checks exactly what
// `npm ci` installs.

import { readFileSync } from "node:fs";

const ALLOWED = new Set([
  "MIT",
  "ISC",
  "0BSD",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "Apache-2.0",
  "OFL-1.1",
  "CC0-1.0",
  "BlueOak-1.0.0",
  // Same as deny.toml; here only lightningcss, Vite's CSS minifier (build time, not shipped).
  "MPL-2.0",
  // Permissive (OSI-approved); only argparse, used at build time by the type generator.
  "Python-2.0",
]);

function ok(expr) {
  // "(MIT OR Apache-2.0)": one allowed choice is enough. "A AND B": every part must be allowed.
  const clean = expr.replace(/[()]/g, "").trim();
  if (clean.includes(" OR ")) return clean.split(" OR ").some((p) => ok(p));
  if (clean.includes(" AND ")) return clean.split(" AND ").every((p) => ok(p));
  return ALLOWED.has(clean);
}

const lock = JSON.parse(readFileSync(new URL("../package-lock.json", import.meta.url), "utf8"));
const bad = [];
for (const [path, pkg] of Object.entries(lock.packages ?? {})) {
  if (!path) continue; // the app itself
  const licence = typeof pkg.license === "string" ? pkg.license : "";
  if (!licence || !ok(licence)) bad.push(`${path.replace(/^.*node_modules\//, "")}: ${licence || "(none)"}`);
}
if (bad.length > 0) {
  console.error(`Packages with licences not on the list:\n  ${bad.join("\n  ")}`);
  process.exit(1);
}
console.log(`licences ok (${Object.keys(lock.packages ?? {}).length - 1} packages)`);
