//! A/B/N: every member's waveform stacked on one time axis with one playhead, the one you hear
//! lit. Keys 1-9 switch at the same moment, Enter picks. The loudness match trims each member to
//! the same LUFS. Blind mode hides names and metadata behind X/Y/Z until you pick.

use rune::{Color, El, col, el, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Switch};

use crate::app::{App, Tab};
use crate::item::Item;
use crate::ui::widgets;

const ROW_H: f32 = 64.0;
const LABEL_W: f32 = 210.0;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let hidden = t.is_blind_hidden();
    // Narrow windows put each name above its waveform instead of beside it.
    let narrow = width < 620.0;
    let lead_w = if narrow { width } else { LABEL_W };
    let strip_w = if narrow { width } else { width - LABEL_W - 12.0 };

    let blind = Switch::new(t.spec.blind).label("Blind").disabled(t.spec.revealed).on_toggle(|s: &mut App| s.toggle_blind()).build(k).key("blind");
    let lm = Switch::new(t.spec.loudness_match).label("Match loudness").on_toggle(|s: &mut App| s.toggle_loudness_match()).build(k).key("match");
    let pick_label = format!("Pick {}", t.member_label(t.active));
    let pick = Button::new(pick_label)
        .variant(ButtonVariant::Contrast)
        .control_size(ControlSize::Small)
        .help("The one you're hearing is better · Enter")
        .on_press(|s: &mut App| s.pick())
        .build(k)
        .key("pick");
    let add = Button::new("Add file…")
        .variant(ButtonVariant::Outline)
        .control_size(ControlSize::Small)
        .help("Add a file to compare (or drop files on the window)")
        .on_press_tx(|_s: &mut App, tx: &mut rune::Tx| crate::ui::pick_files(tx, true))
        .build(k)
        .key("add-file");
    let mut head = row().key("compare-head").w_full().gap(16.0).center_y().flex_wrap().child(text(format!("{} files", t.spec.items.len())).font(k.ty.small).color(c.text_muted)).child(spacer()).child(add);
    if t.spec.items.len() > 1 {
        head = head.child(blind).child(lm).child(pick);
    }

    let mut page = crate::ui::widgets::page("compare", width).gap(10.0).child(head);
    if t.spec.items.is_empty() {
        return page.child(crate::ui::drop_zone("Drop audio files here to compare them", k)).child(widgets::note_field(t, k));
    }
    page = page.child(widgets::transport(s, t, k)).child(widgets::note_field(t, k));
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    if let Some(err) = &t.error {
        page = page.child(text(err.clone()).font(k.ty.small).color(c.danger.fg).wrap());
    }
    if let Some(p) = t.picked {
        let msg = if t.spec.blind { format!("You picked {}: {}", t.member_label(p), member_name(s, t, p)) } else { format!("Picked {}", t.member_label(p)) };
        page = page.child(text(msg).key("picked").font(k.ty.body_medium).color(c.success.fg));
    }

    let mut rows = col().key("members").w_full().gap(6.0);
    for (pos, &m) in t.shown().iter().enumerate() {
        let id = &t.spec.items[m];
        let e = s.entry(id);
        let on = m == t.active;
        let label = t.member_label(m);
        let mut name_col = col().gap(2.0).min_w(0.0).shrink();
        let mut first = row().gap(6.0).center_y().child(text(label.clone()).font(k.ty.ui_medium).color(if on { c.icon_accent } else { c.text_base }));
        if !hidden {
            first = first.child(text(member_name(s, t, m)).font(k.ty.small_medium).color(c.text_base).truncate().shrink());
            if e.is_some_and(|e| e.item.is_ref) {
                first = first.child(widgets::ref_badge(k));
            }
        }
        name_col = name_col.child(first);
        let mut detail = Vec::new();
        if !hidden && let Some(e) = e {
            let sub = e.item.subtitle();
            if !sub.is_empty() {
                detail.push(sub);
            }
        }
        if let Some(l) = s.lufs(id) {
            detail.push(format!("{l:.1} LUFS"));
        }
        if let Some(d) = s.trim_db(t, id) {
            detail.push(format!("{d:+.1} dB"));
        }
        if let Some(score) = s.scores.get(id).filter(|_| !hidden) {
            detail.push(format!("score {score}"));
        }
        name_col = name_col.child(text(detail.join(" · ")).font(k.ty.small).color(c.text_muted).truncate());
        let remove = rune_kit::IconButton::new(&crate::ui::icons::CLOSE)
            .variant(rune_kit::IconButtonVariant::Ghost)
            .label(format!("Remove {label}"))
            .on_press(move |s: &mut App| s.remove_member(m))
            .build(k)
            .key(format!("remove-{m}"));
        let key_hint = text(format!("{}", pos + 1)).font(k.ty.small_medium.features("'tnum' 1")).color(c.text_faint).w(14.0);
        let lead = row()
            .key(format!("member-{m}"))
            .w(lead_w)
            .h(if narrow { 44.0 } else { ROW_H * t.strip_h })
            .px(10.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.md)
            .fill(if on { c.icon_accent.alpha(0.14) } else { c.layer_01 })
            .border(1.0, if on { c.icon_accent.alpha(0.7) } else { Color::TRANSPARENT })
            .cursor(rune::Cursor::Pointer)
            .role("button")
            .label(format!("Listen to {label}"))
            .help(format!("Listen to {label} · press {} · double-click sends it to the agent", pos + 1))
            .motion(rune::Spring::SNAPPY)
            .child(key_hint)
            .child(name_col)
            .child(spacer())
            .child(remove)
            .on_press(move |s: &mut App| s.set_active_member(m))
            .on_double_press_tx(move |s: &mut App, tx: &mut rune::Tx| s.send_to_agent(Some(m), tx));
        let wave = widgets::wave_strip(s, t, e, &format!("wave-{m}"), strip_w, ROW_H, k);
        let wave = if on { wave } else { wave.opacity(0.55) };
        let wave = wave.opacity_motion(rune::Spring::SNAPPY);
        rows = rows.child(if narrow {
            col().w_full().gap(4.0).child(lead).child(wave)
        } else {
            row().w_full().gap(12.0).center_y().child(lead).child(col().grow(1.0).min_w(0.0).child(wave))
        });
    }
    let ruler = widgets::ruler(s, t, "ruler", k);
    page = page.child(if narrow { ruler } else { row().w_full().gap(12.0).child(el().w(LABEL_W)).child(col().grow(1.0).min_w(0.0).child(ruler)) }).child(rows);

    // The spectrogram of the one you hear.
    let active_id = t.spec.items.get(t.active).cloned().unwrap_or_default();
    let title = widgets::section_title(&format!("Spectrogram · {}", t.member_label(t.active)), k);
    let spec = widgets::spec_strip(s, t, s.entry(&active_id), "spec", strip_w, 110.0, k);
    page = page.child(if narrow {
        col().w_full().gap(4.0).child(title).child(spec)
    } else {
        row().w_full().gap(12.0).child(col().w(LABEL_W).child(title)).child(col().grow(1.0).min_w(0.0).child(spec))
    });

    // What differs between them, hidden while blind.
    let lower = if hidden {
        col().key("diff").w_full().grow(1.0).min_h(0.0).center().child(text("Blind: names and metadata show after you pick.").font(k.ty.small).color(c.text_muted))
    } else {
        diff_table(s, t, k)
    };
    page.child(widgets::grip(t, k)).child(lower)
}

