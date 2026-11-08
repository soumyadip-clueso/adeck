//! Pads: for one-shot effects, each file is a big button. Click it (or its key) and it plays
//! from the top, cutting off whatever was playing; it lights and fills as it plays. Score it
//! 0 to 3 on the pad, keep or drop it, and open the strips only when you want them.

use rune::{Color, El, Tx, col, el, row, spacer, text};
use rune_kit::Kit;

use crate::app::{App, Tab};
use crate::board::Group;
use crate::ui::{board, px, widgets};

/// Keys for pads, in order across the focused group.
pub const KEYS: &str = "1234567890qwertyuiop";
const PAD_MIN: f32 = 150.0;
const GAP: f32 = 10.0;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let Some(b) = t.board() else { return col() };
    let mut tally = row().gap(14.0).center_y();
    for (tone, n) in b.tally() {
        let ink = match tone.as_str() {
            "good" => px::GOOD,
            "close" => px::CLOSE,
            "bad" => px::BAD,
            _ => px::SILK,
        };
        tally = tally.child(px::lamp(&format!("{n} {tone}"), ink));
    }
    let toggles = row()
        .gap(8.0)
        .center_y()
        .child(widgets::search_field(t, "Search", 200.0, k))
        .child(px::toggle("Blind", t.spec.blind).key("blind").help("Hide names until you pick · B").on_press(|s: &mut App| s.toggle_blind()))
        .child(
            px::toggle("Match", t.spec.loudness_match).key("match").help("Match loudness: trim every pad to the same LUFS · M").on_press(|s: &mut App| s.toggle_loudness_match()),
        );
    let head = px::header("pads-head", t.spec.label.clone().unwrap_or_else(|| "Pads".into()), tally, toggles);
    let mut page = col().key("pads").w_full().h_full().min_h(0.0).gap(14.0).child(head);
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    let focused = t.focused().map(|g| g.key.clone());
    let inner = width - 14.0;
    let mut list = col().key("pad-groups").w_full().gap(24.0).pr(10.0);
    let shown = s.shown_groups(t);
    for g in &shown {
        let on = focused.as_deref() == Some(g.key.as_str());
        list = list.child(group(s, t, g, on, inner, k));
    }
    let hint = |m: String| text(m).font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).py(20.0);
    if shown.is_empty() && !b.groups.is_empty() {
        list = list.child(hint(format!("No sound matches “{}”.", t.filter.text.trim())));
    }
    if b.groups.is_empty() {
        list = list.child(hint("No pads yet. The agent adds them as sounds land.".into()));
    }
    page.child(col().key("pads-scroll").w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
}

fn group(s: &App, t: &Tab, g: &Group, on: bool, width: f32, k: &Kit) -> El {
    let hidden = t.group_hidden(&g.key);
    let mut head = row().w_full().gap(12.0).center_y().child(text(g.title().to_uppercase()).font(px::pixel(20.0)).color(if on { px::LED } else { px::SILK }));
    if let Some(sub) = &g.subtitle {
        head = head.child(text(sub.to_uppercase()).font(px::pixel(11.0)).color(px::FAINT));
    }
    head = head.child(spacer());
    if let Some(ch) = g.chip.as_ref().filter(|_| !hidden) {
        head = head.child(board::chip(ch, k));
    }
    let mut out = col().key(format!("padgroup-{}", g.key)).w_full().gap(10.0).child(head);
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
    out
}

