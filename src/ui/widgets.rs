//! The pieces every tab is built from: the waveform and spectrogram strips with their playhead,
//! beat grid and loop, the transport, score buttons, and key/value tables.

use rune::{Color, Drag, El, Phase, Tx, col, el, row, spacer, text};
use rune_kit::{IconButton, IconButtonVariant, Kit};

use crate::app::{App, Entry, Tab};
use crate::ui::{icons, px};

/// The strips' dark ground, the same in both themes, as a meter's screen is.
pub fn screen() -> Color {
    px::SCREEN
}

/// Dot pitch, in device pixels, for a picture `h` tall: 8 (4pt at 2x) on strips, smaller on
/// thumbnails so they still show a shape.
fn cell_for(h: usize) -> usize {
    (h / 22).clamp(4, 8)
}

/// Lights one dot of a `cell`-pitch grid.
fn dot(px: &mut [u8], w: usize, h: usize, cell: usize, (c, r): (usize, usize), rgba: [u8; 4]) {
    let d = cell - cell / 4;
    for y in r * cell..(r * cell + d).min(h) {
        let o = (y * w + c * cell) * 4;
        for x in 0..d.min(w.saturating_sub(c * cell)) {
            px[o + x * 4..o + x * 4 + 4].copy_from_slice(&rgba);
        }
    }
}

const UNLIT: [u8; 4] = [255, 255, 255, 12];

/// The waveform as an LED screen: each column of dots lit to the peak (grey) with the loudness
/// core brighter, unlit dots faintly there. Straight-alpha RGBA, `w`×`h`.
pub fn paint_wave(env: &[[f32; 3]], w: usize, h: usize) -> Vec<u8> {
    let mut px = vec![0u8; w * h * 4];
    if env.is_empty() || w == 0 || h == 0 {
        return px;
    }
    let peak = env.iter().fold(0f32, |m, c| m.max(c[0].abs()).max(c[1].abs())).max(1e-6);
    // Quiet files still fill most of the screen, as a DAW's normalised view does.
    let gain = (0.92 / peak).min(64.0);
    let cell = cell_for(h);
    let (cols, rows) = (w.div_ceil(cell), h / cell);
    let mid = rows as f32 / 2.0;
    for c in 0..cols {
        let a = c * cell * env.len() / w;
        let b = (((c + 1) * cell).min(w) * env.len() / w).max(a + 1).min(env.len());
        let (mut lo, mut hi, mut rms) = (0f32, 0f32, 0f32);
        for e in &env[a.min(env.len() - 1)..b] {
            lo = lo.min(e[0]);
            hi = hi.max(e[1]);
            rms = rms.max(e[2]);
        }
        let (hp, hr) = (hi.max(-lo) * gain * mid, rms * gain * mid);
        for r in 0..rows {
            let dy = (r as f32 + 0.5 - mid).abs();
            let ink = if dy <= hr.max(0.5) {
                [238, 238, 240, 255]
            } else if dy <= hp.max(0.5) {
                [150, 152, 160, 230]
            } else {
                UNLIT
            };
            dot(&mut px, w, h, cell, (c, r), ink);
        }
    }
    px
}

