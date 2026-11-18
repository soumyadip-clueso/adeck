//! Stems: a track split into its parts (drums, bass, other and vocals; or six, with guitar and
//! piano), so each can be heard and seen on its own. A split sits beside its track as
//! `<name>.stems/` (or `<name>.stems-<model>/` for a second model's): one audio file per stem plus
//! `stems.json`, which lists the stems, the model and each stem's loudness. A board, a share or
//! anything else finds them by location. `scripts/stems.py` (run through uv) makes them, once
//! per track: it skips any whose `stems.json` still matches it.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

use crate::config::{self, Config};

/// The usual order of a split's stems: rhythm section first, the catch-all last. A stem
/// `stems.json` names that isn't here goes after these, in the order the file gives.
pub const ORDER: [&str; 6] = ["drums", "bass", "guitar", "piano", "vocals", "other"];
/// What a `<name>.stems/` folder without a `stems.json` list holds: the 4-stem split.
pub const FOUR: [&str; 4] = ["drums", "bass", "other", "vocals"];
const SCRIPT: &str = include_str!("../scripts/stems.py");
const EXTS: [&str; 4] = ["flac", "wav", "mp3", "m4a"];

/// One stem set on disk for one track: a `<name>.stems*/` folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Stems {
    /// What tells this set apart from the track's others: "" for `<name>.stems`, else what
    /// follows `.stems-` (e.g. "htdemucs_6s").
    pub key: String,
    /// Stem names, in `ORDER`.
    pub names: Vec<String>,
    /// One file per name.
    pub files: Vec<PathBuf>,
    pub model: Option<String>,
    /// Each stem's loudness (LUFS) and share of the stems' summed loudness energy, 0 to 1.
    pub lufs: Vec<Option<f64>>,
    pub share: Vec<Option<f64>>,
}

impl Stems {
    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
    pub fn share_of(&self, name: &str) -> Option<f64> {
        self.share[self.index(name)?]
    }
    /// The bass stem's share of the loudness: the "low end buries the rest" number.
    pub fn bass_share(&self) -> Option<f64> {
        self.share_of("bass")
    }
    /// The model for people: its file name without the extension, else the folder's key.
    pub fn label(&self) -> String {
        match &self.model {
            Some(m) => m.rsplit_once('.').filter(|(_, e)| ["ckpt", "yaml", "th", "onnx", "pth"].contains(e)).map_or(m.as_str(), |(b, _)| b).to_owned(),
            None if !self.key.is_empty() => self.key.clone(),
            None => "stems".into(),
        }
    }
}

/// Where a stem name sorts: its place in `ORDER`, unknown ones after.
pub fn rank(name: &str) -> usize {
    ORDER.iter().position(|n| *n == name).unwrap_or(ORDER.len())
}

/// `<dir>/<name>.stems` for `<dir>/<name>.<ext>`: where a split lands by default.
pub fn dir_for(track: &Path) -> PathBuf {
    track.with_file_name(format!("{}.stems", base(track)))
}

fn base(track: &Path) -> String {
    track.file_stem().map_or_else(|| "track".into(), |s| s.to_string_lossy().into_owned())
}

/// A folder of stems: `<anything>.stems` or `<anything>.stems-<model>`.
pub fn is_stems_dir(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "stems" || e.to_string_lossy().starts_with("stems-"))
}

#[derive(Deserialize, Default)]
struct Meta {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stems: Option<Vec<String>>,
    #[serde(default)]
    levels: Option<Levels>,
}

#[derive(Deserialize, Default)]
struct Levels {
    #[serde(default)]
    stems: std::collections::HashMap<String, Level>,
}

#[derive(Deserialize, Default)]
struct Level {
    lufs: Option<f64>,
    share: Option<f64>,
}

/// One set from its folder, when every stem it lists is there. The list is `stems.json`'s
/// `stems`; a folder without one is the 4-stem split.
pub fn read_set(dir: &Path, key: &str) -> Option<Stems> {
    let meta: Meta = std::fs::read_to_string(dir.join("stems.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut names: Vec<String> = meta.stems.filter(|v| !v.is_empty()).unwrap_or_else(|| FOUR.iter().map(|s| (*s).to_owned()).collect());
    names.dedup();
    // Stable: unknown names keep the file's order after the known ones.
    names.sort_by_key(|n| rank(n));
    let one = |name: &str| EXTS.iter().map(|e| dir.join(format!("{name}.{e}"))).find(|p| p.is_file());
    let files: Vec<PathBuf> = names.iter().map(|n| one(n)).collect::<Option<_>>()?;
    let lv = meta.levels.unwrap_or_default();
    let get = |n: &String| lv.stems.get(n);
    Some(Stems {
        key: key.to_owned(),
        lufs: names.iter().map(|n| get(n).and_then(|l| l.lufs)).collect(),
        share: names.iter().map(|n| get(n).and_then(|l| l.share)).collect(),
        names,
        files,
        model: meta.model,
    })
}

/// Every stem set beside a track: `<name>.stems/` first, then `<name>.stems-*/` by name.
pub fn find_all(track: &Path) -> Vec<Stems> {
    let (Some(parent), name) = (track.parent(), base(track)) else { return Vec::new() };
    let plain = format!("{name}.stems");
    let pre = format!("{plain}-");
    let mut keys: Vec<String> = std::fs::read_dir(parent)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| {
                    let f = e.file_name().to_string_lossy().into_owned();
                    if f == plain { Some(String::new()) } else { f.strip_prefix(&pre).filter(|k| !k.is_empty() && !k.ends_with(".part")).map(str::to_owned) }
                })
                .collect()
        })
        .unwrap_or_default();
    keys.sort();
    keys.into_iter().filter_map(|k| read_set(&parent.join(if k.is_empty() { plain.clone() } else { format!("{pre}{k}") }), &k)).collect()
}