/// One pad: press to play from the top. Its key is printed big; it lights amber and fills as
/// it plays, and carries its score.
#[allow(clippy::too_many_arguments)]
fn pad(s: &App, t: &Tab, g: &Group, m: usize, focused: bool, pos: usize, w: f32, k: &Kit) -> El {
    let all = g.playable();
    let mem = all[m];
    let hidden = m < g.members.len() && t.group_hidden(&g.key);
    let e = s.entry(&mem.item);
    let active = focused && t.active == m;
    let playing = active && s.mixer_is_for(t.id()) && s.mixer.is_playing();
    let label = t.board_label(g, m);
    let key_char = KEYS.chars().nth(pos).filter(|_| focused).map(|ch| ch.to_ascii_uppercase().to_string());

    let mut top = row().w_full().gap(6.0).center_y().child(text(key_char.clone().unwrap_or_else(|| "·".into())).font(px::pixel(28.0)).color(if playing {
        px::AMBER
    } else if active {
        px::LED
    } else {
        px::OFF
    }));
    top = top.child(spacer());
    if playing {
        top = top.child(px::lamp("Playing", px::AMBER));
    } else if !hidden && e.is_some_and(|e| e.item.is_ref) {
        top = top.child(px::tag("REF", px::SILK));
    }
    if let Some(d) = s.decisions.get(&mem.item) {
        top = top.child(text(d.to_uppercase()).font(px::pixel(10.0)).color(if d == "drop" { px::BAD } else { px::SILK }));
    }
    let name = if hidden {
        label.clone()
    } else {
        mem.label.clone().unwrap_or_else(|| if m < g.members.len() { label.clone() } else { e.map_or(label.clone(), |e| e.item.name.clone()) })
    };
    let ink = if hidden { px::LED } else { board::role_color(&mem.role, k) };
    let mut body = col().w_full().gap(6.0).child(top).child(text(name).font(rune::TextStyle::new(13.0, 500.0).mono()).color(if active { px::LED } else { ink }).truncate());
    if !hidden {
        let nums = board::numbers_line(&mem.numbers, s, t, g);
        if !nums.is_empty() {
            body = body.child(text(nums.to_uppercase()).font(px::pixel(10.0)).color(px::SILK).truncate());
        }
    }
    let secs = e.and_then(|e| e.analysis.as_ref()).map(|a| a.secs);
    let now = s.scores.get(&mem.item).copied();
    let mut dots = row().gap(3.0).center_y();
    for v in 0..4i64 {
        let on = now == Some(v);
        let id = mem.item.clone();
        dots = dots.child(
            row()
                .key(format!("pad-score-{}-{m}-{v}", g.key))
                .size(20.0, 20.0)
                .center()
                .radius(3.0)
                .fill(if on { px::LED } else { Color::TRANSPARENT })
                .border(1.0, if on { px::LED } else { px::GRILLE })
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label(format!("Score {v}"))
                .help(format!("Score {v} · with the pad last played, Z X C V score 0-3"))
                .child(text(v.to_string()).font(px::pixel(11.0)).color(if on { px::CASE } else { px::SILK }))
                .on_press(move |s: &mut App| s.score_item(&id, v)),
        );
    }
    let len = secs.map_or(String::new(), |x| if x < 1.0 { format!("{:.0} MS", x * 1000.0) } else { format!("{x:.1} S") });
    body = body.child(row().w_full().center_y().child(dots).child(spacer()).child(text(len).font(px::pixel(10.0)).color(px::FAINT)));

    // LED segments along the bottom that light as it plays.
    let frac = if playing { secs.filter(|x| *x > 0.0).map_or(0.0, |x| (s.mixer.secs() / x).clamp(0.0, 1.0) as f32) } else { 0.0 };
    const SEGS: usize = 16;
    let lit = (frac * SEGS as f32).round() as usize;
    let mut bar = row().key(format!("pad-bar-{}-{m}", g.key)).instant_changes().w_full().h(3.0).gap(2.0);
    for i in 0..SEGS {
        bar = bar.child(el().grow(1.0).w(0.0).h_full().fill(if i < lit {
            px::AMBER
        } else if playing {
            px::GRILLE_ON
        } else {
            Color::TRANSPARENT
        }));
    }
    body = body.child(bar);
    let key = g.key.clone();
    col()
        .key(format!("pad-{}-{m}", g.key))
        .w(w)
        .min_h(128.0)
        .px(12.0)
        .py(10.0)
        .radius(6.0)
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
        .help(match &key_char {
            Some(kc) => format!("Play {label} · {kc} · double-click sends it to the agent"),
            None => format!("Play {label} · double-click sends it to the agent"),
        })
        .motion(rune::Spring::SNAPPY)
        .child(body)
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
