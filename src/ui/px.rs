//! adeck's look: a sampler's dot-matrix screen. Near-black case, dark screens, lit-LED white,
//! one amber for "playing", and Geist Pixel for labels, numbers and titles. Everything here is
//! the one place those choices live; pages ask for `px::label`, `px::LED` and so on.

use rune::{Color, El, Family, TextStyle, el, row, text};
use rune_kit::Kit;

use crate::app::App;

/// The window behind everything.
pub const CASE: Color = lin(0.00335, 0.00335, 0.00368);
/// Waveform and spectrogram screens, cards, pads.
pub const SCREEN: Color = lin(0.00700, 0.00700, 0.00750);
/// A lifted surface: the pane, a selected row.
pub const RAISED: Color = lin(0.00561, 0.00561, 0.00605);
pub const SELECTED: Color = lin(0.00913, 0.00913, 0.01033);
/// Hairlines between regions.
pub const LINE: Color = lin(0.01033, 0.01033, 0.01161);
/// Outlines on keys and fields.
pub const GRILLE: Color = lin(0.01938, 0.01938, 0.02315);
pub const GRILLE_ON: Color = lin(0.04231, 0.04231, 0.04817);
/// Text: lit, label, faint, off.
pub const LED: Color = lin(0.88792, 0.88792, 0.88792);
pub const SOFT: Color = lin(0.67244, 0.67244, 0.70110);
pub const SILK: Color = lin(0.25415, 0.25415, 0.27889);
pub const FAINT: Color = lin(0.15593, 0.15593, 0.17465);
pub const OFF: Color = lin(0.06848, 0.06848, 0.08022);
/// The one colour: the playhead and whatever is sounding.
pub const AMBER: Color = lin(0.80695, 0.45079, 0.04231);
/// Verdicts only.
pub const GOOD: Color = lin(0.34670, 0.65141, 0.32314);
pub const CLOSE: Color = lin(0.74540, 0.47932, 0.11697);
pub const BAD: Color = lin(0.78354, 0.25415, 0.19462);

pub const PIXEL: &str = "Geist Pixel";

/// A colour in linear light (Rune's space), converted from the design's sRGB hex.
const fn lin(r: f32, g: f32, b: f32) -> Color {
    Color { r, g, b, a: 1.0 }
}

/// Geist Pixel at `size`. Big type gets round dots (it reads as LEDs, like the Dock icon);
/// small labels keep square pixels so they stay crisp.
pub fn pixel(size: f32) -> TextStyle {
    let round = if size >= 20.0 { "'ELSH' 100" } else { "'ELSH' 0" };
    TextStyle::new(size, 400.0).family(Family::Named(PIXEL)).variations(round).tracking(if size <= 13.0 { 0.06 * size } else { 0.0 }).features("'tnum' 1")
}

/// A small uppercase label: "LUFS", "SEED".
pub fn label(s: impl Into<String>) -> El {
    text(s.into().to_uppercase()).font(pixel(11.0)).color(FAINT)
}

/// A title: the page's name in big round-dot type.
pub fn title(s: impl Into<String>) -> El {
    text(s.into().to_uppercase()).font(pixel(30.0)).color(LED).truncate()
}

/// A page's one header line: the title with its lamps beside it on the left, the search and
/// switches on the right. It wraps only when the window is too narrow for both.
pub fn header(key: &str, title_text: impl Into<String>, lamps: El, tools: El) -> El {
    row()
        .key(key.to_owned())
        .w_full()
        .gap(16.0)
        .center_y()
        .flex_wrap()
        .child(row().gap(18.0).items_end().min_w(0.0).shrink().child(title(title_text)).child(lamps.pb(6.0)))
        .child(rune::spacer())
        .child(tools.flex_wrap().min_w(0.0).shrink())
}

/// A readout: label above a pixel number.
pub fn readout(name: &str, value: String) -> El {
    rune::col().gap(4.0).child(label(name)).child(text(value).font(pixel(22.0)).color(LED))
}

/// A hardware key: outlined, lit white when `on`.
pub fn key(label_text: impl Into<String>, on: bool) -> El {
    row()
        .h(28.0)
        .min_w(28.0)
        .px(8.0)
        .center()
        .radius(4.0)
        .fill(if on { LED } else { Color::TRANSPARENT })
        .border(1.0, if on { LED } else { GRILLE })
        .hover(if on { LED } else { SELECTED })
        .cursor(rune::Cursor::Pointer)
        .child(text(label_text.into()).font(pixel(13.0)).color(if on { CASE } else { SOFT }))
}

