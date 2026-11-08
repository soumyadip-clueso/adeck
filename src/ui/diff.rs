//! Run A against run B: a column per metric, a row per group, each cell the change from A to B
//! coloured by whether it got better, with how many got better and worse on top. Rows that
//! name an item from each side open as a compare.

use rune::{El, col, row, text};
use rune_kit::Kit;

use crate::app::{App, Tab, TabKind};
use crate::board::{self, Diff, Metric};
use crate::ui::{px, widgets};

const NAME_W: f32 = 170.0;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let Some(d) = &t.spec.diff else {
        return text("This diff is empty.").font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT);
    };
    let narrow = width < 640.0;
    let name_w = if narrow { 110.0 } else { NAME_W };
    let cell_w = ((width - name_w - 20.0) / d.metrics.len().max(1) as f32).max(60.0);
    let side = |l: &str, name: &str, on: bool| {
        row()
            .gap(8.0)
            .center_y()
            .child(text(l.to_owned()).font(px::pixel(13.0)).color(px::CASE).px(5.0).py(1.0).fill(if on { px::LED } else { px::SILK }).radius(2.0))
            .child(text(name.to_owned()).font(rune::TextStyle::new(13.0, 400.0).mono()).color(if on { px::LED } else { px::SILK }))
    };
    let title = col().gap(8.0).min_w(0.0).shrink().child(px::title(t.spec.label.clone().unwrap_or_else(|| format!("{} vs {}", d.a, d.b)))).child(
        row()
            .gap(14.0)
            .center_y()
            .flex_wrap()
            .child(side("A", &d.a, false))
            .child(text("→").font(px::pixel(13.0)).color(px::OFF))
            .child(side("B", &d.b, true))
            .child(px::lamp(&format!("{} rows", d.rows.len()), px::SILK)),
    );
    let mut page = col().key("diff").w_full().h_full().min_h(0.0).gap(14.0).child(row().key("diff-head").w_full().child(title));
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    // The tally: per metric, how many got better and worse, and the mean change.
    let mut tally = row()
        .key("diff-tally")
        .w_full()
        .px(4.0)
        .py(12.0)
        .radius(6.0)
        .fill(px::SCREEN)
        .border(1.0, px::LINE)
        .child(text("OVERALL").font(px::pixel(11.0)).color(px::FAINT).w(name_w - 4.0).pl(8.0));
    for (m, (up, down, mean)) in d.metrics.iter().zip(d.tally()) {
        let mut cell = col().w(cell_w).gap(4.0).child(text(m.label().to_uppercase()).font(px::pixel(11.0)).color(px::SOFT).truncate());
        if m.better.is_some() {
            let ink = if up > down {
                px::GOOD
            } else if down > up {
                px::BAD
            } else {
                px::SILK
            };
            cell = cell.child(text(format!("{up}↑ {down}↓")).font(px::pixel(20.0)).color(ink));
        }
        if let Some(x) = mean {
            cell = cell.child(text(format!("MEAN {}", signed(x))).font(px::pixel(10.0)).color(px::FAINT).truncate());
        }
        if let Some(h) = &m.help {
            cell = cell.help(h.clone());
        }
        tally = tally.child(cell);
    }
    page = page.child(tally);
    let mut list = col().key("diff-rows").w_full();
    for r in d.sorted() {
        let mut line = row()
            .key(format!("drow-{}", r.key))
            .w_full()
            .py(8.0)
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], px::SELECTED)
            .child(text(r.title.clone().unwrap_or_else(|| r.key.clone())).font(rune::TextStyle::new(13.0, 400.0).mono()).color(px::SOFT).w(name_w).truncate().px(8.0));
        for m in &d.metrics {
            line = line.child(cell(d, m, r, cell_w, narrow, k));
        }
        if let Some(items) = &r.items {
            let ids = vec![items.a.clone(), items.b.clone()];
            let label = format!("{} · {} vs {}", r.title.clone().unwrap_or_else(|| r.key.clone()), d.a, d.b);
            line = line.cursor(rune::Cursor::Pointer).hover(px::RAISED).role("button").help("Open A and B as a compare").on_press(move |s: &mut App| {
                s.open(TabKind::Compare, ids.clone(), Some(label.clone()), false, true, None, None);
            });
        }
        list = list.child(line);
    }
    if d.rows.is_empty() {
        list = list.child(text("No rows yet. The agent adds them as results land.").font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).py(12.0));
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
    if x.abs() >= 100.0 {
        format!("{x:.0}")
    } else if x.abs() >= 10.0 {
        format!("{x:.1}")
    } else {
        format!("{x:.2}")
    }
}

fn cell(_d: &Diff, m: &Metric, r: &board::DiffRow, w: f32, narrow: bool, _k: &Kit) -> El {
    let (a, b) = (board::num(&r.a, &m.key), board::num(&r.b, &m.key));
    let Some(delta) = board::delta(r, &m.key) else {
        let only = b.or(a).map_or("–".to_owned(), plain);
        return text(only).font(px::pixel(12.0)).color(px::OFF).w(w);
    };
    let tone = match m.verdict(delta) {
        1 => px::GOOD,
        -1 => px::BAD,
        _ => px::SOFT,
    };
    let mut cell = col().w(w).gap(2.0).child(text(signed(delta)).font(px::pixel(13.0)).color(tone));
    if !narrow && let (Some(a), Some(b)) = (a, b) {
        cell = cell.child(text(format!("{} → {}", plain(a), plain(b))).font(px::pixel(10.0)).color(px::FAINT).truncate());
    }
    cell
}
