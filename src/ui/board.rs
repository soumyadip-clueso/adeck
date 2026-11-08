//! A board: one card per group, sorted (by search count, say), each a row of tiles: the picks,
//! the "before", the reference files. Click a tile to play it; only one plays, and switching
//! inside a group keeps the position. The focused group opens up: its waveform and
//! spectrogram zoomed to the sound, how its bands, width and tail sit against the target,
//! what made it, the keep/drop/move decision, and the ranked rest.

use rune::{Color, El, Tx, col, el, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Switch};
use serde_json::{Map, Value};

use crate::app::{App, Entry, Tab};
use crate::board::{Chip, Group, Member};
use crate::ui::widgets;

const TILE_MIN: f32 = 168.0;
const GAP: f32 = 10.0;
const BANDS: [&str; 5] = ["sub", "low", "lowmid", "mid", "presence"];

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let Some(b) = t.board() else {
        return text("This board is empty.").font(k.ty.small).color(c.text_muted);
    };
    let blind = Switch::new(t.spec.blind).label("Blind").on_toggle(|s: &mut App| s.toggle_blind()).build(k).key("blind").help("Hide which is which in each group until you pick · B");
    let lm = Switch::new(t.spec.loudness_match).label("Match loudness").on_toggle(|s: &mut App| s.toggle_loudness_match()).build(k).key("match");
    let zoom = Switch::new(t.zoom_sound).label("Zoom to sound").on_toggle(|s: &mut App| s.toggle_zoom()).build(k).key("zoom").help("Strips show just the sound, not the silence · Z");
    let mut tally = row().gap(10.0).center_y();
    for (tone, n) in b.tally() {
        tally = tally.child(row().gap(5.0).center_y().child(chip(&Chip { text: n.to_string(), tone: tone.clone() }, k)).child(text(tone_word(&tone)).font(k.ty.small).color(c.text_muted)));
    }
    let head = row().key("board-head").w_full().gap(14.0).center_y().flex_wrap().child(widgets::search_field(t, "Search groups and sounds", 240.0, k)).child(tally).child(spacer()).child(blind).child(lm).child(zoom);

    let mut page = col().key("board").w_full().h_full().min_h(0.0).gap(10.0).child(head).child(widgets::transport(s, t, k)).child(widgets::note_field(t, k));
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    if let Some(err) = &t.error {
        page = page.child(text(err.clone()).font(k.ty.small).color(c.danger.fg).wrap());
    }
    let focused = t.focused().map(|g| g.key.clone());
    let inner = width - 14.0;
    let mut list = col().key("groups").w_full().gap(12.0).pr(10.0);
    let shown = s.shown_groups(t);
    for g in &shown {
        let on = focused.as_deref() == Some(g.key.as_str());
        list = list.child(group_card(s, t, g, on, inner, k));
    }
    if shown.is_empty() && !b.groups.is_empty() {
        list = list.child(text(format!("No group or sound matches “{}”.", t.filter.text.trim())).font(k.ty.small).color(c.text_muted).py(20.0));
    }
    if b.groups.is_empty() {
        list = list.child(text("Nothing on this board yet. The agent adds groups as results land.").font(k.ty.small).color(c.text_muted).py(20.0));
    }
    page.child(col().key("groups-scroll").w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
}

fn tone_word(tone: &str) -> &str {
    match tone {
        "good" => "good",
        "close" => "close",
        "bad" => "not there yet",
        _ => "other",
    }
}

pub fn chip(ch: &Chip, k: &Kit) -> El {
    let (fill, ink) = match ch.tone.as_str() {
        "good" => (k.c.success.icon.alpha(0.18), k.c.success.fg),
        "close" => (k.c.layer_02, k.c.text_base),
        "bad" => (k.c.warning.icon.alpha(0.16), k.c.warning.fg),
        _ => (k.c.layer_01, k.c.text_muted),
    };
    row().px(7.0).py(2.0).radius(4.0).fill(fill).child(text(ch.text.clone()).font(k.ty.small_medium).color(ink).truncate())
}

pub fn role_color(role: &str, k: &Kit) -> Color {
    match role {
        "pick" => k.c.icon_accent,
        "before" => k.c.warning.fg,
        "ref" => k.c.success.fg,
        _ => k.c.text_muted,
    }
}

