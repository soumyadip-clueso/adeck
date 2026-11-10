//! Sharing a tab as a web page: a self-contained page (players, waveforms, numbers) plus its
//! audio, published by a command you configure, and a record of every share so the Shared tab
//! can list them and take them down, audio included.
//!
//! The publisher is a program named in `[share] command`, called three ways:
//!   `<command> publish <dir> <slug> <title>` → prints JSON with a `url`
//!   `<command> remove <slug>`                 → takes the page down
//!   `<command> list`                          → prints JSON: a list (or `{"sites": [...]}`) of `{slug, url}`
//! Audio goes in the page's folder (`audio = "bundle"`), or to S3 (`audio = "s3"`) under
//! `<prefix>/<slug>/`, served from `public_url`. Reference-pack files are never shared.
//! Files under a `[[share.links]]` folder that its URL already serves (same size) aren't copied
//! at all: the page plays them from there.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{self, ShareCfg};

const PAGE: &str = include_str!("../assets/share.html");

/// One published page.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Share {
    pub slug: String,
    pub title: String,
    pub url: String,
    /// Unix seconds.
    pub created: u64,
    pub files: usize,
    /// Where its audio went: `bundle`, or the S3 URI of its folder.
    pub audio: String,
    /// Set when the publisher no longer lists it (taken down elsewhere).
    #[serde(default)]
    pub gone: bool,
}

/// What a tab hands over: the page's data and the files it plays.
pub struct Export {
    pub title: String,
    pub page: Value,
    /// (local file, name in the page). The page refers to each by name.
    pub files: Vec<(PathBuf, String)>,
}

fn store() -> PathBuf {
    config::dir().join("shares.json")
}

