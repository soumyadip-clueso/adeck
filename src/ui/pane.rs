//! The pane: everything about what you're hearing that isn't the sound itself (metadata, the
//! diff, earshot, the target, your note) in one column docked left or right of the page, or
//! hidden to a thin tab on the edge. Drag its inner edge to resize; `\` hides and brings it back.

use rune::{Color, Drag, El, Phase, Tx, col, el, row, spacer, text};
use rune_kit::Kit;

use crate::app::{App, Entry, Tab, TabKind};
use crate::ui::px::{self, Dock};
use crate::ui::{board, widgets};

const MIN_W: f32 = 280.0;

/// The pane's sections for a tab, or none for tabs that have no pane.
pub fn sections(t: &Tab) -> &'static [&'static str] {
    match t.spec.kind {
        TabKind::Single => &["META", "EARSHOT", "NOTE"],
        TabKind::Compare => &["DIFF", "META", "EARSHOT", "NOTE"],
        TabKind::Board => &["TARGET", "NOTE"],
        TabKind::Run => &["META", "NOTE"],
        TabKind::Render => &["NOTE"],
        TabKind::Diff | TabKind::Shared => &[],
    }
}

/// The width the pane takes from the page right now: 0 when hidden or absent, the edge tab's
/// width when hidden.
pub fn taken(s: &App, t: &Tab, width: f32) -> f32 {
    if sections(t).is_empty() || width < 700.0 {
        0.0
    } else if s.pane == Dock::Hidden {
        EDGE
    } else {
        pane_w(s, width)
    }
}

const EDGE: f32 = 28.0;

fn pane_w(s: &App, width: f32) -> f32 {
    s.sheet_w.clamp(MIN_W, (width * 0.5).max(MIN_W))
}

fn current(s: &App, t: &Tab) -> &'static str {
    let all = sections(t);
    all.iter().copied().find(|x| *x == s.pane_tab).unwrap_or(all.first().copied().unwrap_or(""))
}

/// `page` with the pane beside it on the docked side (or its edge tab when hidden).
pub fn wrap(s: &App, t: &Tab, page: El, width: f32, k: &Kit) -> El {
    let frame = row().key("with-pane").w_full().h_full().min_h(0.0);
    if sections(t).is_empty() || width < 700.0 {
        return frame.child(page);
    }
    let side = if s.pane == Dock::Hidden { s.pane_last } else { s.pane };
    let pane = if s.pane == Dock::Hidden { edge_tab(side) } else { build(s, t, pane_w(s, width), k) };
    if side == Dock::Left { frame.child(pane).child(page) } else { frame.child(page).child(pane) }
}

fn edge_tab(side: Dock) -> El {
    let border = if side == Dock::Left { [0.0, 1.0, 0.0, 0.0] } else { [0.0, 0.0, 0.0, 1.0] };
    col()
        .key("pane-edge")
        .w(EDGE)
        .h_full()
        .center()
        .gap(10.0)
        .fill(px::RAISED)
        .border_sides(border, px::LINE)
        .cursor(rune::Cursor::Pointer)
        .help("Show the pane · \\")
        .child(text(if side == Dock::Left { "›" } else { "‹" }).font(px::pixel(13.0)).color(px::FAINT))
        .on_press(|s: &mut App| s.toggle_pane())
}

