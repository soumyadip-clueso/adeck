//! Boards and diffs: pages the agent fills over MCP and keeps adding to as results land.
//!
//! A board is groups (one per category, say), each a row of members: the picks, a "before",
//! the reference files, with the agent's numbers and a verdict chip on each, plus a ranked
//! "more" list kept folded. A diff is run A against run B: one row per group, one column per
//! metric, the change coloured by which way is better. Both are plain data, saved with the
//! session, and replaced group by group (or row by row) by key.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Chip {
    pub text: String,
    /// good | close | bad | anything else (shown neutral).
    #[serde(default)]
    pub tone: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Member {
    /// An item id: a path, `run:…`, `ref:…` or `s3://…`.
    pub item: String,
    /// pick | before | ref | anything else.
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub label: Option<String>,
    /// The agent's numbers, in the order given: {"vibe": 1.05, "shape": 0.5}.
    #[serde(default)]
    pub numbers: Map<String, Value>,
    #[serde(default)]
    pub chip: Option<Chip>,
    /// What made it: prompt, model, seed, chain… Shown under Details.
    #[serde(default)]
    pub meta: Map<String, Value>,
    /// Measurements in earshot `measure` shape (bands_db, correlation, decay_ms…). Without them
    /// adeck runs earshot itself.
    #[serde(default)]
    pub measure: Option<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Group {
    pub key: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    /// Bigger first.
    #[serde(default)]
    pub sort: Option<f64>,
    #[serde(default)]
    pub chip: Option<Chip>,
    /// What the members should measure like, in `measure` shape; `target_` prefixes are dropped.
    #[serde(default)]
    pub target: Option<Value>,
    #[serde(default)]
    pub members: Vec<Member>,
    /// The rest, best first, folded until asked for.
    #[serde(default)]
    pub more: Vec<Member>,
    #[serde(default)]
    pub note: Option<String>,
}

impl Group {
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.key)
    }
    /// Every member that plays, in mixer order: the members, then the rest.
    pub fn playable(&self) -> Vec<&Member> {
        self.members.iter().chain(self.more.iter()).collect()
    }
    /// The target with `target_` dropped from its keys, so a profile's targets drop straight in.
    pub fn target(&self) -> Option<Map<String, Value>> {
        let o = self.target.as_ref()?.as_object()?;
        Some(o.iter().map(|(k, v)| (k.strip_prefix("target_").unwrap_or(k).to_owned(), v.clone())).collect())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Metric {
    pub key: String,
    #[serde(default)]
    pub label: Option<String>,
    /// higher | lower: which way is better. Unset: neither is coloured.
    #[serde(default)]
    pub better: Option<String>,
    #[serde(default)]
    pub help: Option<String>,
}

impl Metric {
    pub fn label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.key)
    }
    /// +1 when this change is an improvement, -1 when worse, 0 when neither or unknown.
    pub fn verdict(&self, delta: f64) -> i8 {
        let d = if delta.abs() < 1e-9 { 0.0 } else { delta };
        match self.better.as_deref() {
            Some("higher") => d.partial_cmp(&0.0).map_or(0, |o| o as i8),
            Some("lower") => (0.0f64).partial_cmp(&d).map_or(0, |o| o as i8),
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Board {
    #[serde(default)]
    pub groups: Vec<Group>,
    /// Labels and hints for the numbers; numbers without one show by their key.
    #[serde(default)]
    pub metrics: Vec<Metric>,
    /// Decisions offered on each file: keep, drop… Empty: none.
    #[serde(default)]
    pub choices: Vec<String>,
    /// Groups a file can be moved to, recorded as `move:<group>`.
    #[serde(default)]
    pub move_to: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DiffItems {
    pub a: String,
    pub b: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DiffRow {
    pub key: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub sort: Option<f64>,
    #[serde(default)]
    pub a: Map<String, Value>,
    #[serde(default)]
    pub b: Map<String, Value>,
    /// One item from each side: clicking the row opens them as a compare.
    #[serde(default)]
    pub items: Option<DiffItems>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Diff {
    pub a: String,
    pub b: String,
    #[serde(default)]
    pub metrics: Vec<Metric>,
    #[serde(default)]
    pub rows: Vec<DiffRow>,
}

impl Diff {
    /// Each metric: (improved, worse, mean change) over the rows that have both sides.
    pub fn tally(&self) -> Vec<(usize, usize, Option<f64>)> {
        self.metrics
            .iter()
            .map(|m| {
                let ds: Vec<f64> = self.rows.iter().filter_map(|r| delta(r, &m.key)).collect();
                let up = ds.iter().filter(|d| m.verdict(**d) > 0).count();
                let down = ds.iter().filter(|d| m.verdict(**d) < 0).count();
                let mean = (!ds.is_empty()).then(|| ds.iter().sum::<f64>() / ds.len() as f64);
                (up, down, mean)
            })
            .collect()
    }
    pub fn sorted(&self) -> Vec<&DiffRow> {
        let mut v: Vec<&DiffRow> = self.rows.iter().collect();
        v.sort_by(|x, y| y.sort.unwrap_or(f64::MIN).partial_cmp(&x.sort.unwrap_or(f64::MIN)).unwrap_or(std::cmp::Ordering::Equal));
        v
    }
}

pub fn num(m: &Map<String, Value>, key: &str) -> Option<f64> {
    m.get(key).and_then(Value::as_f64)
}

/// B minus A for one metric of one row.
pub fn delta(r: &DiffRow, key: &str) -> Option<f64> {
    Some(num(&r.b, key)? - num(&r.a, key)?)
}

/// Replaces rows with the same key and adds the rest at the end. Returns how many were new.
pub fn upsert<T: Clone>(into: &mut Vec<T>, add: Vec<T>, key: impl Fn(&T) -> &str) -> usize {
    let mut fresh = 0;
    for x in add {
        match into.iter().position(|y| key(y) == key(&x)) {
            Some(i) => into[i] = x,
            None => {
                into.push(x);
                fresh += 1;
            }
        }
    }
    fresh
}

impl Board {
    /// Groups in display order: by `sort`, bigger first; unsorted ones keep their order after.
    pub fn sorted(&self) -> Vec<&Group> {
        let mut v: Vec<&Group> = self.groups.iter().collect();
        v.sort_by(|x, y| y.sort.unwrap_or(f64::MIN).partial_cmp(&x.sort.unwrap_or(f64::MIN)).unwrap_or(std::cmp::Ordering::Equal));
        v
    }
    pub fn group(&self, key: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.key == key)
    }
    /// Every item any group mentions.
    pub fn items(&self) -> Vec<String> {
        let mut out = Vec::new();
        for g in &self.groups {
            for m in g.playable() {
                if !out.contains(&m.item) {
                    out.push(m.item.clone());
                }
            }
        }
        out
    }
    /// How many group chips are in each tone, in first-seen order of good, close, bad, other.
    pub fn tally(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = ["good", "close", "bad"].iter().map(|t| ((*t).to_owned(), 0)).collect();
        for g in &self.groups {
            let Some(c) = &g.chip else { continue };
            let tone = if matches!(c.tone.as_str(), "good" | "close" | "bad") { c.tone.clone() } else { "other".into() };
            match out.iter_mut().find(|(t, _)| *t == tone) {
                Some((_, n)) => *n += 1,
                None => out.push((tone, 1)),
            }
        }
        out.retain(|(_, n)| *n > 0);
        out
    }
}

/// Where the sound is in a file: the first and last envelope columns within `floor_db` of the
/// peak, padded a little, as seconds. One-shots sit in a sliver of their file; this is the
/// sliver.
pub fn sound_range(env: &[[f32; 3]], secs: f64, floor_db: f32) -> (f64, f64) {
    if env.is_empty() || secs <= 0.0 {
        return (0.0, secs.max(0.0));
    }
    let amp = |c: &[f32; 3]| c[0].abs().max(c[1].abs());
    let peak = env.iter().map(amp).fold(0f32, f32::max);
    if peak <= 0.0 {
        return (0.0, secs);
    }
    let floor = peak * 10f32.powf(floor_db / 20.0);
    let first = env.iter().position(|c| amp(c) >= floor).unwrap_or(0);
    let last = env.iter().rposition(|c| amp(c) >= floor).unwrap_or(env.len() - 1);
    let col = secs / env.len() as f64;
    let (a, b) = (first as f64 * col, (last + 1) as f64 * col);
    let pad = ((b - a) * 0.08).max(0.005);
    ((a - pad).max(0.0), (b + pad).min(secs))
}

/// The latest decision on each item, from feedback rows.
pub fn decisions(rows: &[crate::feedback::Row]) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for r in rows.iter().filter(|r| r.kind == "decision") {
        if let (Some(id), Some(v)) = (r.items.first(), r.value.as_str()) {
            out.insert(id.clone(), v.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn upsert_replaces_by_key_and_appends_the_rest() {
        let mut v = vec![Group { key: "a".into(), sort: Some(1.0), ..Group::default() }];
        let n = upsert(&mut v, vec![Group { key: "a".into(), sort: Some(5.0), ..Group::default() }, Group { key: "b".into(), ..Group::default() }], |g| &g.key);
        assert_eq!(n, 1);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].sort, Some(5.0));
    }

    #[test]
    fn groups_sort_biggest_first_and_tally_their_chips() {
        let chip = |t: &str| Some(Chip { text: t.into(), tone: t.into() });
        let b = Board {
            groups: vec![
                Group { key: "x".into(), sort: Some(10.0), chip: chip("close"), ..Group::default() },
                Group { key: "y".into(), sort: Some(900.0), chip: chip("good"), ..Group::default() },
                Group { key: "z".into(), chip: chip("close"), ..Group::default() },
            ],
            ..Board::default()
        };
        assert_eq!(b.sorted().iter().map(|g| g.key.as_str()).collect::<Vec<_>>(), ["y", "x", "z"]);
        assert_eq!(b.tally(), vec![("good".to_owned(), 1), ("close".to_owned(), 2)]);
    }

    #[test]
    fn a_profile_target_drops_in_without_its_prefix() {
        let g: Group = serde_json::from_value(json!({"key": "pop", "target": {"target_decay_ms": 54.0, "target_bands_db": {"sub": -12.0}, "bed": false}})).unwrap();
        let t = g.target().unwrap();
        assert_eq!(t["decay_ms"], json!(54.0));
        assert!(t.contains_key("bands_db") && t.contains_key("bed"));
    }

    #[test]
    fn diff_deltas_know_which_way_is_better() {
        let d: Diff = serde_json::from_value(json!({
            "a": "v1", "b": "v2",
            "metrics": [{"key": "vibe", "better": "higher"}, {"key": "shape", "better": "lower"}, {"key": "secs"}],
            "rows": [
                {"key": "pop", "a": {"vibe": 0.9, "shape": 0.8, "secs": 1}, "b": {"vibe": 1.0, "shape": 0.9, "secs": 2}},
                {"key": "riser", "a": {"vibe": 0.9, "shape": 0.8}, "b": {"vibe": 0.8, "shape": 0.5}},
                {"key": "new", "b": {"vibe": 1.0}},
            ]
        }))
        .unwrap();
        let t = d.tally();
        assert_eq!((t[0].0, t[0].1), (1, 1));
        assert_eq!((t[1].0, t[1].1), (1, 1));
        assert_eq!((t[2].0, t[2].1), (0, 0));
        assert!((t[0].2.unwrap() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn the_sound_is_found_inside_its_silence() {
        let mut env = vec![[0.0f32, 0.0, 0.0]; 1000];
        for c in &mut env[400..420] {
            *c = [-0.5, 0.5, 0.3];
        }
        let (a, b) = sound_range(&env, 10.0, -45.0);
        assert!(a > 3.9 && a < 4.0, "{a}");
        assert!(b > 4.2 && b < 4.3, "{b}");
        assert_eq!(sound_range(&[], 2.0, -45.0), (0.0, 2.0));
    }
}
