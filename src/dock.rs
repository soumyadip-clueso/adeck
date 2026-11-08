//! The Dock icon is a 9×9 dot screen. At rest it shows the adeck mark, a lowercase a in dots.
//! While something is happening (the agent calling tools, analysis running, sound playing, or
//! the agent waiting on your verdict) the a dissolves into a live meter that wanders between a
//! waveform, a spectrum and a scope, then settles back into the a. The meter is simulated once
//! into a reel of 40 frames (4 to dissolve, 36 of meter) the first time it's needed; after that
//! animating only swaps cached images.

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mood {
    Idle,
    /// The agent is driving: a tool call in the last few seconds, or analysis running.
    Working,
    /// Sound is playing.
    Playing,
    /// The agent is blocked in wait_for_feedback.
    Waiting,
}

const N: usize = 9;
const DIM: f32 = 0.12;
const FPS: f32 = 10.0;
/// Reel frames are drawn at Dock-tile size; macOS spends more resampling a bigger image.
const LIVE_PX: u32 = 128;
/// Frames for the mark to dissolve into the meter (played backwards to settle).
const FADES: usize = 4;
/// Frames of meter: a third each on the wave, spectrum and scope, played back and forth.
const REEL: usize = 36;
// The tile at 1024: a flat rounded square.
const TILE: (f32, f32, f32) = (100.0, 924.0, 178.0);
const INK: (u8, u8, u8) = (0xf2, 0xf2, 0xf2);

/// The mark: a lowercase a.
const MARK: [&str; N] = [
    ".........",
    ".........",
    "..#####..",
    ".......#.",
    "..######.",
    ".#.....#.",
    ".#.....#.",
    "..######.",
    ".........",
];

fn mark() -> [[f32; N]; N] {
    let mut g = [[DIM; N]; N];
    for (y, row) in MARK.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            if c == b'#' {
                g[y][x] = 1.0;
            }
        }
    }
    g
}

/// The live meter: one spring per column chasing random targets, shaped by whichever chart is
/// showing, so it moves continuously instead of jumping between frames.
pub struct Meter {
    v: [f32; N],
    vel: [f32; N],
    tgt: [f32; N],
    until: [f32; N],
    peak: [f32; N],
    /// How much of each chart is showing: wave, spectrum, scope. Eased toward `mode`.
    w: [f32; 3],
    mode: usize,
    t: f32,
    next: f32,
    phase: f32,
    seed: u32,
}

impl Default for Meter {
    fn default() -> Self {
        let mut m = Meter { v: [0.2; N], vel: [0.0; N], tgt: [0.2; N], until: [0.0; N], peak: [0.0; N], w: [1.0, 0.0, 0.0], mode: 0, t: 0.0, next: 5.0, phase: 0.0, seed: 0x9e37_79b9 };
        for i in 0..N {
            m.retarget(i, 1.0);
        }
        m
    }
}