fn build(s: &App, t: &Tab, w: f32, k: &Kit) -> El {
    let here = current(s, t);
    let mut tabs = row().gap(14.0).center_y();
    for &name in sections(t) {
        let on = name == here;
        tabs = tabs.child(
            text(name)
                .key(format!("pane-tab-{}", name.to_lowercase()))
                .font(px::pixel(12.0))
                .color(if on { px::LED } else { px::OFF })
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut App| s.pane_tab = name.to_owned()),
        );
    }
    let left = s.pane == Dock::Left;
    let grip = col()
        .key("pane-grip")
        .w(8.0)
        .gap(2.0)
        .cursor(rune::Cursor::ResizeColumn)
        .help("Drag to resize the pane")
        .children((0..3).map(|_| el().w(8.0).h(2.0).fill(px::GRILLE_ON)).collect::<Vec<_>>())
        .on_drag(move |s: &mut App, d: &mut Drag, _tx: &mut Tx| {
            if matches!(d.phase, Phase::Down) {
                d.data = s.sheet_w;
            }
            s.sheet_w = (if left { d.data + d.dx } else { d.data - d.dx }).clamp(MIN_W, 900.0);
            if matches!(d.phase, Phase::Up) {
                s.mark_dirty();
            }
        });
    let head = row()
        .key("pane-head")
        .w_full()
        .h(44.0)
        .pl(16.0)
        .pr(8.0)
        .gap(16.0)
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], px::LINE)
        .child(grip)
        .child(tabs)
        .child(spacer())
        .child(px::dock_buttons(s));
    let inner = w - 32.0;
    let body = match here {
        "META" => meta(s, t, k),
        "EARSHOT" => focus_entry(s, t).map_or_else(|| hint("Nothing selected."), |e| widgets::earshot_panel(s, e, k)),
        "NOTE" => note(s, t, k),
        "DIFF" => diff(s, t),
        "TARGET" => match t.focused() {
            Some(g) => board::detail(s, t, g, inner, k),
            None => hint("Play a sound to see it against the target."),
        },
        _ => el(),
    };
    let border = if left { [0.0, 1.0, 0.0, 0.0] } else { [0.0, 0.0, 0.0, 1.0] };
    col().key("pane").w(w).h_full().min_h(0.0).fill(px::RAISED).border_sides(border, px::LINE).transition(rune::Transition::opacity()).child(head).child(
        col().key("pane-scroll").w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(col().w_full().px(16.0).py(16.0).gap(12.0).child(body)),
    )
}

fn hint(msg: &str) -> El {
    text(msg.to_owned()).font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).wrap()
}

/// The entry the pane is about: the file, the member you hear, or the run's selected row.
fn focus_entry<'a>(s: &'a App, t: &Tab) -> Option<&'a Entry> {
    let id = match t.spec.kind {
        TabKind::Run => t.table.cursor().map(str::to_owned).or_else(|| t.table.selected().next().map(str::to_owned))?,
        _ => t.spec.items.get(t.active).cloned()?,
    };
    s.entry(&id)
}

/// One labelled row: a pixel name and a mono value.
fn kv(name: &str, value: &str, last: bool) -> El {
    let r = row()
        .w_full()
        .py(7.0)
        .child(text(name.to_uppercase()).font(px::pixel(11.0)).color(px::FAINT).w(96.0).pt(1.0).truncate().help_when_truncated(name.to_owned()))
        .child(text(value.to_owned()).font(rune::TextStyle::new(12.0, 400.0).mono().lh(18.0)).color(px::SOFT).wrap().grow(1.0).min_w(0.0).selectable(true));
    if last { r } else { r.border_sides([0.0, 0.0, 1.0, 0.0], px::SELECTED) }
}

fn meta(s: &App, t: &Tab, _k: &Kit) -> El {
    let Some(e) = focus_entry(s, t) else { return hint(if t.spec.kind == TabKind::Run { "Select a row to see what made it." } else { "Opening…" }) };
    if t.is_blind_hidden() {
        return hint("Blind: metadata shows after you pick.");
    }
    let mut rows: Vec<(String, String)> = e.item.meta.clone();
    if let Some(a) = &e.analysis {
        rows.push(("length".into(), format!("{} · {} ch", widgets::clock(a.secs), a.channels)));
    }
    if let Some(src) = e.item.meta_src.as_ref().and_then(|p| p.file_name()) {
        rows.push(("source".into(), src.to_string_lossy().into_owned()));
    }
    if rows.is_empty() {
        return hint("No metadata was found for this file.");
    }
    let n = rows.len();
    col().key("pane-meta").w_full().children(rows.iter().enumerate().map(|(i, (k, v))| kv(k, v, i + 1 == n)).collect::<Vec<_>>())
}