/// A key that shows its keyboard shortcut after its label, dimmer: "REPLAY  R".
pub fn key_k(label_text: impl Into<String>, shortcut: &str, on: bool) -> El {
    row()
        .h(28.0)
        .px(9.0)
        .gap(8.0)
        .center()
        .radius(4.0)
        .fill(if on { LED } else { Color::TRANSPARENT })
        .border(1.0, if on { LED } else { GRILLE })
        .hover(if on { LED } else { SELECTED })
        .cursor(rune::Cursor::Pointer)
        .child(text(label_text.into()).font(pixel(12.0)).color(if on { CASE } else { SOFT }))
        .child(text(shortcut.to_owned()).font(pixel(10.0)).color(if on { SILK } else { OFF }))
}

/// A square icon-sized button with a hover fill: the pane's dock and hide controls.
pub fn icon_btn(glyph: El, on: bool) -> El {
    row()
        .size(30.0, 30.0)
        .center()
        .radius(5.0)
        .fill(if on { SELECTED } else { Color::TRANSPARENT })
        .border(1.0, if on { GRILLE_ON } else { Color::TRANSPARENT })
        .hover(GRILLE)
        .cursor(rune::Cursor::Pointer)
        .child(glyph)
}

/// A section of the pane: a hairline above, its title (and anything on the right of it), then
/// its body.
pub fn section(key: &str, title_text: &str, trailing: Option<El>, body: El) -> El {
    let mut head = row().w_full().h(20.0).center_y().gap(8.0).child(text(title_text.to_uppercase()).font(pixel(11.0)).color(FAINT));
    if let Some(t) = trailing {
        head = head.child(rune::spacer()).child(t);
    }
    rune::col().key(key.to_owned()).w_full().pt(14.0).pb(16.0).gap(10.0).border_sides([1.0, 0.0, 0.0, 0.0], LINE).child(head).child(body)
}

/// A quiet text button for a section's header: "ALL FIELDS ▸".
pub fn link(label_text: impl Into<String>) -> El {
    row().h(20.0).px(6.0).center().radius(3.0).hover(SELECTED).cursor(rune::Cursor::Pointer).child(text(label_text.into().to_uppercase()).font(pixel(10.0)).color(SILK))
}

/// One labelled row: a pixel name and a mono value that wraps, with a hairline under it.
pub fn kv(name: &str, value: &str, last: bool) -> El {
    let r = row()
        .w_full()
        .py(7.0)
        .gap(12.0)
        .child(text(name.to_uppercase()).font(pixel(10.0)).color(FAINT).w(84.0).pt(2.0).truncate().help_when_truncated(name.to_owned()))
        .child(text(value.to_owned()).font(TextStyle::new(12.0, 400.0).mono().lh(18.0)).color(SOFT).wrap().grow(1.0).min_w(0.0).selectable(true));
    if last { r } else { r.border_sides([0.0, 0.0, 1.0, 0.0], SELECTED) }
}

/// Equal-width keys joined into one control, the lit one being the current choice.
pub fn segments(items: Vec<El>) -> El {
    row().w_full().gap(4.0).children(items.into_iter().map(|e| e.grow(1.0).min_w(0.0)).collect::<Vec<_>>())
}

/// A small key for tight rows (the status bar, job rows).
pub fn key_sm(label_text: impl Into<String>, on: bool) -> El {
    row()
        .h(22.0)
        .min_w(22.0)
        .px(7.0)
        .center()
        .radius(3.0)
        .fill(if on { LED } else { Color::TRANSPARENT })
        .border(1.0, if on { LED } else { GRILLE })
        .hover(if on { LED } else { SELECTED })
        .cursor(rune::Cursor::Pointer)
        .child(text(label_text.into().to_uppercase()).font(pixel(10.0)).color(if on { CASE } else { SOFT }))
}