fn member_name(s: &App, t: &Tab, m: usize) -> String {
    let id = &t.spec.items[m];
    s.entry(id).map_or_else(|| id.rsplit('/').next().unwrap_or(id).to_owned(), |e| e.item.label())
}

/// Only the fields that differ, one column per member.
fn diff_table(s: &App, t: &Tab, k: &Kit) -> El {
    let c = &k.c;
    let entries: Vec<Option<&Item>> = t.spec.items.iter().map(|i| s.entry(i).map(|e| &e.item)).collect();
    let present: Vec<&Item> = entries.iter().flatten().copied().collect();
    let keys = crate::item::differing(&present);
    let mut head = row().w_full().px(8.0).py(4.0).gap(12.0).child(text("Differs").font(k.ty.small_medium).color(c.text_muted).w(180.0));
    for m in 0..t.spec.items.len() {
        head = head.child(text(t.member_label(m)).font(k.ty.small_medium).color(if m == t.active { c.icon_accent } else { c.text_muted }).w(0.0).grow(1.0).min_w(0.0));
    }
    let mut list = col().key("diff-rows").w_full();
    for (i, key) in keys.iter().enumerate() {
        let mut r = row()
            .key(format!("diff-{i}"))
            .w_full()
            .px(8.0)
            .py(4.0)
            .gap(12.0)
            .fill(if i % 2 == 1 { c.layer_01 } else { Color::TRANSPARENT })
            .child(text(key.clone()).font(k.ty.small).color(c.text_muted).w(180.0).truncate().help_when_truncated(key.clone()));
        for (m, e) in entries.iter().enumerate() {
            let v = e.and_then(|e| e.get(key)).unwrap_or("–").to_owned();
            r = r.child(
                text(v).font(k.ty.small.features("'tnum' 1")).color(if m == t.active { c.text_base } else { c.text_muted }).wrap().w(0.0).grow(1.0).min_w(0.0).selectable(true),
            );
        }
        list = list.child(r);
    }
    if keys.is_empty() {
        list = list.child(
            text(if present.len() < t.spec.items.len() { "Loading metadata…" } else { "Their metadata is identical (or there is none)." })
                .font(k.ty.small)
                .color(c.text_faint)
                .px(8.0)
                .py(6.0),
        );
    }
    col().key("diff").w_full().grow(1.0).min_h(0.0).gap(2.0).child(head).child(col().w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
}