/// The spectrogram as an LED screen, sampled from its cached picture (`sw`×`sh`, greys, low
/// frequencies at the bottom) between fractions `x0` and `x1` of its width.
pub fn paint_spec(raw: &[u8], (sw, sh): (usize, usize), (x0, x1): (f64, f64), w: usize, h: usize) -> Vec<u8> {
    let mut px = vec![0u8; w * h * 4];
    if raw.len() < sw * sh * 4 || sw == 0 || sh == 0 || w == 0 || h == 0 {
        return px;
    }
    let cell = cell_for(h);
    let (cols, rows) = (w.div_ceil(cell), h / cell);
    for c in 0..cols {
        let fa = x0 + (x1 - x0) * (c * cell) as f64 / w as f64;
        let fb = x0 + (x1 - x0) * (((c + 1) * cell).min(w)) as f64 / w as f64;
        let (xa, xb) = ((fa * sw as f64) as usize, ((fb * sw as f64).ceil() as usize).max((fa * sw as f64) as usize + 1).min(sw));
        for r in 0..rows {
            let (ya, yb) = (r * sh / rows, ((r + 1) * sh / rows).max(r * sh / rows + 1).min(sh));
            let mut v = 0u8;
            for y in ya..yb {
                for x in xa.min(sw - 1)..xb {
                    v = v.max(raw[(y * sw + x) * 4]);
                }
            }
            // The cached picture runs from near-black (22) to white (250).
            let t = ((f32::from(v) - 22.0) / 228.0).clamp(0.0, 1.0);
            let g = (150.0 + 105.0 * t) as u8;
            let a = (12.0 + 243.0 * t.powf(1.3)) as u8;
            dot(&mut px, w, h, cell, (c, r), [g, g, g.saturating_add(3), a]);
        }
    }
    px
}

/// `m:ss.d`.
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0);
    let m = (s / 60.0).floor();
    format!("{}:{:04.1}", m as u64, s - m * 60.0)
}

/// Where the playhead is drawn for a tab: the mixer's position if it's playing this tab, else 0.
pub fn playhead(s: &App, tab: &Tab) -> f64 {
    if s.mixer_is_for(tab.id()) { s.mixer.secs() } else { 0.0 }
}

/// What a strip is drawn against: the longest member, so a short member ends early on a shared
/// time axis.
pub fn tab_secs(s: &App, tab: &Tab) -> f64 {
    let longest = tab.spec.items.iter().filter_map(|i| s.entry(i)).filter_map(|e| e.analysis.as_ref()).map(|a| a.secs).fold(0.0, f64::max);
    if s.mixer_is_for(tab.id()) && s.mixer.frames > 0 { s.mixer.total_secs().max(longest) } else { longest }
}

/// A waveform strip for one member over the tab's time window: its picture, beat grid, loop and
/// playhead. Drag across it to scrub; double-click to send what you're on to the agent.
#[allow(clippy::too_many_arguments)]
pub fn wave_strip(s: &App, tab: &Tab, e: Option<&Entry>, key: &str, width: f32, height: f32, k: &Kit) -> El {
    let height = height * tab.strip_h;
    let mut strip = el().key(key.to_owned()).instant_changes().w_full().h(height).radius(k.radius.md).fill(screen()).clip_rounded(k.radius.md).cursor(rune::Cursor::Pointer);
    let Some(e) = e else {
        return strip.child(loading("Opening…", k));
    };
    let Some(a) = &e.analysis else {
        let why = e.failed.clone().unwrap_or_else(|| "Analysing…".into());
        return strip.child(loading(&why, k));
    };
    let (lo, hi) = s.window(tab);
    let span = (hi - lo).max(1e-6);
    let x_of = |secs: f64| ((secs - lo) / span) as f32 * width;
    strip = strip.child(picture(e, a, lo, hi, width, height, false));
    if !a.beats.is_empty() {
        let mut shape = rune::Shape::new();
        let mut bar = 0;
        let dense = a.beats.iter().filter(|(t, _)| *t >= lo && *t <= hi).count();
        for (t, down) in &a.beats {
            if *down {
                bar += 1;
            }
            if *t < lo || *t > hi {
                continue;
            }
            let x = (x_of(*t) / width.max(1.0)) * 1000.0;
            if *down {
                let phrase = bar % 4 == 1;
                shape = shape.line([x, 0.0], [x, 100.0], if phrase { 1.4 } else { 1.0 }, rune::LineCap::Butt, Color::WHITE.alpha(if phrase { 0.35 } else { 0.18 }));
            } else if width / (dense.max(1) as f32) > 6.0 {
                shape = shape.line([x, 0.0], [x, 100.0], 0.8, rune::LineCap::Butt, Color::WHITE.alpha(0.08));
            }
        }
        strip = strip.child(rune::canvas(shape.view_box(1000.0, 100.0)).absolute(0.0, 0.0).w_full().h_full().inert());
    }
    strip = overlays(strip, s, tab, width, height, (lo, hi), k);
    scrub_handler(strip, lo, hi)
}

