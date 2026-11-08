//! Renders: the picture (muted, following the mixer's clock, so several renders switch like a
//! compare), the cut strip from earshot's critique, its scorecard and fixes, and the sheet.

use rune::{Color, El, SurfaceFit, col, el, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};
use serde_json::Value;

use crate::app::{App, Tab};
use crate::derive::Earshot;
use crate::ui::widgets;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let total = widgets::tab_secs(s, t);
    let id = t.spec.items.get(t.active).cloned().unwrap_or_default();
    let e = s.entry(&id);

    let mut members = row().key("render-members").gap(6.0);
    for (pos, &m) in t.shown().iter().enumerate() {
        let on = m == t.active;
        let name = s.entry(&t.spec.items[m]).map_or_else(|| "…".to_owned(), |e| e.item.name.clone());
        members = members.child(
            Button::new(format!("{} · {name}", pos + 1))
                .variant(if on { ButtonVariant::Contrast } else { ButtonVariant::Outline })
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.set_active_member(m))
                .build(k)
                .key(format!("render-member-{m}")),
        );
    }
    let head = row().key("render-head").w_full().gap(16.0).center_y().child(widgets::transport(s, t, k)).child(spacer()).child(members);

    let video_w = (width * 0.55).max(320.0);
    let video_h = video_w * 9.0 / 16.0;
    let mut stage = el().key("video").w(video_w).h(video_h).radius(k.radius.md).fill(Color::BLACK).clip_rounded(k.radius.md);
    if let Some(v) = t.videos.get(t.active) {
        stage = stage.video(v, SurfaceFit::Fit);
    }

    let mut card = col().key("critique").grow(1.0).min_w(0.0).h(video_h).gap(8.0).scroll(rune::ScrollSpec { stick_bottom: false });
    match e.and_then(|e| e.earshot.get("critique")) {
        Some(v) => card = card.child(critique(v, k)),
        None => {
            let busy = e.is_some_and(|e| e.earshot_busy.contains("critique"));
            let id2 = id.clone();
            card = card.child(widgets::section_title("Scorecard", k)).child(
                Button::new(if busy { "Running earshot critique…" } else { "Run earshot critique" })
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .disabled(busy || e.is_none())
                    .on_press(move |s: &mut App| s.run_earshot(&id2, Earshot::Critique))
                    .build(k)
                    .key("run-critique"),
            );
        }
    }

    let mut page = col().key("render").w_full().h_full().min_h(0.0).gap(10.0).child(head);
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    if let Some(err) = &t.error {
        page = page.child(text(err.clone()).font(k.ty.small).color(c.danger.fg).wrap());
    }
    page = page.child(row().w_full().gap(16.0).child(stage).child(card));
    page = page.child(cut_strip(e.and_then(|e| e.earshot.get("critique")), total, k));
    page = page.child(widgets::wave_strip(s, t, e, "wave", width, 64.0, total, k));

    let sheet = match e.and_then(|e| e.sheet.as_ref()) {
        Some(px) => {
            let (w, h) = px.size();
            let sh = if w > 0 { width * h as f32 / w as f32 } else { 200.0 };
            el().key("sheet").w_full().h(sh).pixels(px)
        }
        None => {
            let busy = e.is_some_and(|e| e.earshot_busy.contains("sheet"));
            let id3 = id.clone();
            row().key("sheet").gap(8.0).child(
                Button::new(if busy { "Drawing the sheet…" } else { "Draw earshot sheet" })
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .disabled(busy || e.is_none())
                    .on_press(move |s: &mut App| s.run_sheet(&id3))
                    .build(k)
                    .key("run-sheet"),
            )
        }
    };
    page.child(col().w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(sheet)).child(widgets::note_field(t, k))
}