/// Only the fields that differ, one narrow column per member; long values stack instead.
fn diff(s: &App, t: &Tab) -> El {
    if t.is_blind_hidden() {
        return hint("Blind: names and metadata show after you pick.");
    }
    let items: Vec<Option<&crate::item::Item>> = t.spec.items.iter().map(|i| s.entry(i).map(|e| &e.item)).collect();
    let present: Vec<&crate::item::Item> = items.iter().flatten().copied().collect();
    let keys = crate::item::differing(&present);
    if keys.is_empty() {
        return hint(if present.len() < items.len() { "Loading metadata…" } else { "Their metadata is identical (or there is none)." });
    }
    let label = |m: usize| t.member_label(m);
    let ink = |m: usize| if m == t.active { px::LED } else { px::SILK };
    let mono = rune::TextStyle::new(12.0, 400.0).mono();
    let mut head = row().w_full().pb(8.0).border_sides([0.0, 0.0, 1.0, 0.0], px::GRILLE).child(text("ONLY DIFFS").font(px::pixel(11.0)).color(px::FAINT).w(84.0));
    for m in 0..items.len() {
        head = head.child(text(label(m)).font(px::pixel(13.0)).color(if m == t.active { px::LED } else { px::FAINT }).w(0.0).grow(1.0));
    }
    let mut out = col().key("pane-diff").w_full().child(head);
    let mut long = Vec::new();
    for key in &keys {
        let vals: Vec<String> = items.iter().map(|e| e.and_then(|e| e.get(key)).unwrap_or("–").to_owned()).collect();
        if vals.iter().any(|v| v.chars().count() > 10) {
            long.push((key.clone(), vals));
            continue;
        }
        let mut r = row()
            .w_full()
            .py(8.0)
            .border_sides([0.0, 0.0, 1.0, 0.0], px::SELECTED)
            .child(text(key.to_uppercase()).font(px::pixel(11.0)).color(px::FAINT).w(84.0).truncate().help_when_truncated(key.clone()));
        for (m, v) in vals.iter().enumerate() {
            r = r.child(text(v.clone()).font(mono).color(ink(m)).w(0.0).grow(1.0).truncate().help_when_truncated(v.clone()));
        }
        out = out.child(r);
    }
    for (key, vals) in long {
        let mut block = col().w_full().pt(12.0).gap(6.0).child(text(key.to_uppercase()).font(px::pixel(11.0)).color(px::FAINT));
        for (m, v) in vals.into_iter().enumerate() {
            block = block.child(
                row()
                    .gap(8.0)
                    .child(text(label(m)).font(px::pixel(12.0)).color(ink(m)).w(14.0))
                    .child(text(v).font(mono.lh(18.0)).color(ink(m)).wrap().grow(1.0).min_w(0.0).selectable(true)),
            );
        }
        out = out.child(block);
    }
    out
}

/// Your note for the agent, always open here.
fn note(_s: &App, t: &Tab, k: &Kit) -> El {
    let input = rune_kit::TextArea::new(&t.note)
        .placeholder("What should the agent know? Saved with your next score or pick.")
        .on_edit(|s: &mut App| &mut s.current_mut().expect("a tab").note)
        .build(k)
        .key("pane-note")
        .w_full()
        .min_h(120.0);
    let save = row()
        .key("pane-note-save")
        .gap(8.0)
        .child(px::key("SAVE", true).on_press(|s: &mut App| s.note_only()))
        .child(text("or send it with S").font(px::pixel(11.0)).color(px::FAINT).pt(7.0));
    col().w_full().gap(10.0).child(input).child(save).child(el().h(1.0).w_full().fill(Color::TRANSPARENT))
}
