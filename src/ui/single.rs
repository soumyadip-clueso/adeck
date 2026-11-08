//! One file: its name in big type with its numbers beside it, the agent's note, and the
//! waveform over the spectrogram filling the page. Metadata and earshot live in the pane; play,
//! score and send live in the player.

use rune::{El, col, row, spacer, text};
use rune_kit::Kit;

use crate::app::{App, Tab};
use crate::ui::{px, widgets};

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let id = t.spec.items.first().cloned().unwrap_or_default();
    let e = s.entry(&id);

    let mut name = row().gap(10.0).center_y().min_w(0.0).child(px::title(e.map_or_else(|| id.clone(), |e| e.item.name.clone())).key("name"));
    if e.is_some_and(|e| e.item.is_ref) {
        name = name.child(widgets::ref_badge(k));
    }
    let sub = e.map(|e| e.item.subtitle()).unwrap_or_default();
    let head = row()
        .key("single-head")
        .w_full()
        .gap(24.0)
        .items_end()
        .flex_wrap()
        .child(col().gap(6.0).min_w(160.0).shrink().child(name).child(text(sub).font(rune::TextStyle::new(13.0, 400.0)).color(px::SILK).truncate()))
        .child(spacer())
        .child(widgets::readouts(e, None));

    let mut page = widgets::page("single", width).gap(16.0).child(head);
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    if let Some(err) = &t.error {
        page = page.child(text(err.clone()).font(rune::TextStyle::new(13.0, 400.0)).color(px::BAD).wrap());
    }
    // The screens share what's left: the waveform a bit more than the spectrogram.
    let (_, win_h, _) = s.win.get();
    let room = (win_h - 40.0 - 68.0 - 40.0 - 110.0 - if t.spec.note.is_some() { 48.0 } else { 0.0 }).max(240.0);
    page.child(
        col()
            .key("screens")
            .w_full()
            .gap(8.0)
            .child(widgets::ruler(s, t, "ruler", k))
            .child(widgets::wave_strip(s, t, e, "wave", width, room * 0.6, k))
            .child(widgets::spec_strip(s, t, e, "spec", width, room * 0.4, k)),
    )
}
