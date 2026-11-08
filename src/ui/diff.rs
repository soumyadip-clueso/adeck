//! Run A against run B: a column per metric, a row per group, each cell the change from A to B
//! coloured by whether it got better, with how many got better and worse on top. Rows that
//! name an item from each side open as a compare.

use rune::{Color, El, col, row, spacer, text};
use rune_kit::Kit;

use crate::app::{App, Tab, TabKind};
use crate::board::{self, Diff, Metric};
use crate::ui::widgets;

const NAME_W: f32 = 170.0;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let Some(d) = &t.spec.diff else {
        return text("This diff is empty.").font(k.ty.small).color(c.text_muted);
    };
    let narrow = width < 640.0;
    let name_w = if narrow { 110.0 } else { NAME_W };
    let cell_w = ((width - name_w - 20.0) / d.metrics.len().max(1) as f32).max(60.0);
    let mut page = col().key("diff").w_full().h_full().min_h(0.0).gap(10.0);
    page = page.child(
        row()
            .w_full()
            .gap(10.0)
            .center_y()
            .flex_wrap()
            .child(text(format!("A  {}", d.a)).font(k.ty.ui_medium).color(c.text_muted))
            .child(text("→").font(k.ty.ui_medium).color(c.text_faint))
            .child(text(format!("B  {}", d.b)).font(k.ty.ui_medium).color(c.text_base))
            .child(spacer())
            .child(text(format!("{} rows", d.rows.len())).font(k.ty.small).color(c.text_muted)),
    );
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    // The tally: per metric, how many got better and worse, and the mean change.
    let mut tally = row().key("diff-tally").w_full().gap(0.0).child(text("Overall").font(k.ty.small_medium).color(c.text_muted).w(name_w));
    for (m, (up, down, mean)) in d.metrics.iter().zip(d.tally()) {
        let mut cell = col().w(cell_w).gap(1.0).child(text(m.label().to_owned()).font(k.ty.small_medium).color(c.text_base).truncate());
        if m.better.is_some() {
            cell = cell.child(text(format!("{up} better · {down} worse")).font(k.ty.small.features("'tnum' 1")).color(if up > down { c.success.fg } else if down > up { c.danger.fg } else { c.text_muted }).truncate());
        }
        if let Some(x) = mean {
            cell = cell.child(text(format!("mean {}", signed(x))).font(k.ty.small.features("'tnum' 1")).color(c.text_muted).truncate());
        }
        if let Some(h) = &m.help {
            cell = cell.help(h.clone());
        }
        tally = tally.child(cell);
    }
    page = page.child(tally.pb(6.0));
    let mut list = col().key("diff-rows").w_full();
    for (i, r) in d.sorted().into_iter().enumerate() {
        let mut line = row()
            .key(format!("drow-{}", r.key))
            .w_full()
            .py(5.0)
            .gap(0.0)
            .center_y()
            .fill(if i % 2 == 1 { c.layer_01 } else { Color::TRANSPARENT })
            .child(text(r.title.clone().unwrap_or_else(|| r.key.clone())).font(k.ty.small).color(c.text_base).w(name_w).truncate().px(4.0));
        for m in &d.metrics {
            line = line.child(cell(d, m, r, cell_w, narrow, k));
        }
        if let Some(items) = &r.items {
            let ids = vec![items.a.clone(), items.b.clone()];
            let label = format!("{} · {} vs {}", r.title.clone().unwrap_or_else(|| r.key.clone()), d.a, d.b);
            line = line.cursor(rune::Cursor::Pointer).role("button").help("Open A and B as a compare").on_press(move |s: &mut App| {
                s.open(TabKind::Compare, ids.clone(), Some(label.clone()), false, true, None, None);
            });
        }
        list = list.child(line);
    }
    if d.rows.is_empty() {
        list = list.child(text("No rows yet. The agent adds them as results land.").font(k.ty.small).color(c.text_muted).py(12.0));
    }
    let _ = s;
    page.child(col().w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
}

fn signed(x: f64) -> String {
    let v = if x.abs() >= 100.0 {
        format!("{:.0}", x.abs())
    } else if x.abs() >= 10.0 {
        format!("{:.1}", x.abs())
    } else {
        format!("{:.3}", x.abs()).trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    format!("{}{}", if x < 0.0 { "−" } else { "+" }, if v.is_empty() { "0".into() } else { v })
}

fn plain(x: f64) -> String {
    if x.abs() >= 100.0 { format!("{x:.0}") } else if x.abs() >= 10.0 { format!("{x:.1}") } else { format!("{x:.2}") }
}

fn cell(_d: &Diff, m: &Metric, r: &board::DiffRow, w: f32, narrow: bool, k: &Kit) -> El {
    let c = &k.c;
    let (a, b) = (board::num(&r.a, &m.key), board::num(&r.b, &m.key));
    let Some(delta) = board::delta(r, &m.key) else {
        let only = b.or(a).map_or("–".to_owned(), plain);
        return text(only).font(k.ty.small.features("'tnum' 1")).color(c.text_faint).w(w);
    };
    let tone = match m.verdict(delta) {
        1 => c.success.fg,
        -1 => c.danger.fg,
        _ => c.text_base,
    };
    let mut cell = col().w(w).gap(0.0).child(text(signed(delta)).font(k.ty.small_medium.features("'tnum' 1")).color(tone));
    if !narrow && let (Some(a), Some(b)) = (a, b) {
        cell = cell.child(text(format!("{} → {}", plain(a), plain(b))).font(k.ty.small.features("'tnum' 1")).color(c.text_faint).truncate());
    }
    cell
}