/// The spectrogram strip for one member, on the same window, playhead and loop.
#[allow(clippy::too_many_arguments)]
pub fn spec_strip(s: &App, tab: &Tab, e: Option<&Entry>, key: &str, width: f32, height: f32, k: &Kit) -> El {
    let height = height * tab.strip_h;
    let mut strip = el().key(key.to_owned()).instant_changes().w_full().h(height).radius(k.radius.md).fill(screen()).clip_rounded(k.radius.md).cursor(rune::Cursor::Pointer);
    let Some((e, a)) = e.and_then(|e| e.analysis.as_ref().map(|a| (e, a))) else {
        return strip;
    };
    let (lo, hi) = s.window(tab);
    strip = strip.child(picture(e, a, lo, hi, width, height, true));
    let nyq = a.rate as f32 / 2.0;
    let flo = 30f32.min(nyq / 4.0);
    for (hz, label) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
        if hz >= nyq {
            continue;
        }
        let y = height * (1.0 - (hz / flo).ln() / (nyq / flo).ln());
        strip = strip.child(
            row()
                .absolute(6.0, (y - 9.0).clamp(4.0, height - 18.0))
                .px(4.0)
                .py(2.0)
                .fill(px::SCREEN)
                .inert()
                .child(text(label.to_uppercase()).font(px::pixel(10.0)).color(px::SILK)),
        );
    }
    strip = overlays(strip, s, tab, width, height, (lo, hi), k);
    scrub_handler(strip, lo, hi)
}

/// A file's picture over [lo, hi], redrawn as LED dots at the strip's pixel size (2x) whenever
/// the window, zoom or size changes, so the dots stay square and sharp.
fn picture(e: &Entry, a: &crate::derive::Analysis, lo: f64, hi: f64, width: f32, height: f32, spec: bool) -> El {
    e.ensure_full();
    let span = (hi - lo).max(1e-6);
    if a.secs <= 0.0 {
        return el();
    }
    let w = ((width * 2.0).round() as u32).clamp(64, 6000);
    let h = ((height * 2.0).round() as u32).clamp(32, 1600);
    // Past the file's end the screen stays dark, so draw only the part that exists.
    let own = ((((a.secs.min(hi) - lo) / span) * f64::from(w)).round() as usize).clamp(1, w as usize);
    let key = ((lo * 1000.0) as u64, (hi * 1000.0) as u64, w, h);
    let (pixels, cache) = if spec { (&e.spec_view, &e.spec_key) } else { (&e.view_px, &e.view_key) };
    if cache.get() != key {
        if spec {
            let raw = e.spec_raw.borrow();
            pixels.set(own as u32, h, paint_spec(&raw, (a.spec_w as usize, a.spec_h as usize), (lo / a.secs, a.secs.min(hi) / a.secs), own, h as usize));
        } else {
            let n = a.envelope.len();
            let ca = ((lo / a.secs) * n as f64).floor().max(0.0) as usize;
            let cb = (((hi / a.secs) * n as f64).ceil() as usize).min(n);
            if cb > ca {
                pixels.set(own as u32, h, paint_wave(&a.envelope[ca..cb], own, h as usize));
            }
        }
        cache.set(key);
    }
    let own_w = (((a.secs.min(hi) - lo) / span) as f32 * width).max(0.0);
    el().absolute(0.0, 0.0).w(own_w).h_full().pixels(pixels).inert()
}

