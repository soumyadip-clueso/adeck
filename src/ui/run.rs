//! A whole run as a table: every item with what it is, its numbers and your latest score.
//! Filter by any text, show only what you haven't scored, Return opens a row, and a selection of
//! several opens as a compare.

use rune::{El, Tx, col, row, text};
use rune_kit::{Cell, Kit, Table, TableColumn, TextInput};

use crate::app::{App, Entry, Tab};
use crate::ui::px;

fn table_state(s: &App) -> &rune_kit::TableState {
    &s.current().expect("a run tab").table
}
fn table_state_mut(s: &mut App) -> &mut rune_kit::TableState {
    &mut s.current_mut().expect("a run tab").table
}
fn rows_len(s: &App) -> usize {
    s.current().map_or(0, |t| t.rows.len())
}
fn row_id(s: &App, i: usize) -> std::borrow::Cow<'_, str> {
    s.current().and_then(|t| t.rows.get(i)).map_or("".into(), |r| r.as_str().into())
}

/// A column's text for an item, shared with sorting.
pub fn cell_value(s: &App, e: &Entry, col: &str) -> String {
    let it = &e.item;
    match col {
        "name" => it.name.clone(),
        "what" => it.pick(&["mood", "type", "kind"]).unwrap_or("").to_owned(),
        "model" => it.pick(&["model"]).unwrap_or("").to_owned(),
        "secs" => e
            .analysis
            .as_ref()
            .map(|a| format!("{:.1}", a.secs))
            .or_else(|| it.pick(&["duration", "duration_s", "seconds", "secs", "length"]).map(str::to_owned))
            .unwrap_or_default(),
        "bpm" => it.pick(&["bpm", "tempo"]).map(str::to_owned).or_else(|| e.analysis.as_ref().and_then(|a| a.bpm).map(|b| format!("{b:.0}"))).unwrap_or_default(),
        "lufs" => it.pick(&["lufs", "loudness", "lufs_i"]).map(str::to_owned).or_else(|| e.analysis.as_ref().and_then(|a| a.lufs).map(|l| format!("{l:.1}"))).unwrap_or_default(),
        "flags" => it.pick(&["flags"]).unwrap_or("").to_owned(),
        "score" => s.scores.get(&it.id).map(|v| v.to_string()).unwrap_or_default(),
        _ => String::new(),
    }
}

pub fn build(s: &App, t: &Tab, k: &Kit) -> El {
    let filter = TextInput::new(&t.filter)
        .placeholder("Filter")
        .leading_icon(&crate::ui::icons::SEARCH)
        .width(Some(200.0))
        .clear_button()
        .on_edit(|s: &mut App| &mut s.current_mut().expect("a run tab").filter)
        .on_change(|s: &mut App, _| {
            if let Some(a) = s.active {
                s.refilter(a);
            }
        })
        .build(k)
        .key("filter");
    let unscored = px::toggle("Unscored", t.unscored_only)
        .help("Show only what you haven't scored · filter matches the name or any metadata")
        .on_press(|s: &mut App| {
            if let Some(t) = s.current_mut() {
                t.unscored_only ^= true;
            }
            if let Some(a) = s.active {
                s.refilter(a);
            }
        })
        .key("unscored");
    let n = t.table.selected_len();
    let mut open = px::key(
        match n {
            0 | 1 => "OPEN".to_owned(),
            n => format!("COMPARE {}", n.min(9)),
        },
        n > 0,
    )
    .key("open-selection")
    .help("Open the selected row, or compare a selection (⌘-click to pick several) · Return");
    if n > 0 {
        open = open.on_press(|s: &mut App| s.open_selection());
    } else {
        open = open.opacity(0.4);
    }
    let mut count = row().gap(14.0).center_y().child(px::lamp(&format!("{} items", t.rows.len()), px::SILK));
    let scored = t.rows.iter().filter(|id| s.scores.contains_key(id.as_str())).count();
    if scored > 0 {
        count = count.child(px::lamp(&format!("{scored} scored"), px::GOOD));
    }
    if t.loading {
        count = count.child(row().gap(6.0).center_y().child(rune_kit::Spinner::new().size(12.0).build(k)).child(px::label("Listing…")));
    }
    let tools = row().key("run-tools").gap(8.0).center_y().child(filter).child(unscored).child(open);
    let head = px::header("run-head", t.spec.run.clone().unwrap_or_else(|| "Run".into()), count, tools);

    let table = Table::new(table_state, table_state_mut, rows_len, row_id)
        .column(TableColumn::new("name", "Name").flex(2.0).min(180.0).sortable())
        .column(TableColumn::new("what", "Mood / type").flex(1.0).min(110.0).sortable())
        .column(TableColumn::new("model", "Model").flex(1.0).min(90.0).sortable())
        .column(TableColumn::new("secs", "Secs").width(64.0).align(rune::Align::End).sortable())
        .column(TableColumn::new("bpm", "BPM").width(60.0).align(rune::Align::End).sortable())
        .column(TableColumn::new("lufs", "LUFS").width(64.0).align(rune::Align::End).sortable())
        .column(TableColumn::new("flags", "Flags").flex(1.0).min(100.0).sortable())
        .column(TableColumn::new("score", "Score").width(60.0).align(rune::Align::End).sortable())
        .cell(|s: &App, i, col| {
            let Some(e) = s.current().and_then(|t| t.rows.get(i)).and_then(|id| s.entry(id)) else {
                return Cell::text("");
            };
            let v = cell_value(s, e, col);

            match col {
                "name" if e.item.is_ref => Cell::text(format!("{v}  [REF]")),
                "secs" | "bpm" | "lufs" | "score" => Cell::text(v).mono(),
                "flags" if !v.is_empty() => Cell::text(v).color(px::CLOSE),
                _ => Cell::text(v),
            }
        })
        .on_sort(|s: &mut App, col: &str, dir, _tx: &mut Tx| {
            if let Some(t) = s.current_mut() {
                t.table.set_sort(col, dir);
            }
            if let Some(a) = s.active {
                s.refilter(a);
            }
        })
        .on_open(|s: &mut App, _id: &str, _tx: &mut Tx| s.open_selection())
        .zebra(true)
        .name("Items")
        .build(s, k)
        .h_full()
        .min_h(0.0);

    let mut page = col().key("run").w_full().h_full().min_h(0.0).gap(14.0).child(head);
    if let Some(err) = &t.error {
        page = page.child(text(err.clone()).font(rune::TextStyle::new(13.0, 400.0)).color(px::BAD).wrap());
    }
    page.child(col().key("run-table").w_full().grow(1.0).min_h(0.0).h(0.0).child(table))
}
