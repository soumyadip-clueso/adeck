//! Stems: a track split into drums, bass, other and vocals, so each part can be heard and seen
//! on its own. The splitting is `scripts/stems.py` (Demucs, run through uv); its output sits
//! beside the track as `<name>.stems/{drums,bass,other,vocals}.flac` plus `stems.json` (the
//! model and each stem's loudness), so a board, a share or anything else finds it by location.
//! A track is split once: the script skips any whose `stems.json` still matches it.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

use crate::config::{self, Config};

pub const NAMES: [&str; 4] = ["drums", "bass", "other", "vocals"];
const SCRIPT: &str = include_str!("../scripts/stems.py");
const EXTS: [&str; 4] = ["flac", "wav", "mp3", "m4a"];

/// What's on disk for one track.
#[derive(Clone, Debug, PartialEq)]
pub struct Stems {
    /// Drums, bass, other, vocals.
    pub files: [PathBuf; 4],
    pub model: Option<String>,
    /// Each stem's loudness (LUFS) and share of the stems' summed loudness energy, 0 to 1.
    pub lufs: [Option<f64>; 4],
    pub share: [Option<f64>; 4],
}

impl Stems {
    /// The bass stem's share of the loudness: the "low end buries the rest" number.
    pub fn bass_share(&self) -> Option<f64> {
        self.share[1]
    }
}

/// `<dir>/<name>.stems` for `<dir>/<name>.<ext>`.
pub fn dir_for(track: &Path) -> PathBuf {
    let name = track.file_stem().map_or_else(|| "track".into(), |s| s.to_string_lossy().into_owned());
    track.with_file_name(format!("{name}.stems"))
}

#[derive(Deserialize, Default)]
struct Meta {
    #[serde(default)]
    model: Option<String>,
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

/// The stems beside a track, when all four are there.
pub fn find(track: &Path) -> Option<Stems> {
    let dir = dir_for(track);
    let one = |name: &str| EXTS.iter().map(|e| dir.join(format!("{name}.{e}"))).find(|p| p.is_file());
    let files = [one(NAMES[0])?, one(NAMES[1])?, one(NAMES[2])?, one(NAMES[3])?];
    let meta: Meta = std::fs::read_to_string(dir.join("stems.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let lv = meta.levels.unwrap_or_default();
    let get = |i: usize| lv.stems.get(NAMES[i]);
    Some(Stems { files, model: meta.model, lufs: std::array::from_fn(|i| get(i).and_then(|l| l.lufs)), share: std::array::from_fn(|i| get(i).and_then(|l| l.share)) })
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
pub fn gains(solo: [bool; 4], mute: [bool; 4]) -> [f32; 4] {
    let any = solo.iter().any(|s| *s);
    std::array::from_fn(|i| if (any && solo[i]) || (!any && !mute[i]) { 1.0 } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_sit_beside_their_track() {
        assert_eq!(dir_for(Path::new("/a/run/take-01.mp3")), PathBuf::from("/a/run/take-01.stems"));
        assert_eq!(dir_for(Path::new("/a/run/x.y.wav")), PathBuf::from("/a/run/x.y.stems"));
    }

    #[test]
    fn found_only_when_all_four_are_there_and_levels_are_read() {
        let root = std::env::temp_dir().join(format!("adeck-stems-test-{}", std::process::id()));
        let track = root.join("t.mp3");
        let d = dir_for(&track);
        std::fs::create_dir_all(&d).unwrap();
        for n in &NAMES[..3] {
            std::fs::write(d.join(format!("{n}.flac")), b"x").unwrap();
        }
        assert!(find(&track).is_none());
        std::fs::write(d.join("vocals.flac"), b"x").unwrap();
        std::fs::write(d.join("stems.json"), r#"{"model":"htdemucs_ft","levels":{"stems":{"bass":{"lufs":-20.5,"share":0.4}}}}"#).unwrap();
        let s = find(&track).unwrap();
        assert_eq!(s.model.as_deref(), Some("htdemucs_ft"));
        assert_eq!(s.bass_share(), Some(0.4));
        assert_eq!(s.lufs[1], Some(-20.5));
        assert_eq!(s.lufs[0], None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn solo_wins_over_mute() {
        assert_eq!(gains([false; 4], [false; 4]), [1.0; 4]);
        assert_eq!(gains([false; 4], [false, true, false, false]), [1.0, 0.0, 1.0, 1.0]);
        assert_eq!(gains([false, false, true, false], [false, true, false, false]), [0.0, 0.0, 1.0, 0.0]);
    }
}