/// The loop region and the playhead over a strip.
fn overlays(mut strip: El, s: &App, tab: &Tab, width: f32, height: f32, (lo, hi): (f64, f64), _k: &Kit) -> El {
    let span = (hi - lo).max(1e-6);
    let x_of = |secs: f64| ((secs - lo) / span) as f32 * width;
    if let Some((a, b)) = tab.loop_secs {
        let (xa, xb) = (x_of(a).clamp(0.0, width), x_of(b).clamp(0.0, width));
        if xb > xa {
            strip = strip.child(el().absolute(xa, 0.0).w((xb - xa).max(1.0)).h(height).fill(px::AMBER.alpha(0.10)).border(1.0, px::AMBER.alpha(0.55)).inert());
        }
    }
    let ph = playhead(s, tab);
    if ph >= lo && ph <= hi {
        let x = x_of(ph);
        strip = strip.child(el().absolute(x - 1.0, 0.0).w(2.0).h(height).fill(px::AMBER).inert());
    }
    strip
}

/// Press or drag to scrub; double-click sends what you're on to the agent.
fn scrub_handler(strip: El, lo: f64, hi: f64) -> El {
    strip
        .on_drag(move |s: &mut App, d: &mut Drag, _tx: &mut Tx| {
            if d.w <= 0.0 {
                return;
            }
            let at = lo + f64::from((d.x / d.w).clamp(0.0, 1.0)) * (hi - lo);
            s.scrub(at, d.phase);
        })
        .on_double_press_tx(|s: &mut App, tx: &mut Tx| s.send_to_agent(None, tx))
}

fn loading(why: &str, _k: &Kit) -> El {
    row().w_full().h_full().center().gap(8.0).inert().child(text(why.to_uppercase()).font(px::pixel(10.0)).color(px::FAINT).truncate())
}

/// `child` placed `f` of the way across its parent, on a layer that ignores the pointer.
fn at_frac(f: f32, child: El) -> El {
    row().absolute(0.0, 0.0).w_full().h_full().inert().child(el().w_frac(f.clamp(0.0, 1.0)).h(1.0)).child(child)
}

/// A tab's page: the window's height when it's wide, its content's when narrow (it scrolls).
pub fn page(key: &str, width: f32) -> El {
    let p = col().key(key.to_owned()).w_full();
    if width < 620.0 { p } else { p.h_full().min_h(0.0) }
}

/// A tick step that puts about six labels across `span` seconds.
fn tick_step(span: f64) -> f64 {
    const STEPS: [f64; 18] = [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0];
    STEPS.iter().copied().find(|st| span / st <= 7.0).unwrap_or(600.0)
}

fn tick_label(t: f64, step: f64) -> String {
    if step < 0.01 {
        format!("{:.0} ms", t * 1000.0)
    } else if step < 1.0 {
        format!("{t:.2}")
    } else {
        clock(t).trim_end_matches(".0").to_owned()
    }
}

/// The time axis over the strips: ticks for the window, and the loop. Drag along it to loop
/// that stretch; click it to clear the loop.
pub fn ruler(s: &App, tab: &Tab, key: &str, k: &Kit) -> El {
    let c = &k.c;
    let (lo, hi) = s.window(tab);
    let span = (hi - lo).max(1e-6);
    let step = tick_step(span);
    let mut r = el().key(key.to_owned()).instant_changes().w_full().h(20.0).cursor(rune::Cursor::Crosshair).help("Drag to loop a stretch · click to clear the loop");
    let mut t = (lo / step).ceil() * step;
    let mut marks = Vec::new();
    while t <= hi + 1e-9 {
        marks.push(t);
        t += step;
    }
    // Percent positions: the ruler has no width of its own until it's laid out.
    for m in marks {
        let f = ((m - lo) / span) as f32;
        let mut mark = col().gap(0.0).child(el().w(1.0).h(4.0).fill(c.border_strong));
        if f < 0.94 {
            mark = mark.child(text(tick_label(m, step)).font(px::pixel(11.0)).color(px::FAINT).offset(3.0, -1.0));
        }
        r = r.child(at_frac(f, mark));
    }
    if let Some((a, b)) = tab.loop_secs {
        let (fa, fb) = (((a - lo) / span).clamp(0.0, 1.0) as f32, ((b - lo) / span).clamp(0.0, 1.0) as f32);
        if fb > fa {
            r = r.child(at_frac(fa, el().w_frac(fb - fa).h(20.0).radius(3.0).fill(px::AMBER.alpha(0.18)).border(1.0, px::AMBER.alpha(0.7))));
        }
    }
    r.on_drag(move |s: &mut App, d: &mut Drag, _tx: &mut Tx| {
        if d.w <= 0.0 {
            return;
        }
        let at = lo + f64::from((d.x / d.w).clamp(0.0, 1.0)) * span;
        let from = lo + f64::from((d.start_x / d.w).clamp(0.0, 1.0)) * span;
        match d.phase {
            Phase::Down => {}
            Phase::Up if d.dx.abs() < 4.0 => s.set_loop(None),
            _ if d.dx.abs() >= 4.0 => {
                s.set_loop(Some((from.min(at), from.max(at))));
                if matches!(d.phase, Phase::Up) {
                    s.seek_secs(from.min(at));
                }
            }
            _ => {}
        }
    })
}

