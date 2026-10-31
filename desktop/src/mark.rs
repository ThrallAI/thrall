//! The living mark for the tray (design system: LivingMark, 15×15 grid below 48px). Drawn into
//! an RGBA buffer per frame; the same shapes as the web app's, so the tray and the window agree.

use crate::constants::{ACCENT, DANGER, INK, OK, TRAY_GRID, UNLIT};

/// What the mark shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Thinking,
    Tool,
    Waiting,
    Done,
    Failed,
}

impl Activity {
    /// Whether the mark moves in this state.
    pub fn animated(self) -> bool {
        matches!(self, Self::Thinking | Self::Tool | Self::Waiting)
    }

    /// In words, for the tray tooltip.
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Thinking => "Thinking",
            Self::Tool => "Running a tool",
            Self::Waiting => "Needs you",
            Self::Done => "Done",
            Self::Failed => "Failed",
        }
    }
}

fn t_mark(u: f32, v: f32, cell: f32) -> bool {
    let hw = 0.13f32.max(cell * 0.55);
    let bh = 0.13f32.max(cell * 0.55);
    (u.abs() < 0.56 && v > -0.58 && v < -0.58 + 2.0 * bh) || (u.abs() < hw && v > -0.58 && v < 0.62)
}

fn segment(u: f32, v: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((u - a.0) * dx + (v - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (u - a.0 - t * dx).hypot(v - a.1 - t * dy)
}

/// Colour and strength of the dot at u,v (-1..1) at time t seconds; `None` is an unlit dot.
fn dot(activity: Activity, u: f32, v: f32, t: f32, cell: f32) -> Option<([u8; 3], f32)> {
    let mark = t_mark(u, v, cell);
    let width = 0.1f32.max(cell * 0.6);
    match activity {
        Activity::Idle => mark.then_some((ACCENT, 1.0)),
        Activity::Thinking => {
            let d = u.hypot(v);
            let mut best = 0.0f32;
            for w in 0..2 {
                let r = ((t * 0.55 + w as f32 * 0.5) % 1.0) * 1.25;
                best = best.max((1.0 - (d - r).abs() / 0.16).max(0.0) * (1.0 - r / 1.4));
            }
            if best > 0.1 {
                Some((ACCENT, best))
            } else {
                mark.then_some((INK, 0.35))
            }
        }
        Activity::Tool => {
            if u.hypot(v) > 0.8 {
                let a = v.atan2(u);
                let tau = std::f32::consts::TAU;
                let head = (t * 3.2) % tau - std::f32::consts::PI;
                let diff = (head - a + 2.0 * tau) % tau;
                (diff < 2.2).then_some((ACCENT, 1.0 - diff / 2.2))
            } else {
                mark.then_some((INK, 0.6))
            }
        }
        Activity::Waiting => {
            let s = 0.45 + 0.55 * (0.5 + 0.5 * (t * 3.4).cos());
            if mark {
                Some((ACCENT, s))
            } else {
                (u.hypot(v) > 0.86).then_some((ACCENT, s * 0.6))
            }
        }
        Activity::Done => {
            let a = segment(u, v, (-0.5, 0.02), (-0.14, 0.38));
            let b = segment(u, v, (-0.14, 0.38), (0.52, -0.34));
            (a < width || b < width).then_some((OK, 1.0))
        }
        Activity::Failed => {
            let on = u.abs() < 0.46
                && v.abs() < 0.46
                && ((u - v).abs() < width || (u + v).abs() < width);
            on.then_some((DANGER, 1.0))
        }
    }
}

/// One frame as RGBA pixels, `px` by `px`.
pub fn frame(activity: Activity, t: f32, px: u32) -> Vec<u8> {
    let n = TRAY_GRID;
    let size = px as f32;
    let step = size / n as f32;
    let cell = 2.0 / n as f32;
    let radius = step * 0.42;
    let mut rgba = vec![0u8; (px * px * 4) as usize];
    for i in 0..n {
        for j in 0..n {
            let u = -1.0 + cell * (i as f32 + 0.5);
            let v = -1.0 + cell * (j as f32 + 0.5);
            if u.hypot(v) > 1.0 {
                continue;
            }
            let (rgb, alpha) = match dot(activity, u, v, t, cell) {
                Some((rgb, strength)) => (rgb, (strength.max(0.15) * 255.0) as u8),
                None => ([UNLIT[0], UNLIT[1], UNLIT[2]], UNLIT[3]),
            };
            let (cx, cy) = (step * (i as f32 + 0.5), step * (j as f32 + 0.5));
            // Paint the dot with a soft edge.
            let x0 = (cx - radius - 1.0).max(0.0) as u32;
            let x1 = ((cx + radius + 1.0) as u32).min(px - 1);
            let y0 = (cy - radius - 1.0).max(0.0) as u32;
            let y1 = ((cy + radius + 1.0) as u32).min(px - 1);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
                    let cover = (radius + 0.5 - d).clamp(0.0, 1.0);
                    if cover <= 0.0 {
                        continue;
                    }
                    let k = ((y * px + x) * 4) as usize;
                    let a = (alpha as f32 * cover) as u8;
                    if a >= rgba[k + 3] {
                        rgba[k] = rgb[0];
                        rgba[k + 1] = rgb[1];
                        rgba[k + 2] = rgb[2];
                        rgba[k + 3] = a;
                    }
                }
            }
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(rgba: &[u8], colour: [u8; 3]) -> usize {
        rgba.chunks(4)
            .filter(|p| p[3] > 128 && p[0] == colour[0] && p[1] == colour[1] && p[2] == colour[2])
            .count()
    }

    #[test]
    fn each_state_draws_its_own_colour() {
        assert!(lit(&frame(Activity::Idle, 0.0, 32), ACCENT) > 20, "the T");
        assert!(lit(&frame(Activity::Done, 1.0, 32), OK) > 10, "the tick");
        assert!(
            lit(&frame(Activity::Failed, 1.0, 32), DANGER) > 10,
            "the cross"
        );
        assert_eq!(lit(&frame(Activity::Done, 1.0, 32), DANGER), 0);
    }

    #[test]
    fn only_working_states_move() {
        assert!(Activity::Tool.animated() && Activity::Waiting.animated());
        assert!(!Activity::Idle.animated() && !Activity::Done.animated());
        assert_ne!(
            frame(Activity::Tool, 0.1, 32),
            frame(Activity::Tool, 0.9, 32)
        );
    }
}
