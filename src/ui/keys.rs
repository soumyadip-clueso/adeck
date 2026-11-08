//! The keyboard help sheet: `?` or the palette opens it, Esc or a click outside closes it.
//! The current tab's keys come first.

use rune::{Color, El, col, el, row, spacer, text};
use rune_kit::Kit;

use crate::app::{App, TabKind};
use crate::ui::px;

type Section = (&'static str, &'static [(&'static str, &'static str)]);

const GENERAL: Section = (
    "Everywhere",
    &[
        ("⌘K", "Search or run a command"),
        ("⌘T", "New tab menu"),
        ("⌘O", "Open a file"),
        ("⌘W", "Close the tab"),
        ("⌘⇧E", "Share the tab as a page"),
        ("⌘⇧S", "Screenshot the tab"),
        ("⌘F  /", "Search this tab"),
        ("?", "This sheet"),
    ],
);
const PANE: Section = ("Pane", &[("\\", "Hide or show the pane"), ("⌘1–5", "Open a section"), ("⌘[", "Dock it left"), ("⌘]", "Dock it right"), ("N", "Your note for the agent")]);
const PLAY: Section = (
    "Listening",
    &[
        ("Space", "Play or pause"),
        ("← →", "Step a second (⇧ five)"),
        ("= −", "Zoom in or out"),
        ("F", "Fit the file or loop"),
        ("[ ]", "Pan"),
        ("⇧ drag", "Select a stretch (Space plays it)"),
        ("L", "Loop the selection"),
        ("Esc", "Clear the loop and selection"),
        ("S", "Send to agent"),
    ],
);
const COMPARE: Section =
    ("Compare", &[("1–9", "Switch to a member"), ("↵", "Pick the one you hear"), ("B", "Blind on or off"), ("M", "Match loudness"), ("0–3", "Score (single file)")]);
const BOARD: Section = (
    "Board",
    &[
        ("↑ ↓", "Previous or next group"),
        ("← →", "Previous or next tile"),
        ("1–9", "Play a tile"),
        ("R", "Replay from the start"),
        ("L", "Loop what the strip shows"),
        ("Z", "Zoom to the sound"),
        ("K D", "Keep or drop"),
        ("↵", "Pick"),
        ("Esc", "Hide or show the pane"),
    ],
);
const PADS: Section = ("Pads", &[("1–0 Q–P", "Play a pad from the top"), ("Space", "Replay the last pad"), ("Z X C V", "Score it 0 to 3")]);

pub fn toggle(s: &mut App) {
    s.keys_help = !s.keys_help;
}

pub fn build(s: &App, width: f32, k: &Kit, layer: &mut Vec<El>) {
    if !s.keys_help {
        return;
    }
    let kind = s.current().map(|t| t.spec.kind);
    let pads = s.current().and_then(|t| t.board()).is_some_and(crate::board::Board::is_pads);
    let mut order: Vec<Section> = match kind {
        Some(TabKind::Board) if pads => vec![PADS, BOARD, PLAY],
        Some(TabKind::Board) => vec![BOARD, PLAY, PADS],
        Some(TabKind::Compare | TabKind::Render | TabKind::Single) => vec![PLAY, COMPARE],
        _ => vec![],
    };
    order.push(PANE);
    order.push(GENERAL);
    for x in [PLAY, COMPARE, BOARD, PADS] {
        if !order.iter().any(|o| o.0 == x.0) {
            order.push(x);
        }
    }
    let panel_w = (width - 32.0).min(980.0);
    let cols = if panel_w > 900.0 {
        3.0
    } else if panel_w > 560.0 {
        2.0
    } else {
        1.0
    };
    let col_w = ((panel_w - 40.0 - 28.0 * (cols - 1.0)) / cols).floor();
    let mut grid = row().w_full().flex_wrap().gap(28.0).px(20.0).py(20.0).items_start();
    for (i, (title, rows)) in order.iter().enumerate() {
        let mut sec = col().w(col_w).gap(7.0).child(
            row().gap(10.0).center_y().pb(4.0).child(text(title.to_uppercase()).font(px::pixel(11.0)).color(px::FAINT)).child(if i == 0 && kind.is_some() {
                px::lamp("This tab", px::AMBER)
            } else {
                text(String::new())
            }),
        );
        for (key, what) in *rows {
            sec = sec.child(
                row()
                    .w_full()
                    .gap(10.0)
                    .center_y()
                    .child(text(*what).font(rune::TextStyle::new(13.0, 400.0)).color(px::SOFT).truncate().shrink().min_w(0.0))
                    .child(spacer())
                    .child(keys_of(key, k)),
            );
        }
        grid = grid.child(sec);
    }
    let panel = col()
        .key("keys-help")
        .w(panel_w)
        .radius(10.0)
        .fill(px::RAISED)
        .border(1.0, px::GRILLE_ON)
        .shadow(18.0, 48.0, Color::BLACK.alpha(0.6))
        .clip_rounded(10.0)
        .transition(rune::Transition::offset(0.0, -10.0).combined(rune::Transition::opacity()).combined(rune::Transition::scale(0.98)))
        .child(
            row()
                .w_full()
                .px(20.0)
                .py(14.0)
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], px::LINE)
                .child(text("KEYS").font(px::pixel(20.0)).color(px::LED))
                .child(spacer())
                .child(crate::ui::palette::hint("esc", "close", k)),
        )
        .child(col().w_full().max_h(620.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(grid));
    let panel = panel.modal(true, |s: &mut App| s.keys_help = false);
    layer.push(col().key("keys-scrim").absolute(0.0, 0.0).w_full().h_full().fill(Color::BLACK.alpha(0.6)).transition(rune::Transition::opacity()).a11y_hidden());
    layer.push(col().key("keys-layer").absolute(0.0, 0.0).w_full().h_full().child(el().h(56.0)).child(row().w_full().child(spacer()).child(panel).child(spacer())));
}

fn keys_of(spec: &str, k: &Kit) -> El {
    spec.split(' ').fold(row().gap(3.0), |r, p| r.child(crate::ui::palette::keycap(p, k)))
}
