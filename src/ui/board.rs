//! A board: one card per group, sorted (by search count, say), each a row of tiles: the picks,
//! the "before", the reference files. Click a tile to play it; only one plays, and switching
//! inside a group keeps the position. The focused group opens up: its waveform and
//! spectrogram zoomed to the sound, how its bands, width and tail sit against the target,
//! what made it, the keep/drop/move decision, and the ranked rest.

use rune::{Color, El, Tx, col, el, row, spacer, text};
use rune_kit::Kit;
use serde_json::{Map, Value};

use crate::app::{App, Entry, Tab};
use crate::board::{Chip, Group, Member};
use crate::ui::{px, widgets};

const TILE_MIN: f32 = 168.0;
const GAP: f32 = 10.0;
const BANDS: [&str; 5] = ["sub", "low", "lowmid", "mid", "presence"];

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let Some(b) = t.board() else {
        return text("This board is empty.").font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT);
    };
    let blind = px::toggle("Blind", t.spec.blind).key("blind").help("Hide which is which in each group until you pick · B").on_press(|s: &mut App| s.toggle_blind());
    let lm = px::toggle("Match", t.spec.loudness_match).key("match").help("Match loudness: trim every take to the same LUFS · M").on_press(|s: &mut App| s.toggle_loudness_match());
    let zoom = px::toggle("Zoom", t.zoom_sound).key("zoom").help("Zoom to the sound: strips show just the sound, not the silence · Z").on_press(|s: &mut App| s.toggle_zoom());
    let mut tally = row().gap(14.0).center_y();
    for (tone, n) in b.tally() {
        tally = tally.child(px::lamp(&format!("{n} {}", tone_word(&tone)), tone_ink(&tone)));
    }
    let tools = row().gap(8.0).center_y().child(widgets::search_field(t, "Search", 200.0, k)).child(blind).child(lm).child(zoom);
    let head = px::header("board-head", t.spec.label.clone().unwrap_or_else(|| "Board".into()), tally, tools);

    let mut page = col().key("board").w_full().h_full().min_h(0.0).gap(14.0).child(head);
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    if let Some(err) = &t.error {
        page = page.child(text(err.clone()).font(rune::TextStyle::new(13.0, 400.0)).color(px::BAD).wrap());
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
        list = list.child(text(format!("No group or sound matches “{}”.", t.filter.text.trim())).font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).py(20.0));
    }
    if b.groups.is_empty() {
        list = list.child(text("Nothing on this board yet. The agent adds groups as results land.").font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).py(20.0));
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

fn tone_ink(tone: &str) -> Color {
    match tone {
        "good" => px::GOOD,
        "close" => px::CLOSE,
        "bad" => px::BAD,
        _ => px::SILK,
    }
}

pub fn chip(ch: &Chip, _k: &Kit) -> El {
    px::tag(&ch.text, tone_ink(&ch.tone))
}

pub fn role_color(role: &str, _k: &Kit) -> Color {
    match role {
        "pick" => px::LED,
        "before" => px::CLOSE,
        "ref" => px::GOOD,
        _ => px::SILK,
    }
}

