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
    /// Members' stems, four per track in order, named like `files`.
    pub stems: Vec<(PathBuf, String)>,
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
        if let Some(facts) = facts(src) {
            fill(&mut page, name, &facts);
        }
    }
    // Stems, a track's four at a time, until the page carries `stems_mb` of them; the rest of
    // the tracks go without.
    let mut stem_bytes = 0u64;
    let mut over = false;
    for set in x.stems.chunks(4) {
        if !over {
            let mut made = Vec::new();
            for (src, name) in set {
                let (out_name, out) = web_stem(src, name, &dir.join("audio"))?;
                stem_bytes += std::fs::metadata(&out).map_or(0, |m| m.len());
                made.push((name.clone(), out_name, out));
            }
            if stem_bytes <= cfg.stems_mb << 20 {
                for (name, out_name, _) in made {
                    if let Some(f) = facts_light(&set.iter().find(|(_, n)| *n == name).map(|(p, _)| p.clone()).unwrap_or_default()) {
                        fill(&mut page, &name, &f);
                    }
                    names.insert(name, Value::String(out_name));
                }
                continue;
            }
            over = true;
            for (_, _, out) in made {
                let _ = std::fs::remove_file(out);
            }
            eprintln!("adeck: stems past {} MB left off the page", cfg.stems_mb);
        }
        if let Some((_, first)) = set.first() {
            drop_stems(&mut page, first);
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

/// Length, loudness, the waveform, the bars and the spectrogram of one file, from the analysis
/// cache (or a fresh decode), so the page draws the same LED screens the app does.
fn facts(src: &Path) -> Option<Value> {
    let a = crate::derive::analyse(src).ok()?;
    let mut facts = serde_json::json!({"secs": a.secs, "lufs": a.lufs, "peaks": peaks(&a.envelope, 700), "rms": rms(&a.envelope, 700)});
    let bars: Vec<f64> = a.beats.iter().filter(|(_, down)| *down).map(|(t, _)| (t * 1000.0).round() / 1000.0).collect();
    if !bars.is_empty() {
        facts["bars"] = serde_json::json!(bars);
    }
    if let Some(raw) = crate::derive::cached_spec(src, &a) {
        let (cols, rows, grid) = spec_grid(&raw, a.spec_w as usize, a.spec_h as usize, 480, 48);
        use base64::Engine as _;
        facts["spec"] = serde_json::json!({"cols": cols, "rows": rows, "nyq": a.rate / 2, "data": base64::engine::general_purpose::STANDARD.encode(grid)});
    }
    Some(facts)
}

/// A stem's waveform and length for the page: no spectrogram or bar lines, which the track
/// already has.
fn facts_light(src: &Path) -> Option<Value> {
    let a = crate::derive::analyse(src).ok()?;
    Some(serde_json::json!({"secs": a.secs, "peaks": peaks(&a.envelope, 700), "rms": rms(&a.envelope, 700)}))
}

/// Takes the stems off the member whose first stem is `first`.
fn drop_stems(v: &mut Value, first: &str) {
    match v {
        Value::Object(m) => {
            if m.get("stems").and_then(|s| s.get(0)).and_then(|s| s.get("src")).and_then(Value::as_str) == Some(first) {
                m.remove("stems");
                m.remove("stem_model");
                return;
            }
            m.values_mut().for_each(|x| drop_stems(x, first));
        }
        Value::Array(a) => a.iter_mut().for_each(|x| drop_stems(x, first)),
        _ => {}
    }
}

/// A stem as VBR MP3 (about 130 kbps; a silent stem costs next to nothing). Every stem goes
/// through the same encoder, so their start padding matches and they stay in step.
fn web_stem(src: &Path, name: &str, dir: &Path) -> Result<(String, PathBuf), String> {
    let out_name = format!("{name}.mp3");
    let out = dir.join(&out_name);
    let ok = Command::new(ffmpeg())
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(src)
        .args(["-vn", "-c:a", "libmp3lame", "-q:a", "5"])
        .arg(&out)
        .output()
        .is_ok_and(|o| o.status.success());
    if ok { Ok((out_name, out)) } else { Err(format!("Couldn't encode the stem {} for the web (needs ffmpeg).", src.display())) }
}

/// ffmpeg: on the PATH, or Homebrew's when the app was opened from the Dock.
fn ffmpeg() -> PathBuf {
    ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"].iter().map(PathBuf::from).find(|p| p.is_file()).unwrap_or_else(|| "ffmpeg".into())
}

/// Puts a published share on this build's page: its data is read back from the copy kept in the
/// cache, each file's screens are worked out again from the audio (the bundled copy, or the
/// local file behind a `[[share.links]]` URL), and it's published again under the same slug, so
/// the link and everyone's votes stay. Returns how many files got fresh screens.
pub fn refresh(sh: &Share, cfg: &ShareCfg) -> Result<usize, String> {
    let command = cfg.command().ok_or("Sharing isn't set up.")?;
    let dir = config::cache_dir().join("shares").join(&sh.slug);
    let html = std::fs::read_to_string(dir.join("index.html")).map_err(|e| format!("No local copy of {} to rebuild from: {e}", sh.slug))?;
    let (open, close) = (r#"<script id="data" type="application/json">"#, "</script>");
    let at = html.find(open).ok_or("The cached page has no data in it.")? + open.len();
    let end = at + html[at..].find(close).ok_or("The cached page's data never ends.")?;
    let mut page: Value = serde_json::from_str(&html[at..end].replace("<\\/", "</")).map_err(|e| format!("The cached page's data doesn't parse: {e}"))?;
    let mut fresh = 0;
    refill(&mut page, &dir, &cfg.links, &mut fresh);
    let json = serde_json::to_string(&page).map_err(|e| e.to_string())?.replace("</", "<\\/");
    let out = PAGE.replace("__TITLE__", &html_escape(&sh.title)).replace("/*__DATA__*/null", &json);
    std::fs::write(dir.join("index.html"), out).map_err(|e| e.to_string())?;
    run(&command, &["publish", &dir.display().to_string(), &sh.slug, &sh.title])?;
    Ok(fresh)
}

/// Adds fresh facts to every member whose audio can still be found on this machine.
fn refill(v: &mut Value, dir: &Path, links: &[config::ShareLink], fresh: &mut usize) {
    match v {
        Value::Object(m) => {
            let light = m.contains_key("stem");
            if let Some(src) = m.get("src").and_then(Value::as_str)
                && let Some(path) = local_for(src, dir, links)
                && let Some(Value::Object(f)) = if light { facts_light(&path) } else { facts(&path) }
            {
                m.extend(f);
                *fresh += 1;
            }
            for (_, x) in m.iter_mut() {
                refill(x, dir, links, fresh);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| refill(x, dir, links, fresh)),
        _ => {}
    }
}

/// Where a page's `src` lives on this machine: beside the page when it was bundled, or under the
/// link folder whose URL serves it.
fn local_for(src: &str, dir: &Path, links: &[config::ShareLink]) -> Option<PathBuf> {
    if !(src.starts_with("https://") || src.starts_with("http://")) {
        let p = dir.join(src);
        return p.is_file().then_some(p);
    }
    // Several folders can sit behind one URL: the first that holds the file wins.
    links.iter().find_map(|l| {
        let base = format!("{}/", l.url.trim_end_matches('/'));
        let rest = src.strip_prefix(&base)?;
        let p = rest.split('/').fold(config::expand(&l.dir), |p, part| p.join(url_decode(part)));
        p.is_file().then_some(p)
    })
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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
    let todo: Vec<(&String, String, u64)> = files.iter().filter_map(|(src, name)| Some((name, link_url(src, links)?, std::fs::metadata(src).ok()?.len()))).collect();
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

/// The loudness core for the page's waveform: `n` RMS heights, 0 to 1.
pub fn rms(env: &[[f32; 3]], n: usize) -> Vec<f32> {
    if env.is_empty() {
        return Vec::new();
    }
    let n = n.min(env.len());
    (0..n)
        .map(|i| {
            let (a, b) = (i * env.len() / n, ((i + 1) * env.len() / n).max(i * env.len() / n + 1));
            (env[a..b].iter().map(|c| c[2]).fold(0.0f32, f32::max) * 1000.0).round() / 1000.0
        })
        .collect()
}

/// The spectrogram shrunk for the page: at most `cols`×`rows` cells, each the loudest grey under
/// it (rows from the top, low frequencies at the bottom, as the cache has them).
fn spec_grid(raw: &[u8], sw: usize, sh: usize, cols: usize, rows: usize) -> (usize, usize, Vec<u8>) {
    let (cols, rows) = (cols.min(sw), rows.min(sh));
    let mut out = vec![0u8; cols * rows];
    if raw.len() < sw * sh * 4 {
        return (cols, rows, out);
    }
    for r in 0..rows {
        let (ya, yb) = (r * sh / rows, ((r + 1) * sh / rows).max(r * sh / rows + 1));
        for c in 0..cols {
            let (xa, xb) = (c * sw / cols, ((c + 1) * sw / cols).max(c * sw / cols + 1));
            out[r * cols + c] = (ya..yb).flat_map(|y| (xa..xb).map(move |x| raw[(y * sw + x) * 4])).max().unwrap_or(0);
        }
    }
    (cols, rows, out)
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
    fn link_urls_lead_back_to_their_files() {
        let root = std::env::temp_dir().join(format!("adeck-share-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("run")).unwrap();
        std::fs::write(root.join("run/a b.mp3"), b"x").unwrap();
        let links = [
            config::ShareLink { dir: root.join("other"), url: "https://cdn.example/runs/".into() },
            config::ShareLink { dir: root.join("run"), url: "https://cdn.example/runs/".into() },
        ];
        assert_eq!(local_for("https://cdn.example/runs/a%20b.mp3", Path::new("/nowhere"), &links), Some(root.join("run/a b.mp3")));
        assert_eq!(local_for("https://cdn.example/runs/missing.mp3", Path::new("/nowhere"), &links), None);
        assert_eq!(local_for("run/a b.mp3", &root, &links), Some(root.join("run/a b.mp3")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn spec_grid_keeps_the_loudest_cell() {
        // 4×2 RGBA picture, one bright pixel in the bottom-right quarter.
        let mut raw = vec![22u8; 4 * 2 * 4];
        raw[(4 + 3) * 4] = 250;
        let (c, r, g) = spec_grid(&raw, 4, 2, 2, 1);
        assert_eq!((c, r), (2, 1));
        assert_eq!(g, vec![22, 250]);
        assert_eq!(rms(&[[0.0, 0.0, 0.2], [0.0, 0.0, 0.6]], 1), vec![0.6]);
    }

    #[test]
    fn peaks_downsample_to_the_loudest() {
        let env: Vec<[f32; 3]> = (0..100).map(|i| [-(i as f32) / 100.0, 0.0, 0.0]).collect();
        let p = peaks(&env, 10);
        assert_eq!(p.len(), 10);
        assert!((p[9] - 0.99).abs() < 1e-3);
    }
}