/// The track's first stem set, if it has one.
#[cfg(test)]
pub fn find(track: &Path) -> Option<Stems> {
    find_all(track).into_iter().next()
}

/// uv: the configured one, else the usual install places (an app launched from the Dock has no
/// shell PATH).
pub fn uv(cfg: &Config) -> Option<PathBuf> {
    if !cfg.stems.uv.is_empty() {
        return Some(config::expand(Path::new(&cfg.stems.uv)));
    }
    let home = config::home();
    [home.join(".local/bin/uv"), home.join(".cargo/bin/uv"), "/opt/homebrew/bin/uv".into(), "/usr/local/bin/uv".into()].into_iter().find(|p| p.is_file())
}

/// The script, written into the cache so an installed app can run it.
fn script() -> Result<PathBuf, String> {
    let p = config::cache_dir().join("stems.py");
    if std::fs::read_to_string(&p).ok().as_deref() != Some(SCRIPT) {
        std::fs::create_dir_all(config::cache_dir()).map_err(|e| e.to_string())?;
        std::fs::write(&p, SCRIPT).map_err(|e| e.to_string())?;
    }
    Ok(p)
}

/// One line the script prints per track (and one when the model has loaded).
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub file: Option<PathBuf>,
    /// model | done | cached | failed
    pub status: String,
    pub error: Option<String>,
}

/// Splits `files` (skipping any already split), reporting each as it lands. Blocking; setting
/// `cancel` stops the splitter within a fraction of a second.
pub fn split(cfg: &Config, files: &[PathBuf], cancel: &std::sync::atomic::AtomicBool, mut each: impl FnMut(Progress)) -> Result<(usize, usize), String> {
    let uv = uv(cfg).ok_or("Splitting stems needs uv (https://docs.astral.sh/uv/): install it, or set [stems] uv = \"/path/to/uv\" in the config.")?;
    let script = script()?;
    let mut c = Command::new(&uv);
    // An explicit Python, so no .python-version above the cache folder can pin an old one.
    c.args(["run", "--quiet", "--python", "3.12", "--script"]).arg(&script).args(["--model", &cfg.stems.model]).args(files);
    c.current_dir(config::cache_dir()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // ffmpeg for decoding: homebrew's, when the app has no shell PATH.
    let path = std::env::var("PATH").unwrap_or_default();
    c.env("PATH", format!("{path}:/opt/homebrew/bin:/usr/local/bin"));
    let mut child = c.spawn().map_err(|e| format!("Couldn't run {}: {e}", uv.display()))?;
    let err = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = std::io::Read::read_to_string(&mut s, &mut buf);
            buf
        })
    });
    let out = child.stdout.take();
    let child = std::sync::Arc::new(std::sync::Mutex::new(child));
    let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    std::thread::scope(|sc| {
        let (kid, over) = (child.clone(), finished.clone());
        sc.spawn(move || {
            while !over.load(std::sync::atomic::Ordering::Acquire) {
                if cancel.load(std::sync::atomic::Ordering::Acquire) {
                    if let Ok(mut c) = kid.lock() {
                        let _ = c.kill();
                    }
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        });
        let r = read_lines(out, &mut each);
        finished.store(true, std::sync::atomic::Ordering::Release);
        r
    })
    .and_then(|(done, failed)| {
        let status = child.lock().map_err(|e| e.to_string())?.wait().map_err(|e| e.to_string())?;
        let stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err("Stopped".into());
        }
        if !status.success() && done == 0 {
            let why = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output").to_owned();
            return Err(format!("The stem splitter failed: {why}"));
        }
        Ok((done, failed))
    })
}

fn read_lines(out: Option<std::process::ChildStdout>, each: &mut impl FnMut(Progress)) -> Result<(usize, usize), String> {
    let (mut done, mut failed) = (0, 0);
    if let Some(out) = out {
        for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            let p = Progress {
                file: v["file"].as_str().map(PathBuf::from),
                status: v["status"].as_str().unwrap_or_default().to_owned(),
                error: v["error"].as_str().map(str::to_owned),
            };
            match p.status.as_str() {
                "done" | "cached" => done += 1,
                "failed" => failed += 1,
                _ => {}
            }
            each(p);
        }
    }
    Ok((done, failed))
}

