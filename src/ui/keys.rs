//! The keyboard help sheet: `?` or the palette opens it, Esc or a click outside closes it.
//! The current tab's keys come first.

use rune::{Color, El, col, el, row, spacer, text};
use rune_kit::Kit;

use crate::app::{App, TabKind};

type Section = (&'static str, &'static [(&'static str, &'static str)]);

const GENERAL: Section = ("Everywhere", &[("⌘K", "Search or run a command"), ("⌘T", "New tab menu"), ("⌘O", "Open a file"), ("⌘W", "Close the tab"), ("⌘⇧E", "Share the tab as a page"), ("⌘⇧S", "Screenshot the tab"), ("/", "Search this tab"), ("?", "This sheet")]);
const PLAY: Section = ("Listening", &[("Space", "Play or pause"), ("← →", "Step a second (⇧ five)"), ("= −", "Zoom in or out"), ("F", "Fit the file or loop"), ("[ ]", "Pan"), ("Esc", "Clear the loop"), ("N", "Note for the agent"), ("S", "Send to agent")]);
const COMPARE: Section = ("Compare", &[("1–9", "Switch to a member"), ("↵", "Pick the one you hear"), ("B", "Blind on or off"), ("M", "Match loudness"), ("0–3", "Score (single file)")]);
const BOARD: Section = ("Board", &[("↑ ↓", "Previous or next group"), ("← →", "Previous or next tile"), ("1–9", "Play a tile"), ("R", "Replay from the start"), ("L", "Loop what the strip shows"), ("Z", "Zoom to the sound"), ("K D", "Keep or drop"), ("↵", "Pick"), ("Esc", "Waveforms sheet")]);
const PADS: Section = ("Pads", &[("1–0 Q–P", "Play a pad from the top"), ("Space", "Replay the last pad"), ("Z X C V", "Score it 0 to 3")]);

pub fn toggle(s: &mut App) {
    s.keys_help = !s.keys_help;
}

pub fn build(s: &App, width: f32, k: &Kit, layer: &mut Vec<El>) {
    if !s.keys_help {
        return;
    }
    let c = &k.c;
    let kind = s.current().map(|t| t.spec.kind);
    let pads = s.current().and_then(|t| t.board()).is_some_and(crate::board::Board::is_pads);
    let mut order: Vec<Section> = match kind {
        Some(TabKind::Board) if pads => vec![PADS, BOARD, PLAY],
        Some(TabKind::Board) => vec![BOARD, PLAY, PADS],
        Some(TabKind::Compare | TabKind::Render | TabKind::Single) => vec![PLAY, COMPARE],
        _ => vec![],
    };
    order.push(GENERAL);
    for x in [PLAY, COMPARE, BOARD, PADS] {
        if !order.iter().any(|o| o.0 == x.0) {
            order.push(x);
        }
    }
    let panel_w = (width - 32.0).min(720.0);
    let col_w = if panel_w > 560.0 { ((panel_w - 40.0 - 32.0) / 2.0).floor() } else { panel_w - 40.0 };
    let mut grid = row().w_full().flex_wrap().gap(24.0).px(20.0).py(18.0).items_start();
    for (i, (title, rows)) in order.iter().enumerate() {
        let mut sec = col().w(col_w).gap(6.0).child(
            row().gap(8.0).center_y().child(text(*title).font(k.ty.small_medium).color(c.text_faint)).child(if i == 0 && kind.is_some() { text("this tab").font(k.ty.small).color(c.icon_accent) } else { text(String::new()) }),
        );
        for (key, what) in *rows {
            sec = sec.child(row().w_full().gap(10.0).center_y().child(text(*what).font(k.ty.ui).color(c.text_base).truncate().shrink().min_w(0.0)).child(spacer()).child(keys_of(key, k)));
        }
        grid = grid.child(sec);
    }
    let panel = col()
        .key("keys-help")
        .w(panel_w)
        .radius(14.0)
        .fill(c.layer_01)
        .border(1.0, c.border_strong)
        .shadow(18.0, 48.0, Color::BLACK.alpha(0.45))
        .clip_rounded(14.0)
        .transition(rune::Transition::offset(0.0, -10.0).combined(rune::Transition::opacity()).combined(rune::Transition::scale(0.98)))
        .child(row().w_full().px(20.0).py(14.0).center_y().border_sides([0.0, 0.0, 1.0, 0.0], c.border_weak).child(text("Keyboard shortcuts").font(k.ty.ui_medium).color(c.text_base)).child(spacer()).child(crate::ui::palette::hint("esc", "close", k)))
        .child(col().w_full().max_h(560.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(grid));
    let panel = panel.modal(true, |s: &mut App| s.keys_help = false);
    layer.push(col().key("keys-scrim").absolute(0.0, 0.0).w_full().h_full().fill(Color::BLACK.alpha(0.4)).transition(rune::Transition::opacity()).a11y_hidden());
    layer.push(col().key("keys-layer").absolute(0.0, 0.0).w_full().h_full().child(el().h(70.0)).child(row().w_full().child(spacer()).child(panel).child(spacer())));
}

fn keys_of(spec: &str, k: &Kit) -> El {
    spec.split(' ').fold(row().gap(3.0), |r, p| r.child(crate::ui::palette::keycap(p, k)))
}