fn group_card(s: &App, t: &Tab, g: &Group, on: bool, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let hidden = t.group_hidden(&g.key);
    let key = g.key.clone();
    let mut head = row().w_full().gap(10.0).center_y().flex_wrap().child(text(g.title().to_owned()).font(k.ty.ui_medium).color(c.text_base));
    if let Some(sub) = &g.subtitle {
        head = head.child(text(sub.clone()).font(k.ty.small).color(c.text_muted));
    }
    head = head.child(spacer());
    if let Some(ch) = g.chip.as_ref().filter(|_| !hidden) {
        head = head.child(chip(ch, k));
    }
    if !on {
        let k2 = key.clone();
        head = head.child(
            Button::new("Open")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| {
                    s.focus_group(&k2);
                    if let Some(t) = s.current_mut() {
                        t.sheet_open = true;
                    }
                })
                .build(k)
                .key(format!("open-{key}")),
        );
    }
    let mut card = col()
        .key(format!("group-{key}"))
        .w_full()
        .px(12.0).py(12.0)
        .gap(10.0)
        .radius(k.radius.lg)
        .fill(if on { c.layer_01 } else { Color::TRANSPARENT })
        .border(1.0, if on { c.text_base.alpha(0.35) } else { c.border_weak })
        .child(head);

    // The tiles, wrapped to the width.
    let order = t.group_order(g);
    let inner = width - 24.0;
    let cols = (((inner + GAP) / (TILE_MIN + GAP)).floor() as usize).clamp(1, order.len().max(1));
    let tile_w = (inner - GAP * (cols as f32 - 1.0)) / cols as f32;
    for (r, chunk) in order.chunks(cols).enumerate() {
        let mut line = row().key(format!("tiles-{key}-{r}")).w_full().gap(GAP);
        for (i, &m) in chunk.iter().enumerate() {
            line = line.child(tile(s, t, g, m, on, r * cols + i, tile_w, k));
        }
        card = card.child(line);
    }
    if !on {
        if !g.more.is_empty() {
            card = card.child(text(format!("+ {} more takes", g.more.len())).font(k.ty.small).color(c.text_faint));
        }
        return card;
    }
    if let Some(n) = &g.note {
        card = card.child(rune_kit::Markdown::new(n.clone()).build(k));
    }
    card
}

/// One member: click anywhere on it to play it.
#[allow(clippy::too_many_arguments)]
fn tile(s: &App, t: &Tab, g: &Group, m: usize, focused: bool, pos: usize, w: f32, k: &Kit) -> El {
    let c = &k.c;
    let mem = &g.members[m];
    let hidden = t.group_hidden(&g.key);
    let e = s.entry(&mem.item);
    let active = focused && t.active == m;
    let playing = active && s.mixer_is_for(t.id()) && s.mixer.is_playing();
    let label = t.board_label(g, m);
    let mut top = row().gap(6.0).center_y();
    if focused && pos < 9 {
        top = top.child(text((pos + 1).to_string()).font(k.ty.small_medium.features("'tnum' 1")).color(c.text_faint));
    }
    top = top.child(text(label.clone()).font(k.ty.small_medium).color(if hidden { c.text_base } else { role_color(&mem.role, k) }).truncate().shrink());
    if !hidden && e.is_some_and(|e| e.item.is_ref) {
        top = top.child(widgets::ref_badge(k));
    }
    top = top.child(spacer());
    if let Some(d) = s.decisions.get(&mem.item) {
        top = top.child(text(d.clone()).font(k.ty.small).color(if d == "drop" { c.danger.fg } else { c.text_muted }));
    }
    let mut thumb = el().w_full().h(44.0).radius(k.radius.sm).fill(widgets::screen()).clip_rounded(k.radius.sm).inert();
    if let Some(e) = e {
        let px = if t.zoom_sound && e.sound.is_some() { &e.zoom } else { &e.thumb };
        if e.analysis.is_some() {
            thumb = thumb.child(el().absolute(0.0, 0.0).w_full().h_full().pixels(px));
        }
        if active && let Some((a, b)) = view_of(t, e) {
            let pos = widgets::playhead(s, t);
            if pos >= a && pos <= b && b > a {
                thumb = thumb.child(el().absolute(((pos - a) / (b - a)) as f32 * (w - 16.0) - 1.0, 0.0).w(2.0).h_full().fill(Color::WHITE));
            }
        }
    }
    let mut body = col().gap(5.0).child(top).child(thumb);
    if !hidden {
        let nums = numbers_line(&mem.numbers, s, t, g);
        if !nums.is_empty() {
            body = body.child(text(nums).font(k.ty.small.features("'tnum' 1")).color(c.text_muted).wrap());
        }
        if let Some(ch) = &mem.chip {
            body = body.child(row().child(chip(ch, k)));
        }
    }
    let secs = e.and_then(|e| e.analysis.as_ref()).map(|a| a.secs);
    if let Some(sec) = secs {
        body = body.child(text(if sec < 1.0 { format!("{:.0} ms", sec * 1000.0) } else { format!("{sec:.1} s") }).font(k.ty.small.features("'tnum' 1")).color(c.text_faint));
    }
    let key = g.key.clone();
    col()
        .key(format!("tile-{}-{m}", g.key))
        .w(w)
        .px(8.0).py(8.0)
        .radius(k.radius.md)
        .fill(if active { c.text_base.alpha(if playing { 0.20 } else { 0.12 }) } else { c.layer_01 })
        .border(1.0, if active { c.text_base.alpha(0.75) } else { c.border_weak })
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(format!("Play {label}"))
        .help(if focused && pos < 9 { format!("Play {label} · {} · double-click sends it to the agent", pos + 1) } else { format!("Play {label} · double-click sends it to the agent") })
        .motion(rune::Spring::SNAPPY)
        .child(body)
        .on_press(move |s: &mut App| s.board_play(&key, m))
        .on_double_press_tx(move |s: &mut App, tx: &mut Tx| s.send_to_agent(Some(m), tx))
}