/// A latching switch: a square LED and a label.
pub fn toggle(label_text: &str, on: bool) -> El {
    row()
        .h(30.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(4.0)
        .fill(if on { SELECTED } else { Color::TRANSPARENT })
        .border(1.0, if on { GRILLE_ON } else { GRILLE })
        .hover(SELECTED)
        .cursor(rune::Cursor::Pointer)
        .child(el().size(8.0, 8.0).fill(if on { LED } else { GRILLE_ON }))
        .child(text(label_text.to_uppercase()).font(pixel(11.0)).color(if on { LED } else { SILK }))
}

/// A verdict tag: "GOOD", "CLOSE".
pub fn tag(label_text: &str, color: Color) -> El {
    row().px(5.0).py(1.0).radius(2.0).border(1.0, color).child(text(label_text.to_uppercase()).font(pixel(10.0)).color(color))
}

/// A status light: a square dot and a label in its colour.
pub fn lamp(label_text: &str, color: Color) -> El {
    row().gap(6.0).center_y().child(el().size(6.0, 6.0).fill(color)).child(text(label_text.to_uppercase()).font(pixel(11.0)).color(color))
}

/// adeck's kit: opencode's controls and spacing, in the sampler palette and Geist. Always dark:
/// the screens are the product, and they read best on a dark case.
pub fn kit() -> &'static Kit {
    static KITS: [std::sync::OnceLock<Kit>; 2] = [const { std::sync::OnceLock::new() }; 2];
    let srgb = rune::theme::blends_srgb();
    KITS[srgb as usize].get_or_init(|| {
        let mut k = rune_theme_opencode::kit(true).clone();
        let c = &mut k.c;
        c.page = CASE;
        c.bg_base = CASE;
        c.bg_deep = CASE;
        c.layer_01 = SCREEN;
        c.layer_02 = SELECTED;
        c.layer_03 = lin(0.01298, 0.01298, 0.01521);
        c.layer_04 = lin(0.01681, 0.01681, 0.01938);
        c.surface_base = SCREEN;
        c.surface_weak = RAISED;
        c.surface_raised = SELECTED;
        c.surface_raised_base = SELECTED;
        c.surface_float = lin(0.00802, 0.00802, 0.00913);
        c.text_strong = LED;
        c.text_base = LED;
        c.text_muted = SILK;
        c.text_weak = SILK;
        c.text_faint = FAINT;
        c.icon_base = SOFT;
        c.icon_muted = SILK;
        c.icon_faint = FAINT;
        // No blue anywhere: the accent is lit white.
        c.text_accent = LED;
        c.icon_accent = LED;
        c.bg_accent = LED;
        c.knob_on = LED;
        c.border_focus = SILK;
        c.border_weak = LINE;
        c.border_muted = LINE;
        c.border_base = GRILLE;
        c.border_strong = GRILLE_ON;
        k
    })
}

/// The pane docks left or right of the page, or hides to a thin tab on the edge.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dock {
    Left,
    #[default]
    Right,
    Hidden,
}

/// The three dock buttons in the pane's head: dock left, dock right, hide.
pub fn dock_buttons(s: &App, compact: bool) -> El {
    let side = |left: bool, on: bool| {
        let ink = if on { LED } else { SILK };
        let bar = el().w(5.0).h_full().fill(ink);
        let frame = row().size(16.0, 13.0).radius(2.0).border(1.5, ink);
        if left { frame.child(bar) } else { frame.child(rune::spacer()).child(bar) }
    };
    let b =
        |d: Dock, glyph: El, help: &str| icon_btn(glyph, s.pane == d).key(format!("dock-{d:?}").to_lowercase()).help(help.to_owned()).on_press(move |s: &mut App| s.set_pane(d));
    let close = icon_btn(text("×").font(pixel(18.0)).color(SOFT), false).key("dock-hidden").help("Hide the pane · \\").on_press(|s: &mut App| s.toggle_pane());
    if compact {
        // A narrow pane gets one button that moves it to the other side.
        let (to, glyph, help) =
            if s.pane == Dock::Left { (Dock::Right, side(false, false), "Move the pane right · ⌘]") } else { (Dock::Left, side(true, false), "Move the pane left · ⌘[") };
        return row().gap(2.0).child(b(to, glyph, help)).child(el().w(1.0).h(18.0).fill(GRILLE).mx(3.0)).child(close);
    }
    row()
        .gap(2.0)
        .child(b(Dock::Left, side(true, s.pane == Dock::Left), "Dock the pane left · ⌘["))
        .child(b(Dock::Right, side(false, s.pane == Dock::Right), "Dock the pane right · ⌘]"))
        .child(el().w(1.0).h(18.0).fill(GRILLE).mx(4.0))
        .child(close)
}