fn group_card(s: &App, t: &Tab, g: &Group, on: bool, width: f32, k: &Kit) -> El {
    let hidden = t.group_hidden(&g.key);
    let key = g.key.clone();
    let mut head = row().w_full().gap(12.0).center_y().flex_wrap().child(text(g.title().to_uppercase()).font(px::pixel(20.0)).color(if on { px::LED } else { px::SOFT }));
    if let Some(sub) = &g.subtitle {
        head = head.child(text(sub.to_uppercase()).font(px::pixel(11.0)).color(px::FAINT));
    }
    head = head.child(spacer());
    if let Some(ch) = g.chip.as_ref().filter(|_| !hidden) {
        head = head.child(chip(ch, k));
    }
    if !on {
        let k2 = key.clone();
        head = head.child(px::key("OPEN", false).key(format!("open-{key}")).help("Focus this group: its target and detail open in the pane").on_press(move |s: &mut App| {
            s.focus_group(&k2);
            if s.pane == px::Dock::Hidden {
                s.toggle_pane();
            }
        }));
    }
    let mut card = col()
        .key(format!("group-{key}"))
        .w_full()
        .px(14.0)
        .py(14.0)
        .gap(12.0)
        .radius(6.0)
        .fill(if on { px::RAISED } else { Color::TRANSPARENT })
        .border(1.0, if on { px::GRILLE_ON } else { px::LINE })
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
            card = card.child(text(format!("+ {} MORE TAKES", g.more.len())).font(px::pixel(11.0)).color(px::FAINT));
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
    let mem = &g.members[m];
    let hidden = t.group_hidden(&g.key);
    let e = s.entry(&mem.item);
    let active = focused && t.active == m;
    let playing = active && s.mixer_is_for(t.id()) && s.mixer.is_playing();
    let label = t.board_label(g, m);
    let mut top = row().gap(6.0).center_y();
    if focused && pos < 9 {
        top = top.child(text((pos + 1).to_string()).font(px::pixel(11.0)).color(px::OFF));
    }
    top = top.child(text(label.to_uppercase()).font(px::pixel(12.0)).color(if hidden { px::LED } else { role_color(&mem.role, k) }).truncate().shrink());
    if !hidden && e.is_some_and(|e| e.item.is_ref) {
        top = top.child(widgets::ref_badge(k));
    }
    top = top.child(spacer());
    if let Some(d) = s.decisions.get(&mem.item) {
        top = top.child(text(d.to_uppercase()).font(px::pixel(10.0)).color(if d == "drop" { px::BAD } else { px::SILK }));
    }
    let mut thumb = el().w_full().h(52.0).radius(3.0).fill(widgets::screen()).clip_rounded(3.0).instant_changes().inert();
    if let Some(e) = e {
        let px = if t.zoom_sound && e.sound.is_some() { &e.zoom } else { &e.thumb };
        if e.analysis.is_some() {
            thumb = thumb.child(el().absolute(0.0, 0.0).w_full().h_full().pixels(px));
        }
        if active && let Some((a, b)) = view_of(t, e) {
            let pos = widgets::playhead(s, t);
            if pos >= a && pos <= b && b > a {
                thumb = thumb.child(el().absolute(((pos - a) / (b - a)) as f32 * (w - 16.0) - 1.0, 0.0).w(2.0).h_full().fill(px::AMBER));
            }
        }
    }
    let mut body = col().gap(5.0).child(top).child(thumb);
    if !hidden {
        let nums = numbers_line(&mem.numbers, s, t, g);
        if !nums.is_empty() {
            body = body.child(text(nums.to_uppercase()).font(px::pixel(10.0)).color(px::SILK).wrap());
        }
        if let Some(ch) = &mem.chip {
            body = body.child(row().child(chip(ch, k)));
        }
    }
    let secs = e.and_then(|e| e.analysis.as_ref()).map(|a| a.secs);
    if let Some(sec) = secs {
        body = body.child(text(if sec < 1.0 { format!("{:.0} MS", sec * 1000.0) } else { format!("{sec:.1} S") }).font(px::pixel(10.0)).color(px::FAINT));
    }
    let key = g.key.clone();
    col()
        .key(format!("tile-{}-{m}", g.key))
        .w(w)
        .px(8.0)
        .py(8.0)
        .radius(4.0)
        .fill(if active { px::SELECTED } else { px::SCREEN })
        .border(
            1.0,
            if playing {
                px::AMBER
            } else if active {
                px::SOFT
            } else {
                px::LINE
            },
        )
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(format!("Play {label}"))
        .help(if focused && pos < 9 {
            format!("Play {label} · {} · double-click sends it to the agent", pos + 1)
        } else {
            format!("Play {label} · double-click sends it to the agent")
        })
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
    let all = g.playable();
    let Some(mem) = all.get(t.active).copied() else { return col() };
    let hidden = t.active < g.members.len() && t.group_hidden(&g.key);
    let e = s.entry(&mem.item);
    let mut d = col().key("detail").w_full().gap(8.0);
    let label = t.board_label(g, t.active);
    let mut caption = row().w_full().gap(8.0).center_y().flex_wrap().child(text(format!("HEARING {}", label.to_uppercase())).font(px::pixel(12.0)).color(px::LED));
    if let Some(e) = e {
        if e.analysis.is_some() {
            let (a, b) = s.window(t);
            let whole = e.analysis.as_ref().map_or(0.0, |x| x.secs);
            let what = if b - a < whole - 1e-3 { format!("{}–{} of {}", widgets::clock(a), widgets::clock(b), widgets::clock(whole)) } else { widgets::clock(whole) };
            caption = caption.child(text(what).font(px::pixel(11.0)).color(px::SILK));
        }
        if let Some(tr) = s.trim_db(t, &mem.item) {
            caption = caption.child(text(format!("MATCHED {tr:+.1} DB")).font(px::pixel(11.0)).color(px::SILK));
        }
    }
    caption = caption.child(spacer());
    let looping = t.loop_secs.is_some();
    caption = caption.child(px::key("REPLAY", false).key("replay").help("From the start of the sound · R").on_press(|s: &mut App| s.board_replay())).child(
        px::key("LOOP", looping).key("loop-toggle").help("Loop what the strip shows · L. Drag across the strip to loop part of it.").on_press(|s: &mut App| s.board_toggle_loop()),
    );
    if t.spec.blind && !t.spec.revealed_groups.contains(&g.key) {
        caption = caption.child(
            px::key(format!("PICK {}", label.to_uppercase()), true)
                .key("board-pick")
                .help("The one you're hearing is the best here · Enter")
                .on_press(|s: &mut App| s.board_pick()),
        );
    }
    d = d.child(caption);
    if let Some(e) = e {
        d = d.child(widgets::ruler(s, t, "bruler", k)).child(widgets::wave_strip(s, t, Some(e), "bwave", width, 72.0, k)).child(widgets::spec_strip(
            s,
            t,
            Some(e),
            "bspec",
            width,
            96.0,
            k,
        ));
    }

    // Analysis beside details when there's room, else stacked.
    let wide = width >= 760.0;
    let half = if wide { (width - 16.0) / 2.0 } else { width };
    let left = if hidden {
        col().w(half).child(text("Blind: the numbers show after you pick.").font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT))
    } else {
        analysis(s, g, mem, e, half, k)
    };
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
    let mut p = col().key("analysis").w(width).gap(8.0).child(px::label("Against the target").pb(2.0));
    let Some(m) = measure_of(mem, e) else {
        let busy = e.is_some_and(|e| e.earshot_busy.contains("measure"));
        let msg = if busy {
            "Measuring…"
        } else if crate::derive::on_path(&s.cfg.earshot) {
            "Not measured yet."
        } else {
            "No measurements: earshot isn't installed, and the board didn't include any."
        };
        return p.child(text(msg).font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).wrap());
    };
    let _ = c;
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
fn meter(name: &str, v: f64, want: Option<f64>, lo: f64, hi: f64, fmt: impl Fn(f64) -> String, ok: f64, near: f64, width: f32, _k: &Kit) -> El {
    const SEGS: usize = 24;
    let bar_w = (width - 64.0 - 158.0).max(40.0);
    let frac = |x: f64| ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    let off = want.map(|w| v - w);
    let tone = match off.map(f64::abs) {
        Some(d) if d <= ok => px::GOOD,
        Some(d) if d <= near => px::CLOSE,
        Some(_) => px::BAD,
        None => px::SOFT,
    };
    let lit = (frac(v) * SEGS as f64).round() as usize;
    let at = want.map(|w| ((frac(w) * SEGS as f64) as usize).min(SEGS - 1));
    let mut bar = row().w(bar_w).h(12.0).gap(2.0).center_y();
    for i in 0..SEGS {
        let (ink, h) = if Some(i) == at {
            (px::LED, 12.0)
        } else if i < lit {
            (tone, 8.0)
        } else {
            (px::GRILLE_ON, 8.0)
        };
        bar = bar.child(el().grow(1.0).w(0.0).h(h).fill(ink));
    }
    let nums = match (want, off) {
        (Some(w), Some(d)) => format!("{} / {}  {}{}", fmt(v), fmt(w), if d >= 0.0 { "+" } else { "−" }, fmt(d.abs())),
        _ => fmt(v),
    };
    row()
        .w(width)
        .gap(8.0)
        .center_y()
        .child(text(name.to_uppercase()).font(px::pixel(10.0)).color(px::FAINT).w(56.0))
        .child(bar)
        .child(text(nums.to_uppercase()).font(px::pixel(11.0)).color(tone).w(150.0).truncate())
}

