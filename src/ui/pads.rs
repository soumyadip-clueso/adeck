//! Pads: for one-shot effects, each file is a big button. Click it (or its key) and it plays
//! from the top, cutting off whatever was playing; it lights and fills as it plays. Score it
//! 0 to 3 on the pad, keep or drop it, and open the strips only when you want them.

use rune::{Color, El, Tx, col, el, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Switch};

use crate::app::{App, Tab};
use crate::board::Group;
use crate::ui::{board, widgets};

/// Keys for pads, in order across the focused group.
pub const KEYS: &str = "1234567890qwertyuiop";
const PAD_MIN: f32 = 150.0;
const GAP: f32 = 10.0;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let Some(b) = t.board() else { return col() };
    let playing = s.mixer_is_for(t.id()) && s.mixer.is_playing();
    let stop = Button::new(if playing { "Stop" } else { "Replay" })
        .variant(ButtonVariant::Outline)
        .control_size(ControlSize::Small)
        .help(if playing { "Stop · Space" } else { "Play the last pad again · Space" })
        .on_press(|s: &mut App| s.pad_replay())
        .build(k)
        .key("pad-stop");
    let mut tally = row().gap(10.0).center_y();
    for (tone, n) in b.tally() {
        tally = tally.child(board::chip(&crate::board::Chip { text: format!("{n} {tone}"), tone }, k));
    }
    let head = row()
        .key("pads-head")
        .w_full()
        .gap(14.0)
        .center_y()
        .flex_wrap()
        .child(widgets::search_field(t, "Search sounds", 200.0, k))
        .child(stop)
        .child(widgets::meter(s, k))
        .child(widgets::volume(s, k))
        .child(tally)
        .child(spacer())
        .child(Switch::new(t.spec.blind).label("Blind").on_toggle(|s: &mut App| s.toggle_blind()).build(k).key("blind"))
        .child(Switch::new(t.spec.loudness_match).label("Match loudness").on_toggle(|s: &mut App| s.toggle_loudness_match()).build(k).key("match"))
        .child(
            Switch::new(t.sheet_open)
                .label("Waveforms")
                .on_toggle(|s: &mut App| s.toggle_sheet())
                .build(k)
                .key("pads-detail")
                .help("Show the waveform, spectrogram and target bars for the pad you played"),
        )
        .child(
            Button::new("Note")
                .variant(if t.note_open { ButtonVariant::Contrast } else { ButtonVariant::Outline })
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.toggle_note())
                .build(k)
                .key("note-toggle"),
        )
        .child(
            Button::new("Send to agent")
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .help("Hand the agent the pad you played last · S, or double-click a pad")
                .on_press_tx(|s: &mut App, tx: &mut Tx| s.send_to_agent(None, tx))
                .build(k)
                .key("send"),
        );
    let mut page = col().key("pads").w_full().h_full().min_h(0.0).gap(10.0).child(head).child(widgets::note_field(t, k));
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    let focused = t.focused().map(|g| g.key.clone());
    let inner = width - 14.0;
    let mut list = col().key("pad-groups").w_full().gap(18.0).pr(10.0);
    let shown = s.shown_groups(t);
    for g in &shown {
        let on = focused.as_deref() == Some(g.key.as_str());
        list = list.child(group(s, t, g, on, inner, k));
    }
    if shown.is_empty() && !b.groups.is_empty() {
        list = list.child(text(format!("No sound matches “{}”.", t.filter.text.trim())).font(k.ty.small).color(c.text_muted).py(20.0));
    }
    if b.groups.is_empty() {
        list = list.child(text("No pads yet. The agent adds them as sounds land.").font(k.ty.small).color(c.text_muted).py(20.0));
    }
    page.child(col().key("pads-scroll").w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
}

fn group(s: &App, t: &Tab, g: &Group, on: bool, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let hidden = t.group_hidden(&g.key);
    let mut head = row().w_full().gap(10.0).center_y().child(text(g.title().to_owned()).font(k.ty.ui_medium).color(if on { c.text_base } else { c.text_muted }));
    if let Some(sub) = &g.subtitle {
        head = head.child(text(sub.clone()).font(k.ty.small).color(c.text_faint));
    }
    head = head.child(spacer());
    if let Some(ch) = g.chip.as_ref().filter(|_| !hidden) {
        head = head.child(board::chip(ch, k));
    }
    let mut out = col().key(format!("padgroup-{}", g.key)).w_full().gap(8.0).child(head);
    let mut order = t.group_order(g);
    order.extend(g.members.len()..g.playable().len());
    let cols = (((width + GAP) / (PAD_MIN + GAP)).floor() as usize).clamp(1, order.len().max(1));
    let w = ((width - GAP * (cols as f32 - 1.0)) / cols as f32).min(240.0);
    for (r, chunk) in order.chunks(cols).enumerate() {
        let mut line = row().key(format!("padrow-{}-{r}", g.key)).w_full().gap(GAP);
        for (i, &m) in chunk.iter().enumerate() {
            line = line.child(pad(s, t, g, m, on, r * cols + i, w, k));
        }
        out = out.child(line);
    }
    let _ = on;
    out
}