/// Linear gain per stem: soloed ones when any is soloed, else every one not muted.
pub fn gains<S: AsRef<str>>(names: &[S], solo: &std::collections::BTreeSet<String>, mute: &std::collections::BTreeSet<String>) -> Vec<f32> {
    let any = names.iter().any(|n| solo.contains(n.as_ref()));
    names.iter().map(|n| if (any && solo.contains(n.as_ref())) || (!any && !mute.contains(n.as_ref())) { 1.0 } else { 0.0 }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("adeck-stems-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn stems_sit_beside_their_track() {
        assert_eq!(dir_for(Path::new("/a/run/take-01.mp3")), PathBuf::from("/a/run/take-01.stems"));
        assert_eq!(dir_for(Path::new("/a/run/x.y.wav")), PathBuf::from("/a/run/x.y.stems"));
        assert!(is_stems_dir(Path::new("/a/x.stems")) && is_stems_dir(Path::new("/a/x.stems-htdemucs_6s")));
        assert!(!is_stems_dir(Path::new("/a/stems")) && !is_stems_dir(Path::new("/a/x.stemsy")));
    }

    #[test]
    fn four_found_only_when_all_are_there_and_levels_are_read() {
        let root = tmp("four");
        let track = root.join("t.mp3");
        let d = dir_for(&track);
        std::fs::create_dir_all(&d).unwrap();
        for n in &FOUR[..3] {
            std::fs::write(d.join(format!("{n}.flac")), b"x").unwrap();
        }
        assert!(find(&track).is_none());
        std::fs::write(d.join("vocals.flac"), b"x").unwrap();
        std::fs::write(d.join("stems.json"), r#"{"model":"htdemucs_ft","levels":{"stems":{"bass":{"lufs":-20.5,"share":0.4}}}}"#).unwrap();
        let s = find(&track).unwrap();
        assert_eq!(s.names, ["drums", "bass", "vocals", "other"]);
        assert_eq!((s.model.as_deref(), s.label(), s.key.as_str()), (Some("htdemucs_ft"), "htdemucs_ft".to_owned(), ""));
        assert_eq!(s.bass_share(), Some(0.4));
        assert_eq!((s.lufs[1], s.lufs[0]), (Some(-20.5), None));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn six_stem_sets_are_listed_in_order_and_each_is_found() {
        let root = tmp("six");
        let track = root.join("song.mp3");
        let six = ["bass", "drums", "other", "vocals", "guitar", "piano", "synth"];
        for (dir, model) in [("song.stems", "BS-Roformer-SW.ckpt"), ("song.stems-htdemucs_6s", "htdemucs_6s.yaml")] {
            let d = root.join(dir);
            std::fs::create_dir_all(&d).unwrap();
            for n in six {
                std::fs::write(d.join(format!("{n}.flac")), b"x").unwrap();
            }
            let meta = serde_json::json!({"model": model, "stems": six, "levels": {"stems": {"piano": {"lufs": -18.3, "share": 0.04}}}});
            std::fs::write(d.join("stems.json"), meta.to_string()).unwrap();
        }
        // Half-written or someone else's: not sets.
        std::fs::create_dir_all(root.join("song.stems-x.part")).unwrap();
        std::fs::create_dir_all(root.join("songs.stems")).unwrap();
        let all = find_all(&track);
        assert_eq!(all.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), ["", "htdemucs_6s"]);
        assert_eq!(all.iter().map(Stems::label).collect::<Vec<_>>(), ["BS-Roformer-SW", "htdemucs_6s"]);
        assert_eq!(all[0].names, ["drums", "bass", "guitar", "piano", "vocals", "other", "synth"]);
        assert_eq!(all[0].files[3], root.join("song.stems/piano.flac"));
        assert_eq!((all[0].share_of("piano"), all[0].lufs[3]), (Some(0.04), Some(-18.3)));
        // A listed stem missing from the folder: the set isn't whole.
        std::fs::remove_file(root.join("song.stems-htdemucs_6s/guitar.flac")).unwrap();
        assert_eq!(find_all(&track).len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn solo_wins_over_mute() {
        let set = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<BTreeSet<_>>();
        let n = ["drums", "bass", "guitar", "piano", "vocals", "other"];
        assert_eq!(gains(&n, &set(&[]), &set(&[])), [1.0; 6]);
        assert_eq!(gains(&n, &set(&[]), &set(&["bass"])), [1.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
        assert_eq!(gains(&n, &set(&["piano"]), &set(&["bass"])), [0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        // A solo on a stem this set doesn't have (switched from a 6-stem set) hears nothing extra.
        assert_eq!(gains(&n[..4], &set(&["vocals"]), &set(&[])), [1.0; 4]);
    }
}