/// What made the file, its numbers and its path.
fn details(s: &App, t: &Tab, mem: &Member, e: Option<&Entry>, width: f32, k: &Kit) -> El {
    let open = t.details_open;
    let toggle = px::key(if open { "DETAILS ▾" } else { "DETAILS ▸" }, false).key("details-toggle").on_press(|s: &mut App| {
        if let Some(t) = s.current_mut() {
            t.details_open ^= true;
        }
    });
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
            p = p.child(text(pr).font(rune::TextStyle::new(13.0, 400.0)).color(px::SOFT).wrap());
        }
        let brief: Vec<String> = ["model", "seed", "chain"]
            .iter()
            .filter_map(|f| rows.iter().find(|(k, _)| k == f || k.ends_with(&format!(".{f}"))).map(|(k, v)| format!("{} {v}", k.rsplit('.').next().unwrap_or(k))))
            .collect();
        if !brief.is_empty() {
            p = p.child(text(brief.join(" · ").to_uppercase()).font(px::pixel(10.0)).color(px::FAINT).wrap());
        }
        return p;
    }
    for (name, value) in rows.iter().take(40) {
        p = p.child(
            row()
                .w_full()
                .gap(8.0)
                .child(text(name.to_uppercase()).font(px::pixel(10.0)).color(px::FAINT).w(110.0).truncate())
                .child(text(value.clone()).font(rune::TextStyle::new(12.0, 400.0).mono()).color(px::SOFT).wrap().grow(1.0).min_w(0.0).selectable(true)),
        );
    }
    let mut nums: Vec<String> = mem.numbers.iter().map(|(k, v)| format!("{k} {v}")).collect();
    if let Some(a) = e.and_then(|e| e.analysis.as_ref()) {
        nums.push(format!("lufs {}", a.lufs.map_or("–".into(), |l| format!("{l:.1}"))));
        nums.push(format!("peak {:.1}", a.peak_dbfs));
    }
    if !nums.is_empty() {
        p = p.child(text(nums.join(" · ").to_uppercase()).font(px::pixel(10.0)).color(px::SILK).wrap().selectable(true));
    }
    if let Some(e) = e {
        let path = e.item.path.display().to_string();
        let copy = path.clone();
        p = p.child(
            row()
                .w_full()
                .gap(6.0)
                .center_y()
                .child(text(path).font(rune::TextStyle { size: 11.0, ..k.ty.mono }).color(px::FAINT).wrap().grow(1.0).min_w(0.0).selectable(true))
                .child(px::key("COPY", false).key("copy-path").help("Copy the path").on_press_tx(move |s: &mut App, tx: &mut Tx| {
                    tx.copy(copy.clone());
                    s.toast = Some(("Path copied".into(), false));
                })),
        );
    }
    let _ = s;
    p
}