pub fn load() -> Vec<Share> {
    std::fs::read_to_string(store()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(all: &[Share]) {
    let _ = std::fs::create_dir_all(config::dir());
    if let Ok(s) = serde_json::to_string_pretty(all) {
        let _ = std::fs::write(store(), s);
    }
}

/// A slug for a title: the prefix, a few words, a short random tail.
pub fn slug(prefix: &str, title: &str) -> String {
    let words: String = title.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let words: Vec<&str> = words.split('-').filter(|w| !w.is_empty()).collect();
    let mut base = words.join("-");
    base.truncate(32);
    let base = base.trim_end_matches('-');
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let tail = format!("{:04x}", (nanos ^ (nanos >> 17)) as u16);
    format!("{prefix}{}{tail}", if base.is_empty() { String::new() } else { format!("{base}-") })
}

/// Builds the page and its audio in a fresh folder, puts the audio where the config says, and
/// publishes it. Blocking: run it off the UI thread.
pub fn publish(x: Export, cfg: &ShareCfg, aws: &str) -> Result<Share, String> {
    let command = cfg.command().ok_or("Sharing isn't set up: add [share] command = \"…\" to ~/.config/adeck/config.toml (see the README).")?;
    let slug = slug(&cfg.prefix, &x.title);
    let dir = config::cache_dir().join("shares").join(&slug);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("audio")).map_err(|e| e.to_string())?;

    // Audio: browser-friendly, small enough to load quickly.
    let mut names = serde_json::Map::new();
    let mut page = x.page.clone();
    let linked = linked_urls(&x.files, &cfg.links);
    for (src, name) in &x.files {
        if let Some(url) = linked.get(name) {
            names.insert(name.clone(), Value::String(url.clone()));
        } else {
            let (out_name, _) = web_audio(src, name, &dir.join("audio"))?;
            names.insert(name.clone(), Value::String(out_name));
        }
        // Length, loudness and the waveform, from the analysis cache (or a fresh decode).
        if let Ok(a) = crate::derive::analyse(src) {
            let facts = serde_json::json!({"secs": a.secs, "lufs": a.lufs, "peaks": peaks(&a.envelope, 700)});
            fill(&mut page, name, &facts);
        }
    }
    let s3_base = if cfg.audio == "s3" {
        let s3 = &cfg.s3;
        if s3.bucket.is_empty() || s3.public_url.is_empty() {
            return Err("audio = \"s3\" needs [share.s3] bucket and public_url.".into());
        }
        let key = format!("{}/{slug}", s3.prefix.trim_matches('/')).trim_start_matches('/').to_owned();
        let uri = format!("s3://{}/{key}/", s3.bucket);
        aws_run(aws, s3, &["s3", "cp", "--recursive", &dir.join("audio").display().to_string(), &uri])?;
        std::fs::remove_dir_all(dir.join("audio")).map_err(|e| e.to_string())?;
        Some((uri, format!("{}/{key}/", s3.public_url.trim_end_matches('/'))))
    } else {
        None
    };
    let base = s3_base.as_ref().map_or_else(|| "audio/".to_owned(), |(_, url)| url.clone());
    rewrite_srcs(&mut page, &names, &base);
    let json = serde_json::to_string(&page).map_err(|e| e.to_string())?.replace("</", "<\\/");
    let html = PAGE.replace("__TITLE__", &html_escape(&x.title)).replace("/*__DATA__*/null", &json);
    std::fs::write(dir.join("index.html"), html).map_err(|e| e.to_string())?;

    let out = run(&command, &["publish", &dir.display().to_string(), &slug, &x.title])?;
    let url = find_str(&out, "url").ok_or_else(|| format!("The publisher printed no url: {}", String::from_utf8_lossy(&serde_json::to_vec(&out).unwrap_or_default())))?;
    let created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    Ok(Share { slug, title: x.title, url, created, files: x.files.len(), audio: s3_base.map_or_else(|| "bundle".to_owned(), |(uri, _)| uri), gone: false })
}

/// Takes a share down: the page (unless it's already gone) and its S3 audio.
pub fn remove(sh: &Share, cfg: &ShareCfg, aws: &str) -> Result<(), String> {
    if !sh.gone {
        let command = cfg.command().ok_or("Sharing isn't set up.")?;
        run(&command, &["remove", &sh.slug])?;
    }
    clean_audio(sh, cfg, aws)?;
    let _ = std::fs::remove_dir_all(config::cache_dir().join("shares").join(&sh.slug));
    Ok(())
}

/// Deletes a share's audio from S3, if it went there.
pub fn clean_audio(sh: &Share, cfg: &ShareCfg, aws: &str) -> Result<(), String> {
    if let Some(uri) = sh.audio.strip_prefix("s3://").map(|_| sh.audio.as_str()) {
        aws_run(aws, &cfg.s3, &["s3", "rm", "--recursive", uri])?;
    }
    Ok(())
}

/// The slugs the publisher still lists.
pub fn live(cfg: &ShareCfg) -> Result<Vec<String>, String> {
    let command = cfg.command().ok_or("Sharing isn't set up.")?;
    let out = run(&command, &["list"])?;
    let rows = out.get("sites").cloned().unwrap_or(out);
    Ok(rows.as_array().map(|a| a.iter().filter_map(|r| r.get("slug").and_then(Value::as_str).map(str::to_owned)).collect()).unwrap_or_default())
}

fn run(command: &Path, args: &[&str]) -> Result<Value, String> {
    let out = Command::new(command).args(args).output().map_err(|e| format!("Couldn't run {}: {e}", command.display()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "{} {} failed: {}",
            command.file_name().and_then(|s| s.to_str()).unwrap_or("publisher"),
            args[0],
            err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output")
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(serde_json::from_str(text.trim()).unwrap_or(Value::Null))
}

fn aws_run(aws: &str, s3: &config::ShareS3, args: &[&str]) -> Result<(), String> {
    let mut c = Command::new(aws);
    c.args(args);
    if !s3.profile.is_empty() {
        c.args(["--profile", &s3.profile]);
    }
    if !s3.region.is_empty() {
        c.args(["--region", &s3.region]);
    }
    let out = c.output().map_err(|e| format!("Couldn't run aws: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("aws {}: {}", args[..2].join(" "), err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed")))
    }
}

/// Copies a file into `dir` as something every browser plays, shrinking big ones to AAC.
fn web_audio(src: &Path, name: &str, dir: &Path) -> Result<(String, PathBuf), String> {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let size = std::fs::metadata(src).map_or(0, |m| m.len());
    let plays = matches!(ext.as_str(), "wav" | "mp3" | "m4a" | "aac" | "mp4");
    if plays && size <= 4 << 20 {
        let out_name = format!("{name}.{ext}");
        let out = dir.join(&out_name);
        std::fs::copy(src, &out).map_err(|e| format!("Couldn't copy {}: {e}", src.display()))?;
        return Ok((out_name, out));
    }
    let out_name = format!("{name}.m4a");
    let out = dir.join(&out_name);
    let ok = Command::new("afconvert").args(["-f", "m4af", "-d", "aac", "-b", "256000"]).arg(src).arg(&out).output().is_ok_and(|o| o.status.success());
    if !ok {
        let ok = Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(src)
            .args(["-vn", "-c:a", "aac", "-b:a", "256k"])
            .arg(&out)
            .output()
            .is_ok_and(|o| o.status.success());
        if !ok {
            return Err(format!("Couldn't convert {} for the web.", src.display()));
        }
    }
    Ok((out_name, out))
}

/// Copies `facts` into every member whose `src` is `name`.
fn fill(v: &mut Value, name: &str, facts: &Value) {
    match v {
        Value::Object(m) => {
            if m.get("src").and_then(Value::as_str) == Some(name)
                && let Some(f) = facts.as_object()
            {
                for (k, x) in f {
                    m.insert(k.clone(), x.clone());
                }
            }
            for (_, x) in m.iter_mut() {
                fill(x, name, facts);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| fill(x, name, facts)),
        _ => {}
    }
}

/// Points every member's `src` (a file name from `Export::files`) at where it ended up.
fn rewrite_srcs(v: &mut Value, names: &serde_json::Map<String, Value>, base: &str) {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(s)) = m.get("src")
                && let Some(Value::String(n)) = names.get(s)
            {
                let full = if n.starts_with("https://") || n.starts_with("http://") { n.clone() } else { format!("{base}{n}") };
                m.insert("src".into(), Value::String(full));
            }
            for (_, x) in m.iter_mut() {
                rewrite_srcs(x, names, base);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| rewrite_srcs(x, names, base)),
        _ => {}
    }
}

