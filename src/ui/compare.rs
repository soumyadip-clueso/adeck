//! A/B/N: every member's waveform stacked on one time axis with one playhead, the one you hear
//! lit. Keys 1-9 switch at the same moment, Enter picks. The loudness match trims each member to
//! the same LUFS. Blind mode hides names and metadata behind X/Y/Z until you pick.

use rune::{Color, El, col, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Switch};

use crate::app::{App, Tab};
use crate::item::Item;
use crate::ui::widgets;

const ROW_H: f32 = 64.0;
const LABEL_W: f32 = 210.0;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let total = widgets::tab_secs(s, t);
    let hidden = t.is_blind_hidden();
    let strip_w = width - LABEL_W - 12.0;

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
    let head = row().key("compare-head").w_full().gap(16.0).center_y().child(widgets::transport(s, t, k)).child(spacer()).child(blind).child(lm).child(pick);

    let mut page = col().key("compare").w_full().h_full().min_h(0.0).gap(10.0).child(head);
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
        let key_hint = text(format!("{}", pos + 1)).font(k.ty.small_medium.features("'tnum' 1")).color(c.text_faint).w(14.0);
        let lead = row()
            .key(format!("member-{m}"))
            .w(LABEL_W)
            .h(ROW_H)
            .px(10.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.md)
            .fill(if on { c.icon_accent.alpha(0.14) } else { c.layer_01 })
            .border(1.0, if on { c.icon_accent.alpha(0.7) } else { Color::TRANSPARENT })
            .cursor(rune::Cursor::Pointer)
            .role("button")
            .label(format!("Listen to {label}"))
            .help(format!("Listen to {label} · press {}", pos + 1))
            .child(key_hint)
            .child(name_col)
            .on_press(move |s: &mut App| s.set_active_member(m));
        let wave = widgets::wave_strip(s, t, e, &format!("wave-{m}"), strip_w, ROW_H, total, k);
        let wave = if on { wave } else { wave.opacity(0.55) };
        rows = rows.child(row().w_full().gap(12.0).center_y().child(lead).child(col().grow(1.0).min_w(0.0).child(wave)));
    }
    page = page.child(rows);

    // The spectrogram of the one you hear.
    let active_id = t.spec.items.get(t.active).cloned().unwrap_or_default();
    page = page.child(
        row()
            .w_full()
            .gap(12.0)
            .child(col().w(LABEL_W).child(widgets::section_title(&format!("Spectrogram · {}", t.member_label(t.active)), k)))
            .child(col().grow(1.0).min_w(0.0).child(widgets::spec_strip(s, t, s.entry(&active_id), "spec", strip_w, 110.0, total, k))),
    );

    // What differs between them, hidden while blind.
    let lower = if hidden {
        col().key("diff").w_full().grow(1.0).min_h(0.0).center().child(text("Blind: names and metadata show after you pick.").font(k.ty.small).color(c.text_muted))
    } else {
        diff_table(s, t, k)
    };
    page.child(lower).child(widgets::note_field(t, k))
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