/// A tab's search box: narrows what the tab lists as you type. `/` focuses it, Esc clears it.
pub fn search_field(t: &Tab, placeholder: &str, width: f32, k: &Kit) -> El {
    rune_kit::TextInput::new(&t.filter)
        .placeholder(placeholder.to_owned())
        .leading_icon(&icons::SEARCH)
        .width(Some(width))
        .clear_button()
        .on_edit(|s: &mut App| &mut s.current_mut().expect("a tab").filter)
        .on_key(|s: &mut App, ev: &rune::KeyEvent, _tx: &mut Tx| {
            if ev.key != rune::Key::Escape {
                return false;
            }
            if let Some(t) = s.current_mut() {
                t.filter = rune::TextEdit::new("");
            }
            true
        })
        .build(k)
        .key("tab-search")
}

/// A small titled number: "LUFS -14.2".
pub fn stat(label: &str, value: String, k: &Kit) -> El {
    col().gap(1.0).child(text(label.to_owned()).font(k.ty.small).color(k.c.text_faint)).child(text(value).font(k.ty.body_medium.features("'tnum' 1")).color(k.c.text_base))
}

/// The file's numbers as pixel readouts: LUFS, peak, BPM, key, rate (and the loudness match).
pub fn readouts(e: Option<&Entry>, trim_db: Option<f64>) -> El {
    let Some(a) = e.and_then(|e| e.analysis.as_ref()) else {
        return row();
    };
    let mut r =
        row().key("readouts").gap(28.0).child(px::readout("LUFS", a.lufs.map_or("–".into(), |l| format!("{l:.1}")))).child(px::readout("PEAK", format!("{:.1}", a.peak_dbfs)));
    if let Some(b) = a.bpm {
        r = r.child(px::readout("BPM", format!("{b:.0}")));
    }
    if let Some(key) = &a.key {
        r = r.child(px::readout("KEY", key.replace(['(', ')'], "")));
    }
    r = r.child(px::readout("RATE", format!("{:.1}K", f64::from(a.rate) / 1000.0)));
    if let Some(t) = trim_db {
        r = r.child(px::readout("MATCH", format!("{t:+.1}")));
    }
    r
}

/// The REF badge for the local-only reference pack.
pub fn ref_badge(_k: &Kit) -> El {
    row().px(5.0).py(1.0).radius(2.0).border(1.0, px::SILK).child(text("REF").font(px::pixel(10.0)).color(px::SOFT)).help("From the local-only reference pack. Never published.")
}