/// Where a file is already public, if a link folder holds it.
fn link_url(src: &Path, links: &[config::ShareLink]) -> Option<String> {
    let src = src.canonicalize().unwrap_or_else(|_| src.to_path_buf());
    links.iter().find_map(|l| {
        let dir = config::expand(&l.dir);
        let dir = dir.canonicalize().unwrap_or(dir);
        let rel = src.strip_prefix(&dir).ok()?;
        let parts: Vec<String> = rel.components().map(|c| url_part(&c.as_os_str().to_string_lossy())).collect();
        (!parts.is_empty() && !l.url.is_empty()).then(|| format!("{}/{}", l.url.trim_end_matches('/'), parts.join("/")))
    })
}

fn url_part(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The files the link folders' URLs really serve (HEAD 200, same size), by page name. The rest get copied.
fn linked_urls(files: &[(PathBuf, String)], links: &[config::ShareLink]) -> std::collections::HashMap<String, String> {
    let todo: Vec<(&String, String, u64)> = files
        .iter()
        .filter_map(|(src, name)| Some((name, link_url(src, links)?, std::fs::metadata(src).ok()?.len())))
        .collect();
    let out = std::sync::Mutex::new(std::collections::HashMap::new());
    std::thread::scope(|s| {
        for chunk in todo.chunks(todo.len().div_ceil(12).max(1)) {
            let out = &out;
            s.spawn(move || {
                for (name, url, size) in chunk {
                    if remote_size(url) == Some(*size) {
                        out.lock().unwrap().insert((*name).clone(), url.clone());
                    }
                }
            });
        }
    });
    out.into_inner().unwrap()
}

/// The size a URL serves, from a HEAD request.
fn remote_size(url: &str) -> Option<u64> {
    let out = Command::new("curl").args(["-sfIL", "--max-time", "10", "-o", "/dev/null", "-w", "%header{content-length}", url]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().parse().ok()).flatten()
}

fn find_str(v: &Value, key: &str) -> Option<String> {
    match v {
        Value::Object(m) => m.get(key).and_then(Value::as_str).map(str::to_owned).or_else(|| m.values().find_map(|x| find_str(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_str(x, key)),
        _ => None,
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A waveform for the page: `n` peak heights, 0 to 1.
pub fn peaks(env: &[[f32; 3]], n: usize) -> Vec<f32> {
    if env.is_empty() {
        return Vec::new();
    }
    let n = n.min(env.len());
    (0..n)
        .map(|i| {
            let (a, b) = (i * env.len() / n, ((i + 1) * env.len() / n).max(i * env.len() / n + 1));
            let p = env[a..b].iter().map(|c| c[0].abs().max(c[1].abs())).fold(0.0f32, f32::max);
            (p * 1000.0).round() / 1000.0
        })
        .collect()
}

/// Metadata safe to publish: no local paths.
pub fn public_meta(pairs: impl IntoIterator<Item = (String, String)>) -> serde_json::Map<String, Value> {
    pairs.into_iter().filter(|(_, v)| !(v.starts_with('/') || v.starts_with("~/") || v.contains("/Users/") || v.starts_with("ref:"))).map(|(k, v)| (k, Value::String(v))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_url_safe_and_prefixed() {
        let s = slug("adeck-", "Whoosh: v1 vs v2 (final!)");
        assert!(s.starts_with("adeck-whoosh-v1-vs-v2-final-"), "{s}");
        assert!(s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "{s}");
    }

    #[test]
    fn srcs_point_where_the_audio_went_and_paths_stay_home() {
        let mut v = serde_json::json!({"groups": [{"members": [{"src": "m0"}, {"src": null}]}]});
        let names: serde_json::Map<String, Value> = [("m0".to_owned(), Value::String("m0.wav".into()))].into_iter().collect();
        rewrite_srcs(&mut v, &names, "https://cdn/x/");
        assert_eq!(v["groups"][0]["members"][0]["src"], "https://cdn/x/m0.wav");
        let names: serde_json::Map<String, Value> = [("m0".to_owned(), Value::String("https://cdn/runs/a.mp3".into()))].into_iter().collect();
        let mut v = serde_json::json!({"src": "m0"});
        rewrite_srcs(&mut v, &names, "audio/");
        assert_eq!(v["src"], "https://cdn/runs/a.mp3");
        let m = public_meta([("prompt".into(), "soft".into()), ("source".into(), "/Users/me/a.wav".into()), ("ref".into(), "ref:kick/a.wav".into())]);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn link_folders_map_files_to_their_public_url() {
        let links = [config::ShareLink { dir: "/no/such/root/run-raw".into(), url: "https://cdn.example/runs/run/music/".into() }];
        assert_eq!(link_url(Path::new("/no/such/root/run-raw/a b.mp3"), &links).as_deref(), Some("https://cdn.example/runs/run/music/a%20b.mp3"));
        assert_eq!(link_url(Path::new("/no/such/root/other/a.mp3"), &links), None);
    }

    #[test]
    fn peaks_downsample_to_the_loudest() {
        let env: Vec<[f32; 3]> = (0..100).map(|i| [-(i as f32) / 100.0, 0.0, 0.0]).collect();
        let p = peaks(&env, 10);
        assert_eq!(p.len(), 10);
        assert!((p[9] - 0.99).abs() < 1e-3);
    }
}