impl Meter {
    fn rnd(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    fn retarget(&mut self, i: usize, energy: f32) {
        let env = (std::f32::consts::PI * (i as f32 + 0.5) / N as f32).sin().powf(1.4);
        let slope = 1.0 - 0.7 * i as f32 / (N - 1) as f32;
        let bias = self.w[0] * env + self.w[1] * slope + self.w[2] * 0.6;
        let r = self.rnd();
        self.tgt[i] = (bias * (0.12 + 0.95 * r.powf(1.6)) * energy).min(1.0);
        let hold = self.rnd();
        self.until[i] = self.t + 0.12 + 0.4 * hold;
    }

    /// Advances by `dt` seconds. `energy` scales how loud it looks (1 at work, low while waiting);
    /// `pace` scales time (slower while waiting); `cycle` lets it wander between the charts.
    pub fn step(&mut self, dt: f32, energy: f32, pace: f32, cycle: bool) {
        let dt = dt * pace;
        self.t += dt;
        if cycle && self.t > self.next {
            self.mode = (self.mode + 1) % 3;
            let r = self.rnd();
            self.next = self.t + 4.0 + 4.0 * r;
        } else if !cycle {
            self.mode = 0;
        }
        let ease = (dt * 1.6).min(1.0);
        for k in 0..3 {
            let goal = if k == self.mode { 1.0 } else { 0.0 };
            self.w[k] += (goal - self.w[k]) * ease;
        }
        for i in 0..N {
            if self.t > self.until[i] {
                self.retarget(i, energy);
            }
        }
        let sm: [f32; N] = std::array::from_fn(|i| 0.7 * self.tgt[i] + 0.15 * self.tgt[i.saturating_sub(1)] + 0.15 * self.tgt[(i + 1).min(N - 1)]);
        for (i, sm) in sm.into_iter().enumerate() {
            let a = 110.0 * (sm - self.v[i]) - 15.0 * self.vel[i];
            self.vel[i] += a * dt;
            self.v[i] = (self.v[i] + self.vel[i] * dt).clamp(0.0, 1.0);
            self.peak[i] = self.v[i].max(self.peak[i] - dt * 0.3);
        }
        self.phase += dt * std::f32::consts::TAU * 0.6;
    }

    /// Each dot's brightness, 0..1, rows top to bottom.
    pub fn grid(&self) -> [[f32; N]; N] {
        let mut g = [[DIM; N]; N];
        let mid = (N - 1) as f32 / 2.0;
        let level = self.v.iter().sum::<f32>() / N as f32;
        let scope_c = |x: f32| mid - 3.4 * (0.45 + 0.55 * (level * 2.0).min(1.0)) * (x / (N - 1) as f32 * std::f32::consts::TAU - self.phase).sin();
        for x in 0..N {
            let h = 1.0 + (N - 1) as f32 * self.v[x];
            let (cw, cs, cc) = (mid, (N - 1) as f32 - (h - 1.0) / 2.0, scope_c(x as f32));
            // A scope trace is one dot thick, thicker where it's steep so it stays joined.
            let hs = 1.15 + ((scope_c(x as f32 + 1.0) - scope_c(x as f32 - 1.0)) / 2.0).abs();
            let c = self.w[0] * cw + self.w[1] * cs + self.w[2] * cc;
            let hh = (self.w[0] + self.w[1]) * h + self.w[2] * hs;
            let (lo, hi) = (c - hh / 2.0, c + hh / 2.0);
            for (y, row) in g.iter_mut().enumerate() {
                let y = y as f32;
                let cov = ((y + 0.5).min(hi) - (y - 0.5).max(lo)).clamp(0.0, 1.0);
                row[x] = row[x].max(DIM + (1.0 - DIM) * cov);
            }
            if self.w[1] > 0.05 {
                let ph = 1.0 + (N - 1) as f32 * self.peak[x];
                let y = ((N - 1) as f32 - (ph - 1.0) - 0.9).max(0.0).round() as usize;
                if y < N && g[y][x] < 0.6 {
                    g[y][x] = g[y][x].max(DIM + (0.65 - DIM) * self.w[1]);
                }
            }
        }
        g
    }
}


/// Draws a dot grid on the tile.
pub fn draw_grid(g: &[[f32; N]; N], size: u32) -> Pixmap {
    let mut pm = Pixmap::new(size, size).expect("a pixmap");
    let s = size as f32 / 1024.0;
    let tf = Transform::from_scale(s, s);
    let (lo, hi, r) = TILE;
    fill(&mut pm, rounded(lo, lo, hi, hi, r), (0x14, 0x14, 0x15), 1.0, tf);
    for (y, row) in g.iter().enumerate() {
        for (x, &a) in row.iter().enumerate() {
            let (cx, cy) = (512.0 + (x as f32 - 4.0) * 68.0, 512.0 + (y as f32 - 4.0) * 68.0);
            fill(&mut pm, PathBuilder::from_circle(cx, cy, 22.0), INK, a, tf);
        }
    }
    pm
}

/// The still icon: the mark.
pub fn still(size: u32) -> Pixmap {
    draw_grid(&mark(), size)
}

fn fill(pm: &mut Pixmap, path: Option<tiny_skia::Path>, c: (u8, u8, u8), a: f32, tf: Transform) {
    let Some(path) = path else { return };
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba8(c.0, c.1, c.2, (a.clamp(0.0, 1.0) * 255.0) as u8));
    paint.anti_alias = true;
    pm.fill_path(&path, &paint, FillRule::Winding, tf, None);
}

/// A rounded rectangle, corners as circular arcs (cubic approximation).
fn rounded(x0: f32, y0: f32, x1: f32, y1: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0);
    if r <= 0.0 {
        return Some(PathBuilder::from_rect(Rect::from_ltrb(x0, y0, x1, y1)?));
    }
    let k = r * 0.552_284_8;
    let mut p = PathBuilder::new();
    p.move_to(x0 + r, y0);
    p.line_to(x1 - r, y0);
    p.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
    p.line_to(x1, y1 - r);
    p.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
    p.line_to(x0 + r, y1);
    p.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
    p.line_to(x0, y0 + r);
    p.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
    p.close();
    p.finish()
}

