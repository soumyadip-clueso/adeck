//! One file adeck can play: where it is, what it is, and everything written about it.
//!
//! Metadata is found, not configured. A sidecar `<stem>.json` beside the file wins (what a
//! generation worker writes). Otherwise every JSON file in the file's folder and the one above
//! is searched for an object that names this file in one of its string fields (what a
//! hill-climb summary writes); that object, plus the plain fields of the objects around it, is
//! the metadata. Either way it's flattened to dotted keys so any two items can be diffed.

use std::path::{Path, PathBuf};

use serde_json::Value;

pub const AUDIO_EXT: &[&str] = &["wav", "mp3", "flac", "aif", "aiff", "m4a", "ogg", "aac"];
pub const VIDEO_EXT: &[&str] = &["mp4", "mov", "m4v"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Media {
    Audio,
    Video,
}

pub fn media_of(path: &Path) -> Option<Media> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if AUDIO_EXT.contains(&ext.as_str()) {
        Some(Media::Audio)
    } else if VIDEO_EXT.contains(&ext.as_str()) {
        Some(Media::Video)
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub struct Item {
    /// How tools and the session name it: an absolute path, `ref:…`, `s3://…`.
    pub id: String,
    /// The local file (an S3 item's cached copy).
    pub path: PathBuf,
    pub name: String,
    pub media: Media,
    /// The run it belongs to, by name, if any.
    pub run: Option<String>,
    /// From the local-only reference pack: shown with a badge, never sent anywhere.
    pub is_ref: bool,
    /// Flattened metadata, in the order it was written.
    pub meta: Vec<(String, String)>,
    /// The JSON file the metadata came from.
    pub meta_src: Option<PathBuf>,
}

impl Item {
    pub fn new(id: String, path: PathBuf, run: Option<String>, is_ref: bool) -> Option<Item> {
        let media = media_of(&path)?;
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_owned();
        let (meta, meta_src) = find_meta(&path).map_or((Vec::new(), None), |(v, src)| (flatten(&v), Some(src)));
        Some(Item { id, path, name, media, run, is_ref, meta, meta_src })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.meta.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// The first of these keys present, matched on the last path segment too, so `bpm` finds
    /// `analysis.bpm`.
    pub fn pick(&self, keys: &[&str]) -> Option<&str> {
        for want in keys {
            if let Some(v) = self.get(want) {
                return Some(v);
            }
            if let Some((_, v)) = self.meta.iter().find(|(k, _)| k.rsplit('.').next() == Some(want)) {
                return Some(v);
            }
        }
        None
    }

    /// A short line under the name: model, mood or type.
    pub fn subtitle(&self) -> String {
        let parts: Vec<&str> = [self.pick(&["model"]), self.pick(&["mood", "type", "kind"]), self.pick(&["chain"])].into_iter().flatten().collect();
        parts.join(" · ")
    }

    /// A label for a tab or a member: the run's last folder and the name.
    pub fn label(&self) -> String {
        match self.path.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()) {
            Some(dir) if !self.is_ref => format!("{dir}/{}", self.name),
            _ => self.name.clone(),
        }
    }

    pub fn to_json(&self) -> Value {
        let meta: serde_json::Map<String, Value> = self.meta.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "media": self.media,
            "run": self.run,
            "ref": self.is_ref,
            // Reference files never leave this machine, not even their paths.
            "path": if self.is_ref { Value::Null } else { Value::String(self.path.display().to_string()) },
            "meta": meta,
        })
    }
}

/// The metadata JSON for `path`, and the file it came from.
pub fn find_meta(path: &Path) -> Option<(Value, PathBuf)> {
    let sidecar = path.with_extension("json");
    if let Some(v) = read_json(&sidecar) {
        return Some((v, sidecar));
    }
    let name = path.file_name()?.to_str()?;
    let dir = path.parent()?;
    // A JSON beside the file may name it alone; one a folder up must name its folder too, since
    // sibling folders often hold files with the same names.
    let with_dir = format!("{}/{name}", dir.file_name()?.to_str()?);
    for (folder, want) in [(Some(dir), name), (dir.parent(), with_dir.as_str())] {
        let Some(folder) = folder else { continue };
        let Ok(rd) = std::fs::read_dir(folder) else {
            continue;
        };
        let mut jsons: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
        jsons.sort();
        for j in jsons {
            // Big manifests are a different job; keep discovery quick.
            if std::fs::metadata(&j).map(|m| m.len()).unwrap_or(0) > 8 << 20 {
                continue;
            }
            let Some(v) = read_json(&j) else { continue };
            if let Some(found) = naming(&v, want, &serde_json::Map::new()) {
                return Some((found, j));
            }
        }
    }
    None
}

fn read_json(p: &Path) -> Option<Value> {
    let s = std::fs::read_to_string(p).ok()?;
    serde_json::from_str(&s).ok()
}

