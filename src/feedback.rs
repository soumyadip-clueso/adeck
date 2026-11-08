//! Verdicts: every score, pick, rank and note, one JSON line each, appended and never rewritten.
//!
//! All of them go to `~/.config/adeck/feedback.jsonl`. A verdict on items that all sit in one
//! local run also goes to `feedback.jsonl` in that run's folder, so it travels with the run. A
//! counter and condvar let `wait_for_feedback` block on the MCP thread until a new row lands.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Row {
    /// Unix seconds.
    pub ts: f64,
    pub tab_id: u64,
    /// score | pick | rank | note
    pub kind: String,
    pub items: Vec<String>,
    /// A score (0-3), the picked item, or the ranked ids.
    pub value: Value,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default)]
    pub blind: bool,
    /// For a blind compare: shown label → item id, written at the reveal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reveal_map: Option<Value>,
}

pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// A count of rows written, for waiters.
#[derive(Clone, Default)]
pub struct Signal(Arc<(Mutex<u64>, Condvar)>);

impl Signal {
    pub fn bump(&self) {
        let (m, cv) = &*self.0;
        if let Ok(mut n) = m.lock() {
            *n += 1;
        }
        cv.notify_all();
    }
    pub fn count(&self) -> u64 {
        self.0.0.lock().map_or(0, |n| *n)
    }
    /// Waits until the count passes `seen` or `timeout` runs out. Returns the count.
    pub fn wait_past(&self, seen: u64, timeout: std::time::Duration) -> u64 {
        let (m, cv) = &*self.0;
        let Ok(g) = m.lock() else { return seen };
        cv.wait_timeout_while(g, timeout, |n| *n <= seen).map_or(seen, |(g, _)| *g)
    }
}

pub struct Store {
    pub path: PathBuf,
    pub signal: Signal,
}

impl Store {
    pub fn new(path: PathBuf) -> Store {
        Store { path, signal: Signal::default() }
    }

    /// Appends the row here and, when given, to the run's own file.
    pub fn append(&self, row: &Row, run_dir: Option<PathBuf>) -> Result<(), String> {
        let line = serde_json::to_string(row).map_err(|e| e.to_string())? + "\n";
        let mut targets = vec![self.path.clone()];
        if let Some(d) = run_dir {
            targets.push(d.join("feedback.jsonl"));
        }
        for t in targets {
            if let Some(d) = t.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&t).map_err(|e| format!("Couldn't write {}: {e}", t.display()))?;
            f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        }
        self.signal.bump();
        Ok(())
    }

    pub fn read_all(&self) -> Vec<Row> {
        read(&self.path)
    }
}

pub fn read(path: &std::path::Path) -> Vec<Row> {
    std::fs::read_to_string(path).map(|s| s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// Rows for a tab, or that mention any of these items, newer than `since`.
pub fn select(rows: &[Row], tab: Option<u64>, items: &[String], since: f64) -> Vec<Row> {
    rows.iter()
        .filter(|r| r.ts > since)
        .filter(|r| match tab {
            Some(t) => r.tab_id == t,
            None => items.is_empty() || r.items.iter().any(|i| items.contains(i)),
        })
        .cloned()
        .collect()
}

/// The latest score each item has, for the run browser.
pub fn latest_scores(rows: &[Row]) -> std::collections::HashMap<String, i64> {
    let mut out = std::collections::HashMap::new();
    for r in rows.iter().filter(|r| r.kind == "score") {
        if let (Some(id), Some(v)) = (r.items.first(), r.value.as_i64()) {
            out.insert(id.clone(), v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(tab: u64, kind: &str, items: &[&str], value: Value) -> Row {
        Row { ts: now(), tab_id: tab, kind: kind.into(), items: items.iter().map(|s| (*s).into()).collect(), value, note: String::new(), blind: false, reveal_map: None }
    }

    #[test]
    fn rows_append_to_both_files_and_wake_waiters() {
        let dir = std::env::temp_dir().join(format!("adeck-fb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.join("all.jsonl"));
        let seen = store.signal.count();
        let s2 = store.signal.clone();
        let waiter = std::thread::spawn(move || s2.wait_past(seen, std::time::Duration::from_secs(5)));
        std::thread::sleep(std::time::Duration::from_millis(20));
        store.append(&row(3, "score", &["a"], Value::from(2)), Some(dir.join("run"))).unwrap();
        assert_eq!(waiter.join().unwrap(), seen + 1);
        assert_eq!(store.read_all().len(), 1);
        assert_eq!(read(&dir.join("run/feedback.jsonl")).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn select_by_tab_or_items_and_latest_scores() {
        let rows = vec![row(1, "score", &["a"], Value::from(1)), row(2, "pick", &["a", "b"], Value::from("b")), row(1, "score", &["a"], Value::from(3))];
        assert_eq!(select(&rows, Some(1), &[], 0.0).len(), 2);
        assert_eq!(select(&rows, None, &["b".into()], 0.0).len(), 1);
        assert_eq!(latest_scores(&rows)["a"], 3);
    }
}