/// Keep, drop, move: the board's choices for the file you're hearing.
fn decisions(s: &App, t: &Tab, mem: &Member, _k: &Kit) -> Option<El> {
    let b = t.board()?;
    if b.choices.is_empty() && b.move_to.is_empty() {
        return None;
    }
    let now = s.decisions.get(&mem.item);
    let mut r = row().key("decide").w_full().gap(6.0).center_y().flex_wrap().child(px::label("Decision").pr(4.0));
    for ch in &b.choices {
        let on = now == Some(ch);
        let (id, v) = (mem.item.clone(), ch.clone());
        let hint = match ch.as_str() {
            "keep" => " · K",
            "drop" => " · D",
            _ => "",
        };
        r = r.child(px::key(ch.to_uppercase(), on).key(format!("decide-{ch}")).help(format!("{ch}{hint}")).on_press(move |s: &mut App| s.decide(&id, &v)));
    }
    if !b.move_to.is_empty() {
        let moved = now.and_then(|n| n.strip_prefix("move:"));
        r = r.child(px::key(moved.map_or_else(|| "MOVE TO…".to_owned(), |m| format!("MOVED TO {}", m.to_uppercase())), moved.is_some()).key("move").on_press(|s: &mut App| {
            if let Some(t) = s.current_mut() {
                t.move_open ^= true;
            }
        }));
    }
    let mut out = col().w_full().gap(6.0).child(r);
    if t.move_open {
        let mut targets = row().key("move-targets").w_full().gap(4.0).flex_wrap();
        for g in &b.move_to {
            let (id, v) = (mem.item.clone(), format!("move:{g}"));
            targets = targets.child(px::key(g.to_uppercase(), false).key(format!("move-{g}")).on_press(move |s: &mut App| s.decide(&id, &v)));
        }
        out = out.child(targets);
    }
    Some(out)
}

/// The ranked rest, folded.
fn more(s: &App, t: &Tab, g: &Group, width: f32, k: &Kit) -> El {
    let open = t.more_open.contains(&g.key);
    let key = g.key.clone();
    let toggle = px::key(if open { format!("HIDE THE OTHER {} ▾", g.more.len()) } else { format!("THE OTHER {} RANKED ▸", g.more.len()) }, false)
        .key("more-toggle")
        .on_press(move |s: &mut App| s.toggle_more(&key));
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
                .fill(if on { px::SELECTED } else { Color::TRANSPARENT })
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label(format!("Play take {}", i + 1))
                .child(text(format!("#{}", i + 1)).font(px::pixel(11.0)).color(px::OFF).w(30.0))
                .child(
                    text(m.label.clone().or_else(|| s.entry(&m.item).map(|e| e.item.name.clone())).unwrap_or_default())
                        .font(rune::TextStyle::new(12.0, 400.0).mono())
                        .color(if on { px::LED } else { px::SOFT })
                        .w(140.0)
                        .truncate(),
                )
                .child(text(nums.to_uppercase()).font(px::pixel(10.0)).color(px::SILK).truncate().grow(1.0).min_w(0.0))
                .child(text(s.decisions.get(&m.item).cloned().unwrap_or_default().to_uppercase()).font(px::pixel(10.0)).color(px::SILK))
                .on_press(move |s: &mut App| s.board_play(&key, idx)),
        );
    }
    p
}