fn blend(a: &[[f32; N]; N], b: &[[f32; N]; N], k: f32) -> [[f32; N]; N] {
    let k = k * k * (3.0 - 2.0 * k);
    let mut g = *a;
    for (gr, (ar, br)) in g.iter_mut().zip(a.iter().zip(b)) {
        for (gv, (av, bv)) in gr.iter_mut().zip(ar.iter().zip(br)) {
            *gv = av + (bv - av) * k;
        }
    }
    g
}

/// Every frame the Dock can show while busy: the dissolve, then the meter reel.
fn frames() -> Vec<[[f32; N]; N]> {
    let dt = 1.0 / FPS;
    let mut m = Meter { next: f32::MAX, ..Meter::default() };
    for _ in 0..FPS as usize {
        m.step(dt, 0.85, 1.0, true);
    }
    let reel: Vec<_> = (0..REEL)
        .map(|f| {
            m.mode = f * 3 / REEL;
            m.step(dt, 0.85, 1.0, true);
            m.grid()
        })
        .collect();
    let fades = (0..FADES).map(|k| blend(&mark(), &reel[0], (k + 1) as f32 / (FADES + 1) as f32));
    fades.chain(reel.iter().copied()).collect()
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
enum Phase {
    #[default]
    Rest,
    /// Dissolving into the meter; the fade frame shown.
    In(usize),
    /// Playing the reel back and forth.
    Run { pos: usize, fwd: bool },
    /// Settling back into the mark.
    Out(usize),
}

/// Drives the Dock icon: the mark at rest; the reel at 10 fps while something is happening.
/// It only touches AppKit from the main thread, so tests and headless renders leave the Dock
/// alone.
#[derive(Default)]
pub struct Dock {
    phase: Phase,
    tick: u32,
    images: Vec<objc2::rc::Retained<objc2_app_kit::NSImage>>,
    shown: bool,
    last: Option<std::time::Instant>,
}

impl Dock {
    /// Moves one frame toward `mood`; the frame to show, or None once at rest.
    fn advance(&mut self, mood: Mood) -> Option<usize> {
        let idle = mood == Mood::Idle;
        self.tick = self.tick.wrapping_add(1);
        let (phase, frame) = match self.phase {
            Phase::Rest if idle => return None,
            Phase::Rest => (Phase::In(0), 0),
            Phase::In(k) | Phase::Out(k) if idle => match k.checked_sub(1) {
                Some(k) => (Phase::Out(k), k),
                None => {
                    self.phase = Phase::Rest;
                    return None;
                }
            },
            Phase::In(k) | Phase::Out(k) if k + 1 < FADES => (Phase::In(k + 1), k + 1),
            Phase::In(_) | Phase::Out(_) => (Phase::Run { pos: 0, fwd: true }, FADES),
            // Heading home: run back to the reel's first frame quickly, then dissolve.
            Phase::Run { pos: 0, .. } if idle => (Phase::Out(FADES - 1), FADES - 1),
            Phase::Run { pos, .. } if idle => {
                let pos = pos.saturating_sub(3);
                (Phase::Run { pos, fwd: false }, FADES + pos)
            }
            Phase::Run { pos, fwd } => {
                // Waiting on a verdict: just the wave, at half speed.
                let (end, hold) = if mood == Mood::Waiting { (REEL / 3, self.tick % 2 == 1) } else { (REEL, false) };
                let (pos, fwd) = match (hold, fwd) {
                    (true, _) => (pos, fwd),
                    _ if pos + 1 >= end => (pos.min(end) - 1, false),
                    (_, true) => (pos + 1, true),
                    (_, false) if pos == 0 => (1, true),
                    (_, false) => (pos - 1, false),
                };
                (Phase::Run { pos, fwd }, FADES + pos)
            }
        };
        self.phase = phase;
        Some(frame)
    }

    /// Shows `mood`; returns when to come back for the next frame (None once at rest).
    pub fn show(&mut self, mood: Mood) -> Option<std::time::Duration> {
        let mtm = objc2::MainThreadMarker::new()?;
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let now = std::time::Instant::now();
        let step = 1.0 / FPS;
        let dt = self.last.map_or(step, |l| now.duration_since(l).as_secs_f32());
        if self.last.is_some() && dt + 0.002 < step {
            return Some(std::time::Duration::from_secs_f32(step - dt));
        }
        let Some(f) = self.advance(mood) else {
            self.last = None;
            if !self.shown {
                self.shown = true;
                if let Some(img) = image(&still(256)) {
                    unsafe { app.setApplicationIconImage(Some(&img)) };
                }
            }
            return None;
        };
        if self.images.is_empty() {
            self.images = frames().iter().filter_map(|g| image(&draw_grid(g, LIVE_PX))).collect();
        }
        self.shown = false;
        if let Some(img) = self.images.get(f) {
            unsafe { app.setApplicationIconImage(Some(img)) };
        }
        self.last = Some(now);
        Some(std::time::Duration::from_secs_f32(step))
    }
}

fn image(pm: &Pixmap) -> Option<objc2::rc::Retained<objc2_app_kit::NSImage>> {
    use objc2::AnyThread;
    let png = pm.encode_png().ok()?;
    let data = objc2_foundation::NSData::with_bytes(&png);
    objc2_app_kit::NSImage::initWithData(objc2_app_kit::NSImage::alloc(), &data)
}

/// `adeck --dock-icon <png> [size]` writes the still mark; `scripts/install` builds the app icon
/// from it.
/// `adeck --dock-live <dir> [frames]` records the Dock at 10 fps: a second at rest, at work, then
/// two seconds settling back.
pub fn dump(args: &[String]) -> Option<std::io::Result<()>> {
    let at = |flag: &str| args.iter().position(|a| a == flag);
    let png = |pm: Pixmap| pm.encode_png().map_err(std::io::Error::other);
    if let Some(i) = at("--dock-icon") {
        let out = args.get(i + 1).map_or("icon.png", String::as_str);
        let size = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(1024);
        return Some(png(still(size)).and_then(|b| std::fs::write(out, b)));
    }
    let i = at("--dock-live")?;
    let dir = std::path::Path::new(args.get(i + 1).map_or("dock-live", String::as_str));
    let count: usize = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(240);
    Some((|| {
        std::fs::create_dir_all(dir)?;
        let all = frames();
        let mut d = Dock::default();
        for f in 0..count {
            let mood = if f >= 10 && f + 20 < count { Mood::Working } else { Mood::Idle };
            let g = d.advance(mood).map_or_else(mark, |i| all[i]);
            std::fs::write(dir.join(format!("{f:04}.png")), png(draw_grid(&g, 128))?)?;
        }
        Ok(())
    })())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_live_meter_moves_smoothly_and_visits_every_chart() {
        let mut m = Meter::default();
        let mut prev = m.grid();
        let mut seen = [false; 3];
        for _ in 0..12 * 40 {
            m.step(1.0 / 12.0, 1.0, 1.0, true);
            let g = m.grid();
            // Connected, not jumping: no dot swings across most of its range in one frame.
            let jump = g.iter().flatten().zip(prev.iter().flatten()).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(jump < 0.95, "a dot jumped {jump}");
            seen[m.mode] = true;
            prev = g;
        }
        assert!(seen.iter().all(|&s| s), "never reached every chart: {seen:?}");
    }

    #[test]
    fn the_dock_plays_a_small_reel_that_never_jumps_and_settles_into_the_mark() {
        let all = frames();
        assert_eq!(all.len(), FADES + REEL);
        let jump = |a: &[[f32; N]; N], b: &[[f32; N]; N]| a.iter().flatten().zip(b.iter().flatten()).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        let mut d = Dock::default();
        let mut prev = mark();
        let mut seen = vec![false; all.len()];
        for t in 0..400 {
            let mood = if t < 300 { Mood::Working } else { Mood::Idle };
            let Some(f) = d.advance(mood) else {
                assert!(t > 300, "came to rest while working");
                assert!(jump(&prev, &mark()) < 0.5, "the last frame before rest isn't near the mark");
                assert!(seen.iter().all(|&s| s), "some frames never played");
                return;
            };
            seen[f] = true;
            // Settling skips frames to get home fast; otherwise frames follow on smoothly.
            if t < 300 {
                assert!(jump(&prev, &all[f]) < 0.95, "frame {t} jumped");
            }
            prev = all[f];
        }
        panic!("never came back to rest");
    }
}
