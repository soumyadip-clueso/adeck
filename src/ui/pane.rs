//! The pane: everything about what you're hearing that isn't the sound itself (metadata, the
//! diff, earshot, the target, your note) in one column docked left or right of the page, or
//! hidden to a thin tab on the edge. Drag its inner edge to resize, drag its grip across to
//! dock it on the other side; `\` hides it, ⌘[ and ⌘] dock it, ⌘1 to ⌘5 open a section.

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

/// The pane's width: what you dragged it to, but never more than 60% of the window, and on a
/// small window never more than 42%, so the page keeps the larger share.
fn pane_w(s: &App, width: f32) -> f32 {
    let cap = if width < 1000.0 { width * 0.42 } else { width * 0.6 };
    s.sheet_w.clamp(MIN_W.min(cap), cap.max(MIN_W.min(cap)))
}

fn current(s: &App, t: &Tab) -> &'static str {
    let all = sections(t);
    all.iter().copied().find(|x| *x == s.pane_tab).unwrap_or(all.first().copied().unwrap_or(""))
}

/// How far the grip has to travel across the window before the pane docks on the other side.
const REDOCK: f32 = 120.0;

/// Where a grip drag of `dx` would dock the pane, docked now on `side`.
fn drop_side(side: Dock, dx: f32) -> Dock {
    match side {
        Dock::Left if dx > REDOCK => Dock::Right,
        Dock::Right if dx < -REDOCK => Dock::Left,
        other => other,
    }
}

/// `page` with the pane beside it on the docked side (or its edge tab when hidden). While the
/// grip is dragged, an outline shows where the pane will land.
pub fn wrap(s: &App, t: &Tab, page: El, width: f32, k: &Kit) -> El {
    let frame = row().key("with-pane").w_full().h_full().min_h(0.0);
    if sections(t).is_empty() || width < 700.0 {
        return frame.child(page);
    }
    let side = if s.pane == Dock::Hidden { s.pane_last } else { s.pane };
    let pw = pane_w(s, width);
    let pane = if s.pane == Dock::Hidden { edge_tab(side) } else { build(s, t, pw, k) };
    let mut frame = if side == Dock::Left { frame.child(pane).child(page) } else { frame.child(page).child(pane) };
    if let Some(dx) = s.pane_drag {
        let to = drop_side(side, dx);
        let moving = to != side;
        let x = if to == Dock::Left { 0.0 } else { width - pw };
        let what = match (to, moving) {
            (Dock::Left, true) => "DROP TO DOCK LEFT",
            (_, true) => "DROP TO DOCK RIGHT",
            _ => "DRAG ACROSS TO MOVE IT",
        };
        frame = frame.child(
            col()
                .key("pane-drop")
                .absolute(x + 6.0, 6.0)
                .w(pw - 12.0)
                .h_full()
                .pb(12.0)
                .center()
                .radius(8.0)
                .fill(if moving { px::AMBER.alpha(0.07) } else { Color::TRANSPARENT })
                .border(if moving { 2.0 } else { 1.0 }, if moving { px::AMBER.alpha(0.8) } else { px::GRILLE_ON })
                .inert()
                .child(text(what).font(px::pixel(13.0)).color(if moving { px::AMBER } else { px::SILK })),
        );
    }
    frame
}

/// The hidden pane: a thin strip on its edge. Click it (or press `\`) to bring the pane back.
fn edge_tab(side: Dock) -> El {
    let border = if side == Dock::Left { [0.0, 1.0, 0.0, 0.0] } else { [0.0, 0.0, 0.0, 1.0] };
    let mut letters = col().gap(3.0).center();
    for ch in "PANE".chars() {
        letters = letters.child(text(ch.to_string()).font(px::pixel(10.0)).color(px::FAINT));
    }
    col()
        .key("pane-edge")
        .w(EDGE)
        .h_full()
        .pt(16.0)
        .gap(14.0)
        .items_center()
        .fill(px::RAISED)
        .border_sides(border, px::LINE)
        .hover(px::SELECTED)
        .cursor(rune::Cursor::Pointer)
        .help("Show the pane · \\")
        .child(text(if side == Dock::Left { "›" } else { "‹" }).font(px::pixel(16.0)).color(px::SOFT))
        .child(letters)
        .on_press(|s: &mut App| s.toggle_pane())
}