/// The agent's pinned note: fold it to one line, or dismiss it.
pub fn annotation(tab: &Tab, k: &Kit) -> Option<El> {
    let md = tab.spec.note.as_ref()?;
    let folded = tab.annotation_folded;
    let first = md.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start_matches(['#', ' ', '>', '-']).replace("**", "");
    let body = if folded {
        text(first).font(rune::TextStyle::new(13.0, 400.0)).color(px::SOFT).truncate().grow(1.0).min_w(0.0)
    } else {
        col().grow(1.0).min_w(0.0).child(rune_kit::Markdown::new(md.clone()).build(k))
    };
    let fold = IconButton::new(if folded { &icons::CHEVRON_RIGHT } else { &icons::CHEVRON_DOWN })
        .variant(IconButtonVariant::Ghost)
        .label(if folded { "Show the whole note" } else { "Fold the note" })
        .on_press(|s: &mut App| {
            if let Some(t) = s.current_mut() {
                t.annotation_folded ^= true;
            }
        })
        .build(k)
        .key("annotation-fold");
    let close = IconButton::new(&icons::CLOSE)
        .variant(IconButtonVariant::Ghost)
        .label("Dismiss the note")
        .on_press(|s: &mut App| {
            if let Some(id) = s.active {
                s.annotate(id, String::new());
            }
        })
        .build(k)
        .key("annotation-close");
    Some(
        row()
            .key("annotation")
            .w_full()
            .px(12.0)
            .py(if folded { 2.0 } else { 4.0 })
            .gap(10.0)
            .center_y()
            .radius(6.0)
            .border(1.0, px::GRILLE)
            .animate_layout()
            .transition(rune::Transition::opacity())
            .child(text("AGENT").font(px::pixel(11.0)).color(px::AMBER))
            .child(body)
            .child(fold)
            .child(close),
    )
}

/// earshot's results for a file and buttons to run what hasn't run.
pub fn earshot_panel(s: &App, e: &Entry, _k: &Kit) -> El {
    let mut panel = col().key("earshot").w_full().gap(8.0);
    let mut buttons = row().gap(6.0).center_y().child(px::label("earshot")).child(spacer());
    for cmd in [crate::derive::Earshot::Describe, crate::derive::Earshot::Measure] {
        let name = cmd.name();
        let busy = e.earshot_busy.contains(name);
        let done = e.earshot.contains_key(name);
        if done {
            continue;
        }
        let id = e.item.id.clone();
        let mut b = px::key_sm(if busy { format!("{name}…") } else { format!("Run {name}") }, false).key(format!("earshot-{name}"));
        if !busy {
            b = b.on_press(move |s: &mut App| s.run_earshot(&id, cmd));
        }
        buttons = buttons.child(b);
    }
    panel = panel.child(buttons);
    let _ = s;
    if e.earshot.is_empty() {
        panel = panel.child(text("Tags, labels and measurements from earshot show here once you run it.").font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).wrap());
    }
    if let Some(v) = e.earshot.get("describe") {
        let rows = crate::item::flatten(&crate::app::summarise_earshot(crate::derive::Earshot::Describe, v));
        panel = panel.child(col().w_full().gap(2.0).children(rows.into_iter().take(14).map(|(kk, vv)| {
            row()
                .w_full()
                .py(5.0)
                .gap(8.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], px::SELECTED)
                .child(text(kk.to_uppercase()).font(px::pixel(10.0)).color(px::FAINT).w(120.0).pt(2.0).truncate())
                .child(text(vv).font(rune::TextStyle::new(12.0, 400.0).mono()).color(px::SOFT).wrap().grow(1.0).min_w(0.0))
        })));
    }
    if let Some(v) = e.earshot.get("measure") {
        let rows = crate::item::flatten(v);
        panel = panel.child(col().w_full().gap(2.0).children(rows.into_iter().filter(|(kk, _)| !kk.contains("path")).take(16).map(|(kk, vv)| {
            row()
                .w_full()
                .py(5.0)
                .gap(8.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], px::SELECTED)
                .child(text(kk.to_uppercase()).font(px::pixel(10.0)).color(px::FAINT).w(120.0).pt(2.0).truncate())
                .child(text(vv).font(px::pixel(11.0)).color(px::SOFT).truncate().grow(1.0).min_w(0.0))
        })));
    }
    panel
}

pub fn section_title(t: &str, k: &Kit) -> El {
    let _ = k;
    px::label(t)
}
