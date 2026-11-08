//! One file: its name and what made it, the transport and score, the waveform with its beat
//! grid over the spectrogram, every metadata field, the native numbers and earshot's.

use rune::{El, col, row, spacer, text};
use rune_kit::Kit;

use crate::app::{App, Tab};
use crate::ui::widgets;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let id = t.spec.items.first().cloned().unwrap_or_default();
    let e = s.entry(&id);

    let mut name =
        row().gap(8.0).center_y().min_w(0.0).child(text(e.map_or_else(|| id.clone(), |e| e.item.name.clone())).key("name").font(k.ty.ui_medium).color(c.text_base).truncate());
    if e.is_some_and(|e| e.item.is_ref) {
        name = name.child(widgets::ref_badge(k));
    }
    let sub = e.map(|e| e.item.subtitle()).unwrap_or_default();
    let head = row()
        .key("single-head")
        .w_full()
        .gap(16.0)
        .center_y()
        .child(col().gap(2.0).min_w(0.0).shrink().child(name).child(text(sub).font(k.ty.small).color(c.text_muted).truncate()))
        .child(spacer())
        .child(
            rune_kit::Button::new("Compare with…")
                .variant(rune_kit::ButtonVariant::Outline)
                .control_size(rune_kit::ControlSize::Small)
                .help("Add a file beside this one (or drop files on the window)")
                .on_press_tx(|_s: &mut App, tx: &mut rune::Tx| crate::ui::pick_files(tx, true))
                .build(k)
                .key("add-compare"),
        )
        .child(widgets::score_buttons(s, t, k));

    let mut page = crate::ui::widgets::page("single", width).gap(10.0).child(head).child(widgets::transport(s, t, k)).child(widgets::note_field(t, k));
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    if let Some(err) = &t.error {
        page = page.child(text(err.clone()).font(k.ty.small).color(c.danger.fg).wrap());
    }
    page = page
        .child(widgets::ruler(s, t, "ruler", k))
        .child(widgets::wave_strip(s, t, e, "wave", width, 120.0, k))
        .child(widgets::spec_strip(s, t, e, "spec", width, 150.0, k))
        .child(widgets::grip(t, k))
        .child(widgets::stats(e, None, k));

    let prompt = e.and_then(|e| e.item.pick(&["prompt", "caption", "text"]).map(str::to_owned));
    let meta: Vec<(String, String)> = e.map(|e| e.item.meta.clone()).unwrap_or_default();
    let src = e.and_then(|e| e.item.meta_src.as_ref()).and_then(|p| p.file_name()).map(|n| format!("from {}", n.to_string_lossy())).unwrap_or_default();
    let mut left = col()
        .key("meta")
        .w_frac(0.58)
        .h_full()
        .min_h(0.0)
        .gap(6.0)
        .child(row().gap(8.0).child(widgets::section_title("Metadata", k)).child(text(src).font(k.ty.small).color(c.text_faint)));
    if let Some(p) = prompt {
        left = left.child(text(p).key("prompt").font(k.ty.body).color(c.text_base).wrap().selectable(true));
    }
    left = left.child(widgets::kv_table("meta-table", &meta, &|_| false, k));
    let right = match e {
        Some(e) => col().key("side").grow(1.0).min_w(0.0).h_full().min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(widgets::earshot_panel(s, e, k)),
        None => col(),
    };
    page.child(row().key("lower").w_full().grow(1.0).min_h(0.0).gap(16.0).child(left).child(right))
}