fn build(s: &App, t: &Tab, w: f32, k: &Kit) -> El {
    let here = current(s, t);
    let left = s.pane == Dock::Left;
    let dragging = s.pane_drag.is_some();

    // The grip: six dots. Drag it across the window to dock the pane on the other side.
    let mut dots = col().gap(3.0);
    for _ in 0..3 {
        dots = dots.child(row().gap(3.0).child(el().size(3.0, 3.0).fill(px::SILK)).child(el().size(3.0, 3.0).fill(px::SILK)));
    }
    let grip = row()
        .key("pane-grip")
        .size(26.0, 32.0)
        .center()
        .radius(5.0)
        .fill(if dragging { px::SELECTED } else { Color::TRANSPARENT })
        .hover(px::SELECTED)
        .cursor(if dragging { rune::Cursor::Grabbing } else { rune::Cursor::Grab })
        .help(if left { "Drag right to dock the pane on the right · ⌘]" } else { "Drag left to dock the pane on the left · ⌘[" })
        .child(dots)
        .on_drag(move |s: &mut App, d: &mut Drag, _tx: &mut Tx| match d.phase {
            Phase::Up => {
                let side = if left { Dock::Left } else { Dock::Right };
                let to = drop_side(side, d.dx);
                if to != side {
                    s.set_pane(to);
                }
                s.pane_drag = None;
            }
            _ => s.pane_drag = Some(d.dx),
        });

    // The sections, as tabs with an underline on the open one. ⌘1 to ⌘5 open them.
    // Below ~420 points the head tightens: closer tabs, one dock button.
    let compact = w < 420.0;
    let mut tabs = row().h_full().gap(if compact { 0.0 } else { 2.0 });
    for (i, &name) in sections(t).iter().enumerate() {
        let on = name == here;
        tabs = tabs.child(
            col()
                .key(format!("pane-tab-{}", name.to_lowercase()))
                .h_full()
                .cursor(rune::Cursor::Pointer)
                .help(format!("{} · ⌘{}", name.to_lowercase(), i + 1))
                .child(spacer())
                .child(row().h(30.0).px(if compact { 5.0 } else { 9.0 }).center().radius(4.0).hover(px::SELECTED).child(text(name).font(px::pixel(12.0)).color(if on {
                    px::LED
                } else {
                    px::SILK
                })))
                .child(spacer())
                .child(el().h(2.0).w_full().fill(if on { px::LED } else { Color::TRANSPARENT }))
                .on_press(move |s: &mut App| s.pane_tab = name.to_owned()),
        );
    }

    // The pane's inner edge, full height: drag it to resize. A line shows while you do.
    let resizing = s.pane_resizing;
    let resize = row()
        .key("pane-resize")
        .absolute(if left { w - 8.0 } else { 0.0 }, 0.0)
        .w(8.0)
        .h_full()
        .cursor(rune::Cursor::ResizeColumn)
        .hover(px::GRILLE.alpha(0.6))
        .help("Drag to resize the pane")
        .child(spacer())
        .child(el().w(2.0).h_full().fill(if resizing { px::AMBER } else { Color::TRANSPARENT }))
        .child(spacer())
        .on_drag(move |s: &mut App, d: &mut Drag, _tx: &mut Tx| {
            if matches!(d.phase, Phase::Down) {
                d.data = s.sheet_w;
            }
            s.sheet_w = (if left { d.data + d.dx } else { d.data - d.dx }).clamp(MIN_W, 900.0);
            s.pane_resizing = !matches!(d.phase, Phase::Up);
            if matches!(d.phase, Phase::Up) {
                s.mark_dirty();
            }
        });
    let head = row()
        .key("pane-head")
        .w_full()
        .h(48.0)
        .pl(if compact { 4.0 } else { 8.0 })
        .pr(if compact { 4.0 } else { 8.0 })
        .gap(if compact { 2.0 } else { 6.0 })
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], px::LINE)
        .child(grip)
        .child(tabs)
        .child(spacer())
        .child(px::dock_buttons(s, compact));
    let inner = w - 40.0;
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
    col()
        .key("pane")
        .w(w)
        .h_full()
        .min_h(0.0)
        .fill(px::RAISED)
        .border_sides(border, px::LINE)
        .opacity(if dragging { 0.55 } else { 1.0 })
        .transition(rune::Transition::opacity())
        .child(head)
        .child(
            col()
                .key("pane-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(rune::ScrollSpec { stick_bottom: false })
                .child(col().w_full().px(20.0).pt(16.0).pb(24.0).gap(12.0).child(body)),
        )
        .child(resize)
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
    col().key("pane-meta").w_full().children(rows.iter().enumerate().map(|(i, (k, v))| px::kv(k, v, i + 1 == n)).collect::<Vec<_>>())
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
