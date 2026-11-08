//! The sheet: the focused group's detail (graph first, then the target bars, what made it,
//! the decision and the ranked rest) in a panel that slides in on the right, beside the tiles
//! or pads instead of under them. Drag its left edge to resize; × or Esc closes it. In a
//! narrow window it takes the page, with a way back.

use rune::{Drag, El, Phase, Tx, col, el, row, spacer, text};
use rune_kit::{IconButton, IconButtonVariant, Kit};

use crate::app::{App, Tab};
use crate::ui::{board, icons};

/// Below this the sheet takes the whole page.
pub const NARROW: f32 = 760.0;
const GAP: f32 = 14.0;

/// The width the main column keeps beside an open sheet.
pub fn main_width(s: &App, t: &Tab, width: f32) -> f32 {
    if is_open(t) && width >= NARROW { width - sheet_w(s, width) - GAP } else { width }
}

pub fn is_open(t: &Tab) -> bool {
    t.sheet_open && t.focused().is_some()
}

fn sheet_w(s: &App, width: f32) -> f32 {
    s.sheet_w.clamp(320.0, (width * 0.7).max(320.0))
}

/// `main` with the sheet beside it (or instead of it, when narrow).
pub fn wrap(s: &App, t: &Tab, main: El, width: f32, k: &Kit) -> El {
    if !is_open(t) {
        return main;
    }
    let Some(g) = t.focused() else { return main };
    let c = &k.c;
    let narrow = width < NARROW;
    let w = if narrow { width } else { sheet_w(s, width) };
    let label = t.board_label(g, t.active);
    let close = IconButton::new(if narrow { &icons::CHEVRON_RIGHT } else { &icons::CLOSE })
        .variant(IconButtonVariant::Ghost)
        .label(if narrow { "Back to the list" } else { "Close the sheet · Esc" })
        .on_press(|s: &mut App| s.toggle_sheet())
        .build(k)
        .key("sheet-close");
    let head = row()
        .key("sheet-head")
        .w_full()
        .gap(8.0)
        .center_y()
        .child(col().gap(1.0).min_w(0.0).shrink().child(text(g.title().to_owned()).font(k.ty.ui_medium).color(c.text_base).truncate()).child(text(label).font(k.ty.small).color(c.text_muted)))
        .child(spacer())
        .child(close);
    let inner = w - 28.0;
    let body = col().key("sheet-scroll").w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(col().w_full().pr(4.0).child(board::detail(s, t, g, inner, k)));
    let mut sheet = col()
        .key("sheet")
        .w(w)
        .h_full()
        .min_h(0.0)
        .px(12.0)
        .py(12.0)
        .gap(10.0)
        .radius(k.radius.lg)
        .fill(c.layer_01)
        .border(1.0, c.border_weak)
        .transition(rune::Transition::offset(48.0, 0.0).combined(rune::Transition::opacity()))
        .child(head)
        .child(body);
    if narrow {
        return sheet;
    }
    // The left edge: drag to resize.
    let grip = el()
        .key("sheet-grip")
        .w(GAP)
        .h_full()
        .cursor(rune::Cursor::ResizeColumn)
        .help("Drag to resize the sheet")
        .child(row().w_full().h_full().center().child(el().w(3.0).h(36.0).radius(2.0).fill(c.border_strong)))
        .on_drag(|s: &mut App, d: &mut Drag, _tx: &mut Tx| {
            if matches!(d.phase, Phase::Down) {
                d.data = s.sheet_w;
            }
            s.sheet_w = (d.data - d.dx).clamp(320.0, 1400.0);
        });
    sheet = sheet.animate_layout();
    row().key("with-sheet").w_full().h_full().min_h(0.0).child(col().w(width - w - GAP).h_full().min_h(0.0).child(main)).child(grip).child(sheet)
}