/// "vibe 1.05 · shape 0.51", labelled by the board's metrics where it has them.
pub fn numbers_line(n: &Map<String, Value>, _s: &App, t: &Tab, _g: &Group) -> String {
    let metrics = t.board().map(|b| b.metrics.as_slice()).unwrap_or_default();
    n.iter()
        .map(|(key, v)| {
            let label = metrics.iter().find(|m| m.key == *key).map_or(key.as_str(), |m| m.label());
            let val = match v {
                Value::Number(x) => x.as_f64().map_or_else(|| x.to_string(), fmt_num),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            format!("{label} {val}")
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn fmt_num(x: f64) -> String {
    if (x.fract() == 0.0 && x.abs() < 1e6) || x.abs() >= 100.0 {
        format!("{x:.0}")
    } else if x.abs() >= 10.0 {
        format!("{x:.1}")
    } else {
        format!("{x:.2}")
    }
}

/// The stretch a strip shows for an entry: just the sound while zoomed.
fn view_of(t: &Tab, e: &Entry) -> Option<(f64, f64)> {
    let secs = e.analysis.as_ref()?.secs;
    Some(if t.zoom_sound { e.sound.unwrap_or((0.0, secs)) } else { (0.0, secs) })
}

/// The focused group opened up.
pub fn detail(s: &App, t: &Tab, g: &Group, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let all = g.playable();
    let Some(mem) = all.get(t.active).copied() else { return col() };
    let hidden = t.active < g.members.len() && t.group_hidden(&g.key);
    let e = s.entry(&mem.item);
    let mut d = col().key("detail").w_full().gap(8.0);
    let label = t.board_label(g, t.active);
    let mut caption = row().w_full().gap(8.0).center_y().flex_wrap().child(text(format!("Hearing {label}")).font(k.ty.small_medium).color(c.text_base));
    if let Some(e) = e {
        if e.analysis.is_some() {
            let (a, b) = s.window(t);
            let whole = e.analysis.as_ref().map_or(0.0, |x| x.secs);
            let what = if b - a < whole - 1e-3 { format!("{}–{} of {}", widgets::clock(a), widgets::clock(b), widgets::clock(whole)) } else { widgets::clock(whole) };
            caption = caption.child(text(what).font(k.ty.small.features("'tnum' 1")).color(c.text_muted));
        }
        if let Some(tr) = s.trim_db(t, &mem.item) {
            caption = caption.child(text(format!("matched {tr:+.1} dB")).font(k.ty.small).color(c.text_muted));
        }
    }
    caption = caption.child(spacer());
    let looping = t.loop_secs.is_some();
    caption = caption
        .child(Button::new("Replay").variant(ButtonVariant::Outline).control_size(ControlSize::Small).help("From the start of the sound · R").on_press(|s: &mut App| s.board_replay()).build(k).key("replay"))
        .child(
            Button::new(if looping { "Looping" } else { "Loop" })
                .variant(if looping { ButtonVariant::Contrast } else { ButtonVariant::Outline })
                .control_size(ControlSize::Small)
                .help("Loop what the strip shows · L. Drag across the strip to loop part of it.")
                .on_press(|s: &mut App| s.board_toggle_loop())
                .build(k)
                .key("loop-toggle"),
        );
    if t.spec.blind && !t.spec.revealed_groups.contains(&g.key) {
        caption = caption.child(
            Button::new(format!("Pick {label}"))
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::Small)
                .help("The one you're hearing is the best here · Enter")
                .on_press(|s: &mut App| s.board_pick())
                .build(k)
                .key("pick"),
        );
    }
    d = d.child(caption);
    if let Some(e) = e {
        d = d
            .child(widgets::ruler(s, t, "bruler", k))
            .child(widgets::wave_strip(s, t, Some(e), "bwave", width, 72.0, k))
            .child(widgets::spec_strip(s, t, Some(e), "bspec", width, 96.0, k))
            .child(widgets::grip(t, k));
    }

    // Analysis beside details when there's room, else stacked.
    let wide = width >= 760.0;
    let half = if wide { (width - 16.0) / 2.0 } else { width };
    let left = if hidden { col().w(half).child(text("Blind: the numbers show after you pick.").font(k.ty.small).color(c.text_muted)) } else { analysis(s, g, mem, e, half, k) };
    let right = if hidden { col().w(half) } else { details(s, t, mem, e, half, k) };
    d = d.child(if wide { row().w_full().gap(16.0).child(left).child(right) } else { col().w_full().gap(12.0).child(left).child(right) });

    if let Some(dec) = decisions(s, t, mem, k) {
        d = d.child(dec);
    }
    if !g.more.is_empty() {
        d = d.child(more(s, t, g, width, k));
    }
    d
}

/// The measurements a member is shown with: its own, else earshot's.
fn measure_of<'a>(mem: &'a Member, e: Option<&'a Entry>) -> Option<&'a Value> {
    mem.measure.as_ref().or_else(|| e.and_then(|e| e.earshot.get("measure")))
}

/// Bands, stereo width and tail against the group's target.
fn analysis(s: &App, g: &Group, mem: &Member, e: Option<&Entry>, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let mut p = col().key("analysis").w(width).gap(6.0).child(widgets::section_title("Against the target", k));
    let Some(m) = measure_of(mem, e) else {
        let busy = e.is_some_and(|e| e.earshot_busy.contains("measure"));
        let msg = if busy {
            "Measuring…"
        } else if crate::derive::on_path(&s.cfg.earshot) {
            "Not measured yet."
        } else {
            "No measurements: earshot isn't installed, and the board didn't include any."
        };
        return p.child(text(msg).font(k.ty.small).color(c.text_muted).wrap());
    };
    let target = g.target().unwrap_or_default();
    let tb = target.get("bands_db").and_then(Value::as_object);
    if let Some(bands) = m.get("bands_db").and_then(Value::as_object) {
        for b in BANDS {
            let Some(v) = bands.get(b).and_then(Value::as_f64) else { continue };
            let want = tb.and_then(|t| t.get(b)).and_then(Value::as_f64);
            p = p.child(meter(b, v, want, -60.0, 0.0, |x| format!("{x:.1}"), 3.0, 6.0, width, k));
        }
    }
    if let Some(v) = m.get("correlation").and_then(Value::as_f64) {
        let want = target.get("correlation").and_then(Value::as_f64);
        p = p.child(meter("width", v, want, -1.0, 1.0, |x| format!("{x:.2}"), 0.15, 0.35, width, k).help("Stereo correlation: 1 is mono, 0 wide, below 0 out of phase"));
    }
    for (key, label) in [("decay_ms", "tail"), ("attack_ms", "attack")] {
        if let Some(v) = m.get(key).and_then(Value::as_f64) {
            let want = target.get(key).and_then(Value::as_f64);
            let hi = v.max(want.unwrap_or(0.0)) * 1.25 + 1.0;
            let tol = want.map_or(f64::MAX, |w| w * 0.3);
            p = p.child(meter(label, v, want, 0.0, hi, |x| format!("{x:.0} ms"), tol, tol * 2.0, width, k));
        }
    }
    if let Some(v) = m.get("lufs").and_then(Value::as_f64) {
        let want = target.get("lufs").and_then(Value::as_f64);
        p = p.child(meter("lufs", v, want, -40.0, 0.0, |x| format!("{x:.1}"), 2.0, 4.0, width, k));
    }
    p
}

/// One row: name, a bar for the value on [lo, hi], a tick where the target is, the numbers and
/// the gap coloured by how far off it is.
#[allow(clippy::too_many_arguments)]
fn meter(name: &str, v: f64, want: Option<f64>, lo: f64, hi: f64, fmt: impl Fn(f64) -> String, ok: f64, near: f64, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let bar_w = (width - 64.0 - 158.0).max(40.0);
    let frac = |x: f64| ((x - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
    let off = want.map(|w| v - w);
    let tone = match off.map(f64::abs) {
        Some(d) if d <= ok => c.success.fg,
        Some(d) if d <= near => c.warning.fg,
        Some(_) => c.danger.fg,
        None => c.text_muted,
    };
    let mut bar = el().w(bar_w).h(10.0).radius(3.0).fill(c.layer_02).child(el().absolute(0.0, 0.0).w(bar_w * frac(v)).h(10.0).radius(3.0).fill(tone.alpha(0.7)));
    if let Some(w) = want {
        bar = bar.child(el().absolute(bar_w * frac(w) - 1.0, -3.0).w(2.0).h(16.0).fill(c.text_base));
    }
    let nums = match (want, off) {
        (Some(w), Some(d)) => format!("{} / {}  {}{}", fmt(v), fmt(w), if d >= 0.0 { "+" } else { "−" }, fmt(d.abs())),
        _ => fmt(v),
    };
    row()
        .w(width)
        .gap(8.0)
        .center_y()
        .child(text(name.to_owned()).font(k.ty.small).color(c.text_muted).w(56.0))
        .child(bar)
        .child(text(nums).font(k.ty.small.features("'tnum' 1")).color(tone).w(150.0).truncate())
}

/// What made the file, its numbers and its path.
fn details(s: &App, t: &Tab, mem: &Member, e: Option<&Entry>, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let open = t.details_open;
    let toggle = Button::new(if open { "Details ▾" } else { "Details ▸" })
        .variant(ButtonVariant::Outline)
        .control_size(ControlSize::Small)
        .on_press(|s: &mut App| {
            if let Some(t) = s.current_mut() {
                t.details_open ^= true;
            }
        })
        .build(k)
        .key("details-toggle");
    let mut p = col().key("details").w(width).gap(4.0).child(row().w_full().child(toggle));
    let mut rows: Vec<(String, String)> = crate::item::flatten(&Value::Object(mem.meta.clone()));
    if rows.is_empty()
        && let Some(e) = e
    {
        rows = e.item.meta.clone();
    }
    let prompt = rows.iter().find(|(k, _)| k == "prompt" || k.ends_with(".prompt")).map(|(_, v)| v.clone());
    if !open {
        if let Some(pr) = prompt {
            p = p.child(text(pr).font(k.ty.small).color(c.text_base).wrap());
        }
        let brief: Vec<String> = ["model", "seed", "chain"].iter().filter_map(|f| rows.iter().find(|(k, _)| k == f || k.ends_with(&format!(".{f}"))).map(|(k, v)| format!("{} {v}", k.rsplit('.').next().unwrap_or(k)))).collect();
        if !brief.is_empty() {
            p = p.child(text(brief.join(" · ")).font(k.ty.small).color(c.text_muted).wrap());
        }
        return p;
    }
    for (name, value) in rows.iter().take(40) {
        p = p.child(
            row()
                .w_full()
                .gap(8.0)
                .child(text(name.clone()).font(k.ty.small).color(c.text_faint).w(110.0).truncate())
                .child(text(value.clone()).font(k.ty.small.features("'tnum' 1")).color(c.text_base).wrap().grow(1.0).min_w(0.0).selectable(true)),
        );
    }
    let mut nums: Vec<String> = mem.numbers.iter().map(|(k, v)| format!("{k} {v}")).collect();
    if let Some(a) = e.and_then(|e| e.analysis.as_ref()) {
        nums.push(format!("lufs {}", a.lufs.map_or("–".into(), |l| format!("{l:.1}"))));
        nums.push(format!("peak {:.1}", a.peak_dbfs));
    }
    if !nums.is_empty() {
        p = p.child(text(nums.join(" · ")).font(k.ty.small.features("'tnum' 1")).color(c.text_muted).wrap().selectable(true));
    }
    if let Some(e) = e {
        let path = e.item.path.display().to_string();
        let copy = path.clone();
        p = p.child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(text(path).font(rune::TextStyle { size: 11.0, ..k.ty.mono }).color(c.text_muted).wrap().grow(1.0).min_w(0.0).selectable(true))
                .child(
                    Button::new("Copy path")
                        .variant(ButtonVariant::Outline)
                        .control_size(ControlSize::Small)
                        .on_press_tx(move |s: &mut App, tx: &mut Tx| {
                            tx.copy(copy.clone());
                            s.toast = Some(("Path copied".into(), false));
                        })
                        .build(k)
                        .key("copy-path"),
                ),
        );
    }
    let _ = s;
    p
}

/// Keep, drop, move: the board's choices for the file you're hearing.
fn decisions(s: &App, t: &Tab, mem: &Member, k: &Kit) -> Option<El> {
    let b = t.board()?;
    if b.choices.is_empty() && b.move_to.is_empty() {
        return None;
    }
    let c = &k.c;
    let now = s.decisions.get(&mem.item);
    let mut r = row().key("decide").w_full().gap(6.0).center_y().flex_wrap().child(text("Decision").font(k.ty.small).color(c.text_muted));
    for ch in &b.choices {
        let on = now == Some(ch);
        let (id, v) = (mem.item.clone(), ch.clone());
        let hint = match ch.as_str() {
            "keep" => " · K",
            "drop" => " · D",
            _ => "",
        };
        r = r.child(
            Button::new(ch.clone())
                .variant(if on { ButtonVariant::Contrast } else { ButtonVariant::Outline })
                .control_size(ControlSize::Small)
                .help(format!("{ch}{hint}"))
                .on_press(move |s: &mut App| s.decide(&id, &v))
                .build(k)
                .key(format!("decide-{ch}")),
        );
    }
    if !b.move_to.is_empty() {
        let moved = now.and_then(|n| n.strip_prefix("move:"));
        r = r.child(
            Button::new(moved.map_or_else(|| "Move to…".to_owned(), |m| format!("Moved to {m}")))
                .variant(if moved.is_some() { ButtonVariant::Contrast } else { ButtonVariant::Outline })
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| {
                    if let Some(t) = s.current_mut() {
                        t.move_open ^= true;
                    }
                })
                .build(k)
                .key("move"),
        );
    }
    let mut out = col().w_full().gap(6.0).child(r);
    if t.move_open {
        let mut targets = row().key("move-targets").w_full().gap(4.0).flex_wrap();
        for g in &b.move_to {
            let (id, v) = (mem.item.clone(), format!("move:{g}"));
            targets = targets.child(Button::new(g.clone()).variant(ButtonVariant::Outline).control_size(ControlSize::Small).on_press(move |s: &mut App| s.decide(&id, &v)).build(k).key(format!("move-{g}")));
        }
        out = out.child(targets);
    }
    Some(out)
}

/// The ranked rest, folded.
fn more(s: &App, t: &Tab, g: &Group, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let open = t.more_open.contains(&g.key);
    let key = g.key.clone();
    let toggle = Button::new(if open { format!("Hide the other {} ▾", g.more.len()) } else { format!("The other {} takes, ranked ▸", g.more.len()) })
        .variant(ButtonVariant::Outline)
        .control_size(ControlSize::Small)
        .on_press(move |s: &mut App| s.toggle_more(&key))
        .build(k)
        .key("more-toggle");
    let mut p = col().key("more").w_full().gap(2.0).child(row().w_full().child(toggle));
    if !open {
        return p;
    }
    for (i, m) in g.more.iter().enumerate() {
        let idx = g.members.len() + i;
        let on = t.active == idx;
        let key = g.key.clone();
        let nums = numbers_line(&m.numbers, s, t, g);
        p = p.child(
            row()
                .key(format!("more-{i}"))
                .w(width)
                .px(8.0)
                .py(5.0)
                .gap(10.0)
                .center_y()
                .radius(k.radius.sm)
                .fill(if on { c.text_base.alpha(0.12) } else if i % 2 == 1 { c.layer_02 } else { Color::TRANSPARENT })
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label(format!("Play take {}", i + 1))
                .child(text(format!("#{}", i + 1)).font(k.ty.small_medium.features("'tnum' 1")).color(c.text_faint).w(30.0))
                .child(text(m.label.clone().or_else(|| s.entry(&m.item).map(|e| e.item.name.clone())).unwrap_or_default()).font(k.ty.small).color(c.text_base).w(140.0).truncate())
                .child(text(nums).font(k.ty.small.features("'tnum' 1")).color(c.text_muted).truncate().grow(1.0).min_w(0.0))
                .child(text(s.decisions.get(&m.item).cloned().unwrap_or_default()).font(k.ty.small).color(c.text_muted))
                .on_press(move |s: &mut App| s.board_play(&key, idx)),
        );
    }
    p
}