/// One pad: press to play from the top. It flashes, fills as it plays, and carries its score.
#[allow(clippy::too_many_arguments)]
fn pad(s: &App, t: &Tab, g: &Group, m: usize, focused: bool, pos: usize, w: f32, k: &Kit) -> El {
    let c = &k.c;
    let all = g.playable();
    let mem = all[m];
    let hidden = m < g.members.len() && t.group_hidden(&g.key);
    let e = s.entry(&mem.item);
    let active = focused && t.active == m;
    let playing = active && s.mixer_is_for(t.id()) && s.mixer.is_playing();
    let label = t.board_label(g, m);
    let key_char = KEYS.chars().nth(pos).filter(|_| focused).map(|ch| ch.to_ascii_uppercase().to_string());
    let tint = if hidden { c.icon_accent } else { board::role_color(&mem.role, k) };

    let mut top = row().w_full().gap(6.0).center_y();
    if let Some(kc) = &key_char {
        top = top.child(row().px(5.0).py(1.0).radius(4.0).border(1.0, c.border_strong).child(text(kc.clone()).font(k.ty.small_medium.features("'tnum' 1")).color(c.text_muted)));
    }
    top = top.child(spacer());
    if !hidden && e.is_some_and(|e| e.item.is_ref) {
        top = top.child(widgets::ref_badge(k));
    }
    if let Some(d) = s.decisions.get(&mem.item) {
        top = top.child(text(d.clone()).font(k.ty.small).color(if d == "drop" { c.danger.fg } else { c.text_muted }));
    }
    let name = if hidden { label.clone() } else { mem.label.clone().unwrap_or_else(|| if m < g.members.len() { label.clone() } else { e.map_or(label.clone(), |e| e.item.name.clone()) }) };
    let mut body = col()
        .w_full()
        .gap(6.0)
        .child(top)
        .child(text(name).font(rune::TextStyle { size: 17.0, ..k.ty.ui_medium }).color(if active { c.text_base } else { tint }).truncate());
    if !hidden {
        let nums = board::numbers_line(&mem.numbers, s, t, g);
        if !nums.is_empty() {
            body = body.child(text(nums).font(k.ty.small.features("'tnum' 1")).color(c.text_muted).truncate());
        }
    }
    let secs = e.and_then(|e| e.analysis.as_ref()).map(|a| a.secs);
    // The score dots, 0 to 3.
    let now = s.scores.get(&mem.item).copied();
    let mut dots = row().gap(4.0).center_y();
    for v in 0..4i64 {
        let on = now == Some(v);
        let id = mem.item.clone();
        dots = dots.child(
            row()
                .key(format!("pad-score-{}-{m}-{v}", g.key))
                .size(22.0, 22.0)
                .center()
                .radius(11.0)
                .fill(if on { tint } else { Color::TRANSPARENT })
                .border(1.0, if on { tint } else { c.border_strong })
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label(format!("Score {v}"))
                .help(format!("Score {v} · with the pad last played, Z X C V score 0-3"))
                .child(text(v.to_string()).font(k.ty.small_medium).color(if on { Color::WHITE } else { c.text_muted }))
                .on_press(move |s: &mut App| s.score_item(&id, v)),
        );
    }
    let len = secs.map_or(String::new(), |x| if x < 1.0 { format!("{:.0} ms", x * 1000.0) } else { format!("{x:.1} s") });
    body = body.child(row().w_full().center_y().child(dots).child(spacer()).child(text(len).font(k.ty.small.features("'tnum' 1")).color(c.text_faint)));

    // A bar along the bottom that fills as it plays.
    let frac = if playing { secs.filter(|x| *x > 0.0).map_or(0.0, |x| (s.mixer.secs() / x).clamp(0.0, 1.0) as f32) } else { 0.0 };
    let progress = row().absolute(0.0, 0.0).w_full().h_full().inert().child(col().w_full().h_full().child(spacer()).child(row().w_full().h(3.0).child(el().w_frac(frac).h_full().fill(tint))));
    let key = g.key.clone();
    col()
        .key(format!("pad-{}-{m}", g.key))
        .w(w)
        .min_h(112.0)
        .px(12.0)
        .py(10.0)
        .radius(k.radius.lg)
        .fill(if playing {
            tint.alpha(0.22)
        } else if active {
            tint.alpha(0.12)
        } else {
            c.layer_01
        })
        .border(if active { 1.5 } else { 1.0 }, if active { tint.alpha(0.9) } else { c.border_weak })
        .clip_rounded(k.radius.lg)
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(format!("Play {label}"))
        .help(match &key_char {
            Some(kc) => format!("Play {label} · {kc} · double-click sends it to the agent"),
            None => format!("Play {label} · double-click sends it to the agent"),
        })
        .motion(rune::Spring::SNAPPY)
        .child(body)
        .child(progress)
        .on_press(move |s: &mut App| s.pad_play(&key, m))
        .on_double_press_tx(move |s: &mut App, tx: &mut Tx| s.send_to_agent(Some(m), tx))
}

/// A key on a pads tab: a pad's key plays it, Z X C V score the last one 0-3.
pub fn on_key(s: &mut App, ch: char) -> bool {
    let ch = ch.to_ascii_lowercase();
    match ch {
        'z' | 'x' | 'c' | 'v' => {
            s.score_active("zxcv".find(ch).unwrap_or(0) as i64);
            return true;
        }
        _ => {}
    }
    let Some(pos) = KEYS.find(ch) else { return false };
    let Some(t) = s.current() else { return false };
    let Some(g) = t.focused() else { return false };
    let mut order = t.group_order(g);
    order.extend(g.members.len()..g.playable().len());
    let Some(&m) = order.get(pos) else { return true };
    let key = g.key.clone();
    s.pad_play(&key, m);
    true
}
