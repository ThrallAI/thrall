// Draws the app icon: the living mark at rest (T in a ring, 15×15 dots so it still reads at
// small sizes) on the dark ground, as icons/mark.svg. `npm run icons` turns it into every size.

import { mkdirSync, writeFileSync } from "node:fs";

const SIZE = 1024;
const GRID = 15;
const PAD = 120;
const GROUND = "#111018";
const ACCENT = "#8b6cff";
const UNLIT = "#2f2c40";

const cell = 2 / GRID;
const step = (SIZE - PAD * 2) / GRID;
const hw = Math.max(0.13, cell * 0.55);
const bh = Math.max(0.13, cell * 0.55);
const isT = (u, v) =>
  (Math.abs(u) < 0.56 && v > -0.58 && v < -0.58 + 2 * bh) || (Math.abs(u) < hw && v > -0.58 && v < 0.62);

const dots = [];
for (let i = 0; i < GRID; i++) {
  for (let j = 0; j < GRID; j++) {
    const u = -1 + cell * (i + 0.5);
    const v = -1 + cell * (j + 0.5);
    if (Math.hypot(u, v) > 1) continue;
    const x = PAD + step * (i + 0.5);
    const y = PAD + step * (j + 0.5);
    dots.push(`<circle cx="${x.toFixed(1)}" cy="${y.toFixed(1)}" r="${(step * 0.38).toFixed(1)}" fill="${isT(u, v) ? ACCENT : UNLIT}"/>`);
  }
}

const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${SIZE}" height="${SIZE}" viewBox="0 0 ${SIZE} ${SIZE}">
<rect width="${SIZE}" height="${SIZE}" rx="224" fill="${GROUND}"/>
${dots.join("\n")}
</svg>
`;
mkdirSync(new URL("../icons/", import.meta.url), { recursive: true });
writeFileSync(new URL("../icons/mark.svg", import.meta.url), svg);
console.log(`icons/mark.svg: ${dots.length} dots`);