/// The innermost object under `v` with a string field ending in `/name` (or equal to it), with
/// the plain fields of the objects around it carried in, nearest last so they don't override.
fn naming(v: &Value, name: &str, around: &serde_json::Map<String, Value>) -> Option<Value> {
    match v {
        Value::Object(map) => {
            let mut here = around.clone();
            for (k, x) in map {
                if !x.is_object() && !x.is_array() {
                    here.insert(k.clone(), x.clone());
                }
            }
            for x in map.values() {
                if let Some(found) = naming(x, name, &here) {
                    return Some(found);
                }
            }
            let names_it = map.values().any(|x| x.as_str().is_some_and(|s| s == name || s.ends_with(&format!("/{name}")) || Path::new(s).ends_with(name)));
            if names_it {
                let mut out = around.clone();
                for (k, x) in map {
                    out.insert(k.clone(), x.clone());
                }
                return Some(Value::Object(out));
            }
            None
        }
        Value::Array(xs) => xs.iter().find_map(|x| naming(x, name, around)),
        _ => None,
    }
}

/// Dotted keys and printable values. Arrays of scalars join with commas; long arrays of
/// objects are summarised so a manifest can't flood the panel.
pub fn flatten(v: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk(v, String::new(), &mut out);
    out
}

fn walk(v: &Value, prefix: String, out: &mut Vec<(String, String)>) {
    let key = |k: &str| {
        if prefix.is_empty() { k.to_owned() } else { format!("{prefix}.{k}") }
    };
    match v {
        Value::Object(map) => {
            for (k, x) in map {
                walk(x, key(k), out);
            }
        }
        Value::Array(xs) if xs.iter().all(|x| !x.is_object() && !x.is_array()) => {
            let parts: Vec<String> = xs.iter().map(scalar).collect();
            let mut s = parts.join(", ");
            if s.len() > 400 {
                s.truncate(400);
                s.push('…');
            }
            out.push((prefix, s));
        }
        Value::Array(xs) if xs.len() <= 8 => {
            for (i, x) in xs.iter().enumerate() {
                walk(x, key(&i.to_string()), out);
            }
        }
        Value::Array(xs) => out.push((prefix, format!("[{} entries]", xs.len()))),
        _ => out.push((prefix, scalar(v))),
    }
}

fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() => {
                let s = format!("{f:.4}");
                s.trim_end_matches('0').trim_end_matches('.').to_owned()
            }
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// The keys whose values differ between items, in first-seen order. Keys an item lacks count
/// as different.
pub fn differing(items: &[&Item]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for it in items {
        for (k, _) in &it.meta {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    keys.retain(|k| {
        let first = items.first().and_then(|i| i.get(k));
        items.iter().any(|i| i.get(k) != first)
    });
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flattening_keeps_order_and_summarises_long_lists() {
        let v = json!({"model": "sa3", "analysis": {"bpm": 124.0, "lufs": -14.25}, "flags": ["vocals", "clipping"], "rows": (0..20).map(|i| json!({"i": i})).collect::<Vec<_>>()});
        let f = flatten(&v);
        assert_eq!(f[0], ("model".into(), "sa3".into()));
        assert!(f.contains(&("analysis.bpm".into(), "124".into())));
        assert!(f.contains(&("analysis.lufs".into(), "-14.25".into())));
        assert!(f.contains(&("flags".into(), "vocals, clipping".into())));
        assert!(f.contains(&("rows".into(), "[20 entries]".into())));
    }

    #[test]
    fn a_summary_that_names_the_file_gives_its_object_and_the_fields_around_it() {
        let v = json!({"kind": "whoosh-soft", "takes": [{"prompt": "soft whoosh", "versions": [
            {"chain": "v1", "path": "work/x/t0-0-v1.wav", "total": 0.5},
            {"chain": "raw", "path": "work/x/t0-0-raw.wav", "total": 0.9}]}]});
        let found = naming(&v, "t0-0-raw.wav", &serde_json::Map::new()).unwrap();
        assert_eq!(found["chain"], "raw");
        assert_eq!(found["prompt"], "soft whoosh");
        assert_eq!(found["kind"], "whoosh-soft");
    }

    #[test]
    fn a_summary_one_folder_up_must_name_the_folder_too() {
        let root = std::env::temp_dir().join(format!("adeck-meta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for kind in ["boom-deep", "whoosh-soft"] {
            std::fs::create_dir_all(root.join(kind)).unwrap();
            std::fs::write(root.join(kind).join("t0-0-raw.wav"), b"").unwrap();
            let summary = json!({"kind": kind, "takes": [{"versions": [{"chain": "raw", "path": format!("work/{kind}/t0-0-raw.wav")}]}]});
            std::fs::write(root.join(format!("{kind}.json")), summary.to_string()).unwrap();
        }
        let (v, src) = find_meta(&root.join("whoosh-soft/t0-0-raw.wav")).unwrap();
        assert_eq!(v["kind"], "whoosh-soft");
        assert!(src.ends_with("whoosh-soft.json"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn differing_keys_include_ones_an_item_lacks() {
        let mk = |meta: Vec<(&str, &str)>| Item {
            id: String::new(),
            path: PathBuf::new(),
            name: String::new(),
            media: Media::Audio,
            run: None,
            is_ref: false,
            meta: meta.into_iter().map(|(a, b)| (a.into(), b.into())).collect(),
            meta_src: None,
        };
        let a = mk(vec![("model", "sa3"), ("seed", "1"), ("steps", "8")]);
        let b = mk(vec![("model", "sa3"), ("seed", "2")]);
        assert_eq!(differing(&[&a, &b]), vec!["seed".to_owned(), "steps".to_owned()]);
    }
}
