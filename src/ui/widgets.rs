//! The pieces every tab is built from: the waveform and spectrogram strips with their playhead,
//! beat grid and loop, the transport, score buttons, and key/value tables.

use rune::{Color, Drag, El, Phase, Tx, col, el, oklch, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit};

use crate::app::{App, Entry, Tab};
use crate::ui::icons;

/// The strips' dark ground, the same in both themes, as a meter's screen is.
pub fn screen() -> Color {
    oklch(0.17, 0.005, 260.0)
}

pub fn wave_ink() -> [u8; 3] {
    [96, 168, 255]
}

/// The waveform from an envelope: min/max as the body, RMS brighter inside it, mirrored about the
/// middle. Straight-alpha RGBA, `w`×`h`.
pub fn paint_wave(env: &[[f32; 3]], w: usize, h: usize) -> Vec<u8> {
    let mut px = vec![0u8; w * h * 4];
    if env.is_empty() || w == 0 || h == 0 {
        return px;
    }
    let peak = env.iter().fold(0f32, |m, c| m.max(c[0].abs()).max(c[1].abs())).max(1e-6);
    // Quiet files still fill most of the strip, as a DAW's normalised view does.
    let gain = (0.95 / peak).min(64.0);
    let mid = h as f32 / 2.0;
    let body = wave_ink();
    let core = [200u8, 225, 255];
    for x in 0..w {
        let a = x * env.len() / w;
        let b = ((x + 1) * env.len() / w).max(a + 1).min(env.len());
        let (mut lo, mut hi, mut rms) = (0f32, 0f32, 0f32);
        for c in &env[a..b] {
            lo = lo.min(c[0]);
            hi = hi.max(c[1]);
            rms = rms.max(c[2]);
        }
        let top = (mid - hi * gain * mid).floor().max(0.0) as usize;
        let bottom = ((mid - lo * gain * mid).ceil() as usize).min(h).max(top + 1);
        let r = (rms * gain * mid).max(0.5);
        let (rt, rb) = ((mid - r).floor().max(0.0) as usize, ((mid + r).ceil() as usize).min(h));
        for y in top..bottom {
            let ink = if (rt..rb).contains(&y) { core } else { body };
            let o = (y * w + x) * 4;
            px[o..o + 3].copy_from_slice(&ink);
            px[o + 3] = 255;
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
    strip = strip.child(picture(e, a, lo, hi, width, false));
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
    let mut strip =
        el().key(key.to_owned()).instant_changes().w_full().h(height).radius(k.radius.md).fill(oklch(0.12, 0.01, 290.0)).clip_rounded(k.radius.md).cursor(rune::Cursor::Pointer);
    let Some((e, a)) = e.and_then(|e| e.analysis.as_ref().map(|a| (e, a))) else {
        return strip;
    };
    let (lo, hi) = s.window(tab);
    strip = strip.child(picture(e, a, lo, hi, width, true));
    let nyq = a.rate as f32 / 2.0;
    let flo = 30f32.min(nyq / 4.0);
    for (hz, label) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
        if hz >= nyq {
            continue;
        }
        let y = height * (1.0 - (hz / flo).ln() / (nyq / flo).ln());
        strip = strip
            .child(el().absolute(0.0, y).w_full().h(1.0).fill(Color::WHITE.alpha(0.08)).inert())
            .child(text(label).font(rune::TextStyle { size: 9.0, ..k.ty.small }).color(Color::WHITE.alpha(0.45)).absolute(4.0, y - 12.0).inert());
    }
    strip = overlays(strip, s, tab, width, height, (lo, hi), k);
    scrub_handler(strip, lo, hi)
}

/// A file's picture over [lo, hi]: the waveform redrawn for that stretch when zoomed in (so it
/// stays sharp), else the whole picture stretched and shifted into place.
fn picture(e: &Entry, a: &crate::derive::Analysis, lo: f64, hi: f64, width: f32, spec: bool) -> El {
    let span = (hi - lo).max(1e-6);
    let whole = lo <= 1e-6 && hi >= a.secs - 1e-6;
    if !spec && !whole && a.secs > 0.0 {
        let w = (width.round() as u32).clamp(64, 3000);
        let key = ((lo * 1000.0) as u64, (hi * 1000.0) as u64, w);
        if e.view_key.get() != key {
            let n = a.envelope.len();
            let ca = ((lo / a.secs) * n as f64).floor().max(0.0) as usize;
            let cb = (((hi / a.secs) * n as f64).ceil() as usize).min(n);
            if cb > ca {
                // Past the file's end the strip stays dark, so draw only the part that exists.
                e.view_px.set(w, 160, paint_wave(&a.envelope[ca..cb], w as usize, 160));
            }
            e.view_key.set(key);
        }
        let own = (((a.secs.min(hi) - lo) / span) as f32 * width).max(0.0);
        return el().absolute(0.0, 0.0).w(own).h_full().pixels(&e.view_px).inert();
    }
    let full_w = width * (a.secs / span) as f32;
    let x0 = -width * (lo / span) as f32;
    el().absolute(x0, 0.0).w(full_w).h_full().pixels(if spec { &e.spec } else { &e.wave }).inert()
}

/// The loop region and the playhead over a strip.
fn overlays(mut strip: El, s: &App, tab: &Tab, width: f32, height: f32, (lo, hi): (f64, f64), k: &Kit) -> El {
    let span = (hi - lo).max(1e-6);
    let x_of = |secs: f64| ((secs - lo) / span) as f32 * width;
    if let Some((a, b)) = tab.loop_secs {
        let (xa, xb) = (x_of(a).clamp(0.0, width), x_of(b).clamp(0.0, width));
        if xb > xa {
            strip = strip.child(el().absolute(xa, 0.0).w((xb - xa).max(1.0)).h(height).fill(k.c.icon_accent.alpha(0.16)).border(1.0, k.c.icon_accent.alpha(0.6)).inert());
        }
    }
    let ph = playhead(s, tab);
    if ph >= lo && ph <= hi {
        let x = x_of(ph);
        // A soft glow behind a crisp line, like a DAW's cursor.
        strip = strip.child(el().absolute(x - 4.0, 0.0).w(8.0).h(height).fill(Color::WHITE.alpha(0.10)).inert()).child(el().absolute(x - 1.0, 0.0).w(2.0).h(height).fill(Color::WHITE).inert());
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

fn loading(why: &str, k: &Kit) -> El {
    row().w_full().h_full().center().gap(8.0).inert().child(text(why.to_owned()).font(k.ty.small).color(Color::WHITE.alpha(0.55)).truncate())
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
        let mut mark = col().gap(0.0).child(el().w(1.0).h(6.0).fill(c.border_strong));
        if f < 0.94 {
            mark = mark.child(text(tick_label(m, step)).font(rune::TextStyle { size: 10.0, ..k.ty.small.features("'tnum' 1") }).color(c.text_faint).offset(3.0, -3.0));
        }
        r = r.child(at_frac(f, mark));
    }
    if let Some((a, b)) = tab.loop_secs {
        let (fa, fb) = (((a - lo) / span).clamp(0.0, 1.0) as f32, ((b - lo) / span).clamp(0.0, 1.0) as f32);
        if fb > fa {
            r = r.child(at_frac(fa, el().w_frac(fb - fa).h(20.0).radius(3.0).fill(c.icon_accent.alpha(0.22)).border(1.0, c.icon_accent.alpha(0.7))));
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

/// A handle under the strips: drag it to make them taller or shorter.
pub fn grip(tab: &Tab, k: &Kit) -> El {
    let c = &k.c;
    let h = tab.strip_h;
    row()
        .key("strip-grip")
        .w_full()
        .h(10.0)
        .center()
        .cursor(rune::Cursor::ResizeRow)
        .help(format!("Drag to resize the strips ({:.0}%) · double-click resets", h * 100.0))
        .child(el().w(36.0).h(4.0).radius(2.0).fill(c.border_strong))
        .on_drag(|s: &mut App, d: &mut Drag, _tx: &mut Tx| {
            let Some(t) = s.current_mut() else { return };
            if matches!(d.phase, Phase::Down) {
                d.data = t.strip_h;
            }
            t.strip_h = (d.data + d.dy / 140.0).clamp(0.5, 3.0);
        })
        .on_double_press(|s: &mut App| {
            if let Some(t) = s.current_mut() {
                t.strip_h = 1.0;
            }
        })
}

/// A thin draggable bar: `frac` filled, the drag hands back a new fraction.
fn bar(key: &str, frac: f32, w: Option<f32>, fill: Color, k: &Kit, on: impl Fn(&mut App, f32, Phase) + 'static) -> El {
    let c = &k.c;
    let f = frac.clamp(0.0, 1.0);
    let mut b = el().key(key.to_owned()).instant_changes().h(18.0).cursor(rune::Cursor::Pointer);
    b = match w {
        Some(w) => b.w(w),
        None => b.grow(1.0).min_w(60.0),
    };
    b.child(el().absolute(0.0, 7.0).w_full().h(4.0).radius(2.0).fill(c.layer_03))
        .child(el().absolute(0.0, 7.0).w_frac(f).h(4.0).radius(2.0).fill(fill))
        .child(at_frac(f, el().offset(-6.0, 3.0).size(12.0, 12.0).radius(6.0).fill(Color::WHITE).border(1.0, c.border_strong)))
        .on_drag(move |s: &mut App, d: &mut Drag, _tx: &mut Tx| {
            if d.w > 0.0 {
                on(s, (d.x / d.w).clamp(0.0, 1.0), d.phase);
            }
        })
}

/// The output level, a meter that falls back smoothly.
pub fn meter(s: &App, k: &Kit) -> El {
    let db = s.mixer.level_db();
    let f = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    let color = if db > -1.0 {
        k.c.danger.icon
    } else if db > -9.0 {
        k.c.warning.icon
    } else {
        k.c.success.icon
    };
    row()
        .key("level")
        .w(40.0)
        .h(6.0)
        .radius(3.0)
        .fill(k.c.layer_03)
        .clip_rounded(3.0)
        .help(if db <= -89.0 { "Level".to_owned() } else { format!("Level {db:.0} dBFS") })
        .child(el().w_frac(f).h_full().fill(color).motion(rune::Spring::SMOOTH))
}

/// The volume control: an icon and a bar.
pub fn volume(s: &App, k: &Kit) -> El {
    let vol = s.mixer.volume;
    row()
        .gap(4.0)
        .center_y()
        .help(format!("Volume {}", if vol <= 0.001 { "muted".into() } else { format!("{:+.1} dB", 20.0 * vol.log10()) }))
        .child(rune::icon(if vol <= 0.001 { &icons::MUTED } else { &icons::SPEAKER }, 14.0).color(k.c.text_muted))
        .child(bar("volume", vol / 1.5, Some(70.0), k.c.text_muted, k, |s: &mut App, f, _| s.set_volume(f * 1.5)))
}

/// The transport every audio tab shares: play, time, a scrubber over the whole file (with the
/// zoomed window marked), zoom, volume, the level, the loop, the note and send-to-agent.
pub fn transport(s: &App, tab: &Tab, k: &Kit) -> El {
    let c = &k.c;
    let playing = s.mixer_is_for(tab.id()) && s.mixer.is_playing();
    let play = IconButton::new(if playing { &icons::PAUSE } else { &icons::PLAY })
        .variant(IconButtonVariant::Contrast)
        .label(if playing { "Pause" } else { "Play" })
        .shortcut("Space")
        .on_press(|s: &mut App| s.toggle_play())
        .build(k)
        .key("play");
    let total = tab_secs(s, tab);
    let ph = playhead(s, tab);
    let mut scrub = bar("scrubber", if total > 0.0 { (ph / total) as f32 } else { 0.0 }, None, c.icon_accent, k, move |s: &mut App, f, phase| s.scrub(f64::from(f) * total, phase));
    if let Some((a, b)) = tab.view.filter(|_| total > 0.0) {
        scrub = scrub.child(at_frac((a / total) as f32, el().offset(0.0, 4.0).w_frac(((b - a) / total) as f32).h(10.0).radius(3.0).border(1.0, c.text_muted.alpha(0.6))));
    }
    let ghost = |label: &str, help: &str, key: &str| Button::new(label.to_owned()).variant(ButtonVariant::Ghost).control_size(ControlSize::Small).help(help.to_owned()).build(k).key(key.to_owned());
    let zoom = row()
        .gap(0.0)
        .center_y()
        .child(ghost("−", "Zoom out · −", "zoom-out").on_press(|s: &mut App| s.zoom(2.0)))
        .child(ghost(if tab.view.is_some() { "Fit" } else { "All" }, "Whole file, or the loop · F", "zoom-fit").on_press(|s: &mut App| s.zoom_fit()))
        .child(ghost("+", "Zoom in · =", "zoom-in").on_press(|s: &mut App| s.zoom(0.5)));
    let vol = s.mixer.volume;
    let volume = row()
        .gap(4.0)
        .center_y()
        .help(format!("Volume {}", if vol <= 0.001 { "muted".into() } else { format!("{:+.1} dB", 20.0 * vol.log10()) }))
        .child(rune::icon(if vol <= 0.001 { &icons::MUTED } else { &icons::SPEAKER }, 14.0).color(c.text_muted))
        .child(bar("volume", vol / 1.5, Some(70.0), c.text_muted, k, |s: &mut App, f, _| s.set_volume(f * 1.5)));
    let mut r = row()
        .key("transport")
        .w_full()
        .gap(10.0)
        .center_y()
        .child(play)
        .child(meter(s, k))
        .child(text(format!("{} / {}", clock(ph), clock(total))).key("clock").font(k.ty.body_medium.features("'tnum' 1")).color(c.text_base))
        .child(scrub)
        .child(zoom)
        .child(volume);
    if s.mixer_is_for(tab.id()) && s.mixer.is_loading() {
        r = r.child(rune_kit::Spinner::new().size(12.0).build(k));
    }
    if let Some((a, b)) = tab.loop_secs {
        r = r.child(
            Button::new(format!("Loop {}–{}  ✕", clock(a), clock(b)))
                .variant(ButtonVariant::Ghost)
                .control_size(ControlSize::Small)
                .help("Clear the loop · Esc")
                .on_press(|s: &mut App| s.set_loop(None))
                .build(k)
                .key("loop"),
        );
    }
    r = r
        .child(
            Button::new("Note")
                .variant(if tab.note_open { ButtonVariant::Contrast } else { ButtonVariant::Ghost })
                .control_size(ControlSize::Small)
                .help("Write a note for the agent · N")
                .on_press(|s: &mut App| s.toggle_note())
                .build(k)
                .key("note-toggle"),
        )
        .child(
            Button::new("Send to agent")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .help("Hand the agent what you're on: the file, the group, the loop or the zoomed stretch · S, or double-click it")
                .on_press_tx(|s: &mut App, tx: &mut Tx| s.send_to_agent(None, tx))
                .build(k)
                .key("send"),
        );
    if let Some(e) = &s.mixer.error {
        r = r.child(text(e.clone()).font(k.ty.small).color(c.danger.fg).truncate().max_w(360.0));
    }
    r.flex_wrap()
}

/// Buttons 0 to 3 for the member you're hearing, with the latest score lit.
pub fn score_buttons(s: &App, tab: &Tab, k: &Kit) -> El {
    let c = &k.c;
    let current = tab.spec.items.get(tab.active).and_then(|i| s.scores.get(i)).copied();
    let help = ["0 · unusable", "1 · usable under a voiceover", "2 · as good as what ships today", "3 · library level"];
    let mut r = row().key("scores").gap(4.0).center_y().child(text("Score").font(k.ty.small).color(c.text_muted));
    for v in 0..4i64 {
        let on = current == Some(v);
        r = r.child(
            Button::new(v.to_string())
                .variant(if on { ButtonVariant::Contrast } else { ButtonVariant::Outline })
                .control_size(ControlSize::Small)
                .help(format!("{} · press {v}", help[v as usize]))
                .on_press(move |s: &mut App| s.score(v))
                .build(k)
                .key(format!("score-{v}")),
        );
    }
    r
}

/// The note field, shown when you ask for it (Note, or N), with save and close.
pub fn note_field(tab: &Tab, k: &Kit) -> El {
    if !tab.note_open {
        return el();
    }
    let input = rune_kit::TextInput::new(&tab.note)
        .placeholder("Note for the agent")
        .on_edit(|s: &mut App| &mut s.current_mut().expect("a tab").note)
        .build(k)
        .key("note")
        .w_full();
    let save = Button::new("Save").variant(ButtonVariant::Contrast).control_size(ControlSize::Small).on_press(|s: &mut App| s.note_only()).build(k).key("note-save");
    let close = IconButton::new(&icons::CLOSE).variant(IconButtonVariant::Ghost).label("Hide the note").on_press(|s: &mut App| s.toggle_note()).build(k).key("note-close");
    row().key("note-row").w_full().gap(8.0).center_y().transition(rune::Transition::offset(0.0, 12.0).combined(rune::Transition::opacity())).child(col().grow(1.0).min_w(0.0).child(input)).child(save).child(close)
}

/// Rows of key and value, in a scrolling column.
pub fn kv_table(key: &str, rows: &[(String, String)], highlight: &dyn Fn(&str) -> bool, k: &Kit) -> El {
    let c = &k.c;
    let mut list = col().key(format!("{key}-rows")).w_full().gap(0.0);
    for (i, (name, value)) in rows.iter().enumerate() {
        let hot = highlight(name);
        list = list.child(
            row()
                .key(format!("{key}-{i}"))
                .w_full()
                .px(8.0)
                .py(4.0)
                .gap(12.0)
                .fill(if hot {
                    c.icon_accent.alpha(0.10)
                } else if i % 2 == 1 {
                    c.layer_01
                } else {
                    Color::TRANSPARENT
                })
                .child(text(name.clone()).font(k.ty.small.features("'tnum' 1")).color(c.text_muted).w(180.0).truncate().help_when_truncated(name.clone()))
                .child(text(value.clone()).font(k.ty.small.features("'tnum' 1")).color(c.text_base).wrap().grow(1.0).min_w(0.0).selectable(true)),
        );
    }
    if rows.is_empty() {
        list = list.child(text("No metadata was found for this file.").font(k.ty.small).color(c.text_faint).px(8.0).py(6.0));
    }
    col().key(key.to_owned()).w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list)
}

/// A small titled number: "LUFS -14.2".
pub fn stat(label: &str, value: String, k: &Kit) -> El {
    col().gap(1.0).child(text(label.to_owned()).font(k.ty.small).color(k.c.text_faint)).child(text(value).font(k.ty.body_medium.features("'tnum' 1")).color(k.c.text_base))
}

/// The native numbers for one file.
pub fn stats(e: Option<&Entry>, trim_db: Option<f64>, k: &Kit) -> El {
    let Some(a) = e.and_then(|e| e.analysis.as_ref()) else {
        return row();
    };
    let mut r = row().gap(18.0).center_y().child(stat("Length", clock(a.secs), k)).child(stat("LUFS", a.lufs.map_or("–".into(), |l| format!("{l:.1}")), k)).child(stat(
        "Peak",
        format!("{:.1} dB", a.peak_dbfs),
        k,
    ));
    if let Some(b) = a.bpm {
        r = r.child(stat("BPM", format!("{b:.1}"), k));
    }
    if let Some(key) = &a.key {
        r = r.child(stat("Key", key.clone(), k));
    }
    r = r.child(stat("Rate", format!("{} Hz · {}ch", a.rate, a.channels), k));
    if let Some(t) = trim_db {
        r = r.child(stat("Match", format!("{t:+.1} dB",), k));
    }
    r
}

/// The REF badge for the local-only reference pack.
pub fn ref_badge(k: &Kit) -> El {
    row()
        .px(5.0)
        .py(1.0)
        .radius(4.0)
        .fill(oklch(0.75, 0.15, 75.0))
        .child(text("REF").font(rune::TextStyle { size: 10.0, ..k.ty.small_medium }).color(oklch(0.2, 0.02, 0.0)))
        .help("From the local-only reference pack. Never published.")
}

/// The agent's pinned note: fold it to one line, or dismiss it.
pub fn annotation(tab: &Tab, k: &Kit) -> Option<El> {
    let md = tab.spec.note.as_ref()?;
    let c = &k.c;
    let folded = tab.annotation_folded;
    let first = md.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim_start_matches(['#', ' ', '>', '-']).replace("**", "");
    let body = if folded {
        text(first).font(k.ty.small).color(c.text_base).truncate().grow(1.0).min_w(0.0)
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
            .py(if folded { 4.0 } else { 8.0 })
            .gap(10.0)
            .center_y()
            .radius(k.radius.md)
            .fill(c.icon_accent.alpha(0.08))
            .border(1.0, c.icon_accent.alpha(0.25))
            .animate_layout()
            .transition(rune::Transition::opacity())
            .child(rune::icon(&icons::SPARK, 14.0).color(c.icon_accent))
            .child(body)
            .child(fold)
            .child(close),
    )
}

/// earshot's results for a file and buttons to run what hasn't run.
pub fn earshot_panel(s: &App, e: &Entry, k: &Kit) -> El {
    let c = &k.c;
    let mut panel = col().key("earshot").w_full().gap(8.0);
    let mut buttons = row().gap(6.0).child(text("earshot").font(k.ty.small_medium).color(c.text_muted)).child(spacer());
    for cmd in [crate::derive::Earshot::Describe, crate::derive::Earshot::Measure] {
        let name = cmd.name();
        let busy = e.earshot_busy.contains(name);
        let done = e.earshot.contains_key(name);
        if done {
            continue;
        }
        let id = e.item.id.clone();
        buttons = buttons.child(
            Button::new(if busy { format!("{name}…") } else { name.to_owned() })
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .disabled(busy)
                .on_press(move |s: &mut App| s.run_earshot(&id, cmd))
                .build(k)
                .key(format!("earshot-{name}")),
        );
    }
    panel = panel.child(buttons);
    let _ = s;
    if let Some(v) = e.earshot.get("describe") {
        let rows = crate::item::flatten(&crate::app::summarise_earshot(crate::derive::Earshot::Describe, v));
        panel = panel.child(col().w_full().gap(2.0).children(rows.into_iter().take(14).map(|(kk, vv)| {
            row().gap(8.0).child(text(kk).font(k.ty.small).color(c.text_faint).w(140.0).truncate()).child(text(vv).font(k.ty.small).color(c.text_base).wrap().grow(1.0).min_w(0.0))
        })));
    }
    if let Some(v) = e.earshot.get("measure") {
        let rows = crate::item::flatten(v);
        panel = panel.child(col().w_full().gap(2.0).children(rows.into_iter().filter(|(kk, _)| !kk.contains("path")).take(16).map(|(kk, vv)| {
            row()
                .gap(8.0)
                .child(text(kk).font(k.ty.small).color(c.text_faint).w(140.0).truncate())
                .child(text(vv).font(k.ty.small.features("'tnum' 1")).color(c.text_base).truncate().grow(1.0).min_w(0.0))
        })));
    }
    panel
}

pub fn section_title(t: &str, k: &Kit) -> El {
    text(t.to_owned()).font(k.ty.small_medium).color(k.c.text_muted)
}