/// The verdict, score, the metrics outside their range and the first fixes.
fn critique(v: &Value, k: &Kit) -> El {
    let c = &k.c;
    let verdict = v.get("verdict").and_then(Value::as_str).unwrap_or("–").to_owned();
    let score = v.get("score").and_then(Value::as_f64).map_or("–".into(), |s| format!("{s:.0}"));
    let good = matches!(verdict.as_str(), "launch-grade" | "close");
    let mut out = col().gap(8.0).child(widgets::section_title("Scorecard", k)).child(
        row()
            .gap(18.0)
            .child(widgets::stat("Verdict", verdict, k).key("verdict"))
            .child(widgets::stat("Score", score, k))
            .child(widgets::stat("Critical", v.get("critical_issues").map_or("0".into(), |x| x.to_string()), k))
            .child(widgets::stat("Major", v.get("major_issues").map_or("0".into(), |x| x.to_string()), k)),
    );
    let _ = good;
    if let Some(ms) = v.get("metrics").and_then(Value::as_array) {
        let mut list = col().gap(2.0);
        for m in ms.iter().filter(|m| m.get("status").and_then(Value::as_str).is_some_and(|s| s != "ok" && s != "in range")).take(10) {
            let name = m.get("metric").and_then(Value::as_str).unwrap_or("?").to_owned();
            let val = m.get("value").map_or("–".into(), fmt);
            let range = match (m.get("min"), m.get("max")) {
                (Some(a), Some(b)) => format!("{}–{}", fmt(a), fmt(b)),
                _ => String::new(),
            };
            list = list.child(row().gap(8.0).child(text(name).font(k.ty.small).color(c.text_muted).w(150.0).truncate()).child(text(val).font(k.ty.small.features("'tnum' 1")).color(c.warning.fg).w(70.0)).child(text(range).font(k.ty.small).color(c.text_faint)));
        }
        out = out.child(list);
    }
    if let Some(acts) = v.get("actions").and_then(Value::as_array) {
        let mut list = col().gap(3.0).child(widgets::section_title("Fixes", k));
        for a in acts.iter().take(6) {
            let line = a.get("action").and_then(Value::as_str).map_or_else(|| a.to_string(), str::to_owned);
            list = list.child(text(format!("• {line}")).font(k.ty.small).color(c.text_base).wrap());
        }
        out = out.child(list);
    }
    out
}

fn fmt(v: &Value) -> String {
    match v.as_f64() {
        Some(f) if f.abs() >= 100.0 => format!("{f:.0}"),
        Some(f) => format!("{f:.2}"),
        None => v.as_str().map_or_else(|| v.to_string(), str::to_owned),
    }
}

/// Every cut as a tick: on a beat (green), covered by a hit (amber), or missed (red), as the
/// eval pages draw it. Read from the critique's per-cut list when it has one.
fn cut_strip(critique: Option<&Value>, total: f64, k: &Kit) -> El {
    let c = &k.c;
    let mut strip = el().key("cuts").w_full().h(22.0).radius(4.0).fill(c.layer_01);
    let Some(cuts) = critique.and_then(cuts_of) else {
        return strip.child(text("Cuts show here once earshot critique has run on a video.").font(k.ty.small).color(c.text_faint).px(8.0).py(3.0));
    };
    if total <= 0.0 {
        return strip;
    }
    let mut shape = rune::Shape::new();
    for (t, kind) in &cuts {
        let x = (*t / total).clamp(0.0, 1.0) as f32 * 1000.0;
        let ink = match kind {
            Cut::Beat => rune::oklch(0.75, 0.17, 145.0),
            Cut::Hit => rune::oklch(0.8, 0.15, 80.0),
            Cut::Miss => rune::oklch(0.65, 0.2, 25.0),
        };
        shape = shape.line([x, 2.0], [x, 20.0], 4.0, rune::LineCap::Round, ink);
    }
    strip = strip.child(rune::canvas(shape.view_box(1000.0, 22.0)).absolute(0.0, 0.0).w_full().h_full().inert());
    let (b, h, m) = cuts.iter().fold((0, 0, 0), |(b, h, m), (_, k)| match k {
        Cut::Beat => (b + 1, h, m),
        Cut::Hit => (b, h + 1, m),
        Cut::Miss => (b, h, m + 1),
    });
    strip.help(format!("{} cuts: {b} on a beat, {h} with a hit, {m} missed", cuts.len()))
}

#[derive(Clone, Copy)]
enum Cut {
    Beat,
    Hit,
    Miss,
}

/// The per-cut list in a critique, whatever its field is called.
fn cuts_of(v: &Value) -> Option<Vec<(f64, Cut)>> {
    let list = ["cuts", "cut_sync", "cut_list"].iter().find_map(|k| v.get(*k).and_then(Value::as_array)).or_else(|| v.get("sync").and_then(|s| s.get("cuts")).and_then(Value::as_array))?;
    let out: Vec<(f64, Cut)> = list
        .iter()
        .filter_map(|c| {
            let t = c.get("t").or_else(|| c.get("time")).or_else(|| c.get("time_s")).and_then(Value::as_f64)?;
            let near = |k: &str| c.get(k).and_then(Value::as_f64).is_some_and(|ms| ms.abs() <= 50.0);
            let kind = if near("beat_ms") || near("b") || c.get("on_beat").and_then(Value::as_bool) == Some(true) {
                Cut::Beat
            } else if near("hit_ms") || near("o") || c.get("hit").and_then(Value::as_bool) == Some(true) || c.get("has_sound").and_then(Value::as_bool) == Some(true) {
                Cut::Hit
            } else {
                Cut::Miss
            };
            Some((t, kind))
        })
        .collect();
    (!out.is_empty()).then_some(out)
}
