// Thrall's logo: a 25×25 dot grid that shows what Thrall is doing (design system: LivingMark).
// Below 48px it uses the 15×15 drawing, since 25 dots blur together at icon sizes.

import { useEffect, useRef } from "react";

import type { Activity } from "../transcript";

const LARGE_GRID = 25;
const SMALL_GRID = 15;
const SMALL_BELOW_PX = 48;
const DOT_RADIUS = 0.36;

type Dot = ["off" | "ink" | "acc" | "ok" | "err", number];

/** The resting shape: T in a ring. */
function tMark(u: number, v: number, cell: number): boolean {
  const hw = Math.max(0.13, cell * 0.55);
  const bh = Math.max(0.13, cell * 0.55);
  return (
    (Math.abs(u) < 0.56 && v > -0.58 && v < -0.58 + 2 * bh) ||
    (Math.abs(u) < hw && v > -0.58 && v < 0.62)
  );
}

function seg(u: number, v: number, ax: number, ay: number, bx: number, by: number): [number, number] {
  const dx = bx - ax;
  const dy = by - ay;
  const t = Math.max(0, Math.min(1, ((u - ax) * dx + (v - ay) * dy) / (dx * dx + dy * dy)));
  return [Math.hypot(u - ax - t * dx, v - ay - t * dy), t];
}

/** Each state: what a dot at u,v shows at time t (seconds since the state began). */
function dot(activity: Activity, u: number, v: number, t: number, cell: number, still: boolean): Dot {
  const mark = tMark(u, v, cell);
  switch (activity) {
    case "idle":
      return mark ? ["acc", 0.8 + 0.2 * Math.sin(t * 1.2)] : ["off", 1];
    case "thinking": {
      const d = Math.hypot(u, v);
      let best = 0;
      for (let w = 0; w < 2; w++) {
        const r = ((t * 0.55 + w * 0.5) % 1) * 1.25;
        best = Math.max(best, Math.max(0, 1 - Math.abs(d - r) / 0.12) * (1 - r / 1.4));
      }
      if (best > 0.08) return ["acc", best];
      return mark ? ["ink", 0.28] : ["off", 1];
    }
    case "tool": {
      if (Math.hypot(u, v) > 0.84) {
        const a = Math.atan2(v, u);
        const head = ((t * 3.2) % (Math.PI * 2)) - Math.PI;
        const diff = (head - a + Math.PI * 4) % (Math.PI * 2);
        return diff < 2.2 ? ["acc", 1 - diff / 2.2] : ["off", 1];
      }
      return mark ? ["ink", 0.55] : ["off", 1];
    }
    case "writing": {
      const rows = [-0.45, -0.2, 0.05, 0.3, 0.55];
      const lens = [0.9, 0.7, 0.85, 0.55, 0.75];
      const p = (t * 0.9) % (rows.length + 1);
      for (let i = 0; i < rows.length; i++) {
        if (Math.abs(v - (rows[i] ?? 0)) > cell * 0.55) continue;
        const start = -0.75;
        const end = start + (lens[i] ?? 0) * 1.5;
        const line = Math.floor(p);
        const fill = i < line ? end : i === line ? start + (p % 1) * (end - start) : start - 1;
        if (u < start || u > end || u > fill) return ["off", 1];
        return Math.abs(u - fill) < cell && i === line ? ["acc", 1] : ["ink", 0.85];
      }
      return ["off", 1];
    }
    case "waiting": {
      const s = 0.45 + 0.55 * (0.5 + 0.5 * Math.cos(t * 3.4));
      if (mark) return ["acc", s];
      return Math.hypot(u, v) > 0.9 ? ["acc", s * 0.6] : ["off", 1];
    }
    case "done": {
      const a = seg(u, v, -0.5, 0.02, -0.14, 0.38);
      const b = seg(u, v, -0.14, 0.38, 0.52, -0.34);
      const progress = still ? 1 : Math.min(1, t / 0.6);
      const la = 0.51;
      const lb = 0.98;
      const reach = progress * (la + lb);
      const width = Math.max(0.1, cell * 0.6);
      if (a[0] < width && a[1] * la <= reach) return ["ok", 1];
      if (b[0] < width && la + b[1] * lb <= reach) return ["ok", 1];
      return ["off", 1];
    }
    case "failed": {
      const on = still || t > 0.5 || Math.floor(t * 6) % 2 === 0;
      const width = Math.max(0.1, cell * 0.6);
      if (on && Math.abs(u) < 0.46 && Math.abs(v) < 0.46 && (Math.abs(u - v) < width || Math.abs(u + v) < width))
        return ["err", 1];
      return ["off", 1];
    }
  }
}

function colours(): Record<Dot[0], string> {
  const s = getComputedStyle(document.documentElement);
  const get = (n: string) => s.getPropertyValue(n).trim();
  return { off: get("--unlit"), ink: get("--ink"), acc: get("--accent"), ok: get("--ok"), err: get("--danger") };
}

export function LivingMark({ activity, size }: { activity: Activity; size: number }) {
  const canvas = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const cv = canvas.current;
    if (!cv) return;
    const ctx = cv.getContext("2d");
    if (!ctx) return;
    const n = size < SMALL_BELOW_PX ? SMALL_GRID : LARGE_GRID;
    const cell = 2 / n;
    const step = size / n;
    const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
    const began = performance.now();
    let frame = 0;
    let palette = colours();
    const theme = matchMedia("(prefers-color-scheme: light)");
    const recolour = () => {
      palette = colours();
      if (still) draw(performance.now());
    };
    theme.addEventListener("change", recolour);
    // The person's own theme choice (data-theme on <html>) changes the colours too.
    const picked = new MutationObserver(recolour);
    picked.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });

    const draw = (now: number) => {
      const dpr = window.devicePixelRatio || 1;
      if (cv.width !== Math.round(size * dpr)) {
        cv.width = Math.round(size * dpr);
        cv.height = Math.round(size * dpr);
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, size, size);
      const t = still ? 1.2 : (now - began) / 1000;
      for (let i = 0; i < n; i++) {
        for (let j = 0; j < n; j++) {
          const u = -1 + cell * (i + 0.5);
          const v = -1 + cell * (j + 0.5);
          if (Math.hypot(u, v) > 1) continue;
          const [kind, strength] = dot(activity, u, v, t, cell, still);
          ctx.globalAlpha = kind === "off" ? 1 : Math.max(0.15, strength);
          ctx.fillStyle = palette[kind];
          ctx.beginPath();
          ctx.arc(step * (i + 0.5), step * (j + 0.5), Math.max(step * DOT_RADIUS, 0.45), 0, Math.PI * 2);
          ctx.fill();
        }
      }
      ctx.globalAlpha = 1;
      if (!still) frame = requestAnimationFrame(draw);
    };
    draw(performance.now());
    return () => {
      cancelAnimationFrame(frame);
      theme.removeEventListener("change", recolour);
      picked.disconnect();
    };
  }, [activity, size]);

  return <canvas ref={canvas} style={{ width: size, height: size, display: "block", flex: "none" }} aria-hidden="true" />;
}

/** The state in words, for the label beside the mark and screen readers. */
export const ACTIVITY_LABELS: Record<Activity, string> = {
  idle: "Ready",
  thinking: "Thinking",
  tool: "Running a tool",
  writing: "Writing",
  waiting: "Needs you",
  done: "Done",
  failed: "Failed",
};
