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

/// A waveform strip for one member: its picture, beat grid, loop, playhead, and a click to seek
/// or a drag to loop. `total` is the time the strip's width stands for.
pub fn wave_strip(s: &App, tab: &Tab, e: Option<&Entry>, key: &str, width: f32, height: f32, total: f64, k: &Kit) -> El {
    let mut strip = el().key(key.to_owned()).instant_changes().w_full().h(height).radius(k.radius.md).fill(screen()).clip_rounded(k.radius.md).cursor(rune::Cursor::Pointer);
    let Some(e) = e else { return strip.child(loading("Opening…", k)) };
    let Some(a) = &e.analysis else {
        let why = e.failed.clone().unwrap_or_else(|| "Analysing…".into());
        return strip.child(loading(&why, k));
    };
    let frac_of = |secs: f64| if total > 0.0 { (secs / total).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let own = frac_of(a.secs);
    strip = strip.child(el().absolute(0.0, 0.0).h_full().w(width * own).pixels(&e.wave).inert());
    if !a.beats.is_empty() && total > 0.0 {
        let mut shape = rune::Shape::new();
        let mut bar = 0;
        for (t, down) in &a.beats {
            let x = frac_of(*t) * 1000.0;
            if *down {
                bar += 1;
                let phrase = bar % 4 == 1;
                shape = shape.line([x, 0.0], [x, 100.0], if phrase { 1.4 } else { 1.0 }, rune::LineCap::Butt, Color::WHITE.alpha(if phrase { 0.35 } else { 0.18 }));
            } else if width / (a.beats.len() as f32) > 6.0 {
                shape = shape.line([x, 0.0], [x, 100.0], 0.8, rune::LineCap::Butt, Color::WHITE.alpha(0.08));
            }
        }
        strip = strip.child(rune::canvas(shape.view_box(1000.0, 100.0)).absolute(0.0, 0.0).w_full().h_full().inert());
    }
    strip = overlays(strip, s, tab, width, height, total, k);
    seek_handler(strip, total)
}

/// The spectrogram strip for one member, with the same playhead and loop.
pub fn spec_strip(s: &App, tab: &Tab, e: Option<&Entry>, key: &str, width: f32, height: f32, total: f64, k: &Kit) -> El {
    let mut strip = el().key(key.to_owned()).instant_changes().w_full().h(height).radius(k.radius.md).fill(oklch(0.12, 0.01, 290.0)).clip_rounded(k.radius.md).cursor(rune::Cursor::Pointer);
    let Some(a) = e.and_then(|e| e.analysis.as_ref()) else { return strip };
    let own = if total > 0.0 { (a.secs / total).clamp(0.0, 1.0) as f32 } else { 1.0 };
    strip = strip.child(el().absolute(0.0, 0.0).h_full().w(width * own).pixels(&e.unwrap().spec).inert());
    // Frequency marks on the log axis the spectrogram uses.
    let nyq = a.rate as f32 / 2.0;
    let lo = 30f32.min(nyq / 4.0);
    for (hz, label) in [(100.0, "100"), (1000.0, "1k"), (10000.0, "10k")] {
        if hz >= nyq {
            continue;
        }
        let y = height * (1.0 - (hz / lo).ln() / (nyq / lo).ln());
        strip = strip
            .child(el().absolute(0.0, y).w_full().h(1.0).fill(Color::WHITE.alpha(0.08)).inert())
            .child(text(label).font(rune::TextStyle { size: 9.0, ..k.ty.small }).color(Color::WHITE.alpha(0.45)).absolute(4.0, y - 12.0).inert());
    }
    strip = overlays(strip, s, tab, width, height, total, k);
    seek_handler(strip, total)
}

/// The loop region and the playhead over a strip.
fn overlays(mut strip: El, s: &App, tab: &Tab, width: f32, height: f32, total: f64, k: &Kit) -> El {
    if total <= 0.0 {
        return strip;
    }
    let x_of = |secs: f64| (secs / total).clamp(0.0, 1.0) as f32 * width;
    if let Some((a, b)) = tab.loop_secs {
        strip = strip.child(el().absolute(x_of(a), 0.0).w((x_of(b) - x_of(a)).max(1.0)).h(height).fill(k.c.icon_accent.alpha(0.16)).border(1.0, k.c.icon_accent.alpha(0.6)).inert());
    }
    let x = x_of(playhead(s, tab));
    strip.child(el().absolute(x - 1.0, 0.0).w(2.0).h(height).fill(Color::WHITE).inert())
}

/// A click seeks there; a drag sets a loop over what it covers.
fn seek_handler(strip: El, total: f64) -> El {
    strip.on_drag(move |s: &mut App, d: &mut Drag, _tx: &mut Tx| {
        if d.w <= 0.0 || total <= 0.0 {
            return;
        }
        let at = f64::from((d.x / d.w).clamp(0.0, 1.0)) * total;
        let from = f64::from((d.start_x / d.w).clamp(0.0, 1.0)) * total;
        match d.phase {
            Phase::Down => {}
            Phase::Up if d.dx.abs() < 4.0 => s.seek_secs(at),
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

fn loading(why: &str, k: &Kit) -> El {
    row().w_full().h_full().center().gap(8.0).inert().child(text(why.to_owned()).font(k.ty.small).color(Color::WHITE.alpha(0.55)).truncate())
}

/// Play/pause, the time, and the loop with its clear button.
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
    let mut r = row().gap(10.0).center_y().child(play).child(text(format!("{} / {}", clock(playhead(s, tab)), clock(total))).key("clock").font(k.ty.body_medium.features("'tnum' 1")).color(c.text_base));
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
    if let Some(e) = &s.mixer.error {
        r = r.child(text(e.clone()).font(k.ty.small).color(c.danger.fg).truncate().max_w(360.0));
    }
    r
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

/// The note field and its save button, shared by every tab.
pub fn note_field(tab: &Tab, k: &Kit) -> El {
    let input = rune_kit::TextInput::new(&tab.note)
        .placeholder("Note for the LLM (saved with your next score or pick)")
        .on_edit(|s: &mut App| &mut s.current_mut().expect("a tab").note)
        .build(k)
        .key("note")
        .grow(1.0);
    let save = Button::new("Save note").variant(ButtonVariant::Ghost).control_size(ControlSize::Small).on_press(|s: &mut App| s.note_only()).build(k).key("note-save");
    row().w_full().gap(8.0).center_y().child(input).child(save)
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
                .fill(if hot { c.icon_accent.alpha(0.10) } else if i % 2 == 1 { c.layer_01 } else { Color::TRANSPARENT })
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
    let Some(a) = e.and_then(|e| e.analysis.as_ref()) else { return row() };
    let mut r = row().gap(18.0).center_y().child(stat("Length", clock(a.secs), k)).child(stat("LUFS", a.lufs.map_or("–".into(), |l| format!("{l:.1}")), k)).child(stat("Peak", format!("{:.1} dB", a.peak_dbfs), k));
    if let Some(b) = a.bpm {
        r = r.child(stat("BPM", format!("{b:.1}"), k));
    }
    if let Some(key) = &a.key {
        r = r.child(stat("Key", key.clone(), k));
    }
    r = r.child(stat("Rate", format!("{} Hz · {}ch", a.rate, a.channels), k));
    if let Some(t) = trim_db {
        r = r.child(stat("Match", format!("{t:+.1} dB", ), k));
    }
    r
}

/// The REF badge for the local-only reference pack.
pub fn ref_badge(k: &Kit) -> El {
    row().px(5.0).py(1.0).radius(4.0).fill(oklch(0.75, 0.15, 75.0)).child(text("REF").font(rune::TextStyle { size: 10.0, ..k.ty.small_medium }).color(oklch(0.2, 0.02, 0.0))).help("From the local-only reference pack. Never published.")
}

/// The LLM's pinned note.
pub fn annotation(tab: &Tab, k: &Kit) -> Option<El> {
    let md = tab.spec.note.as_ref()?;
    let c = &k.c;
    Some(
        row()
            .key("annotation")
            .w_full()
            .px(12.0)
            .py(8.0)
            .gap(10.0)
            .radius(k.radius.md)
            .fill(c.icon_accent.alpha(0.08))
            .border(1.0, c.icon_accent.alpha(0.25))
            .child(rune::icon(&icons::SPARK, 14.0).color(c.icon_accent))
            .child(col().grow(1.0).min_w(0.0).child(rune_kit::Markdown::new(md.clone()).build(k))),
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
            row().gap(8.0).child(text(kk).font(k.ty.small).color(c.text_faint).w(140.0).truncate()).child(text(vv).font(k.ty.small.features("'tnum' 1")).color(c.text_base).truncate().grow(1.0).min_w(0.0))
        })));
    }
    panel
}

pub fn section_title(t: &str, k: &Kit) -> El {
    text(t.to_owned()).font(k.ty.small_medium).color(k.c.text_muted)
}
