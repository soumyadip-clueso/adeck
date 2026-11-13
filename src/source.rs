//! Where items come from: local run folders, the reference pack, S3, or any path.
//!
//! Ids: an absolute path; `run:<run>/<path in run>`; `ref:<path in the pack>`; `s3://b/k`.
//! A run is a folder under a run root that holds audio or video, named by its path under the
//! root (`hill/lib1`). S3 runs are the "folders" under the configured prefix, named `s3:<name>`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{self, Config};
use crate::item::{Item, media_of};

#[derive(Clone, Debug, serde::Serialize)]
pub struct Run {
    pub name: String,
    pub items: usize,
    pub source: &'static str,
}

/// The local folder a run name points at.
pub fn run_dir(cfg: &Config, run: &str) -> Option<PathBuf> {
    if run == "refs" {
        return cfg.refs_dir.clone();
    }
    if let Some(name) = run.strip_prefix("s3:") {
        return Some(s3_run_cache(name));
    }
    // A folder opened from the window is its own run.
    if Path::new(run).is_absolute() {
        return Path::new(run).is_dir().then(|| PathBuf::from(run));
    }
    cfg.run_roots.iter().map(|r| r.join(run)).find(|d| d.is_dir())
}

/// Turns an id into a local item. S3 ids download first; the caller runs this off the UI thread
/// when it might.
pub fn resolve(cfg: &Config, id: &str) -> Result<Item, String> {
    let id = id.trim();
    if let Some(rel) = id.strip_prefix("ref:") {
        let dir = cfg.refs_dir.as_ref().ok_or("No refs_dir is set in ~/.config/adeck/config.toml.")?;
        let path = dir.join(rel);
        return Item::new(id.to_owned(), path.clone(), Some("refs".into()), true).filter(|_| path.is_file()).ok_or_else(|| format!("No reference file {rel}."));
    }
    if let Some(rest) = id.strip_prefix("run:") {
        let (run, rel) = split_run(cfg, rest).ok_or_else(|| format!("No run matches {rest}."))?;
        let path = run_dir(cfg, &run).ok_or_else(|| format!("No run {run}."))?.join(&rel);
        if run.starts_with("s3:") && !path.is_file() {
            fetch_s3(cfg, &format!("{}{}/{rel}", s3_root(cfg), &run[3..]), &path)?;
        }
        return item_at(cfg, path);
    }
    if id.starts_with("s3://") {
        let path = s3_cache_path(id);
        if !path.is_file() {
            fetch_s3(cfg, id, &path)?;
        }
        let run = s3_run_of(cfg, id);
        return Item::new(id.to_owned(), path, run, false).ok_or_else(|| format!("{id} isn't audio or video adeck can play."));
    }
    let path = config::expand(Path::new(id));
    let path = if path.is_relative() && !path.is_file() { relative(cfg, &path).unwrap_or(path) } else { path };
    if !path.is_file() {
        return Err(format!("No file at {id}."));
    }
    item_at(cfg, path)
}

/// A relative path from the agent (whose working folder adeck can't see), found under a run root
/// or the folder holding one (`work/hill/x.wav` with root `…/repo/work`).
fn relative(cfg: &Config, rel: &Path) -> Option<PathBuf> {
    cfg.run_roots.iter().flat_map(|r| [Some(r.as_path()), r.parent()]).flatten().map(|d| d.join(rel)).find(|p| p.is_file())
}

/// A local file as an item, under its run or the reference pack when it's inside one.
pub fn item_at(cfg: &Config, path: PathBuf) -> Result<Item, String> {
    let path = path.canonicalize().unwrap_or(path);
    if let Some(dir) = cfg.refs_dir.as_ref().and_then(|d| d.canonicalize().ok())
        && let Ok(rel) = path.strip_prefix(&dir)
    {
        let id = format!("ref:{}", rel.display());
        return Item::new(id, path.clone(), Some("refs".into()), true).ok_or_else(|| format!("{} isn't audio or video.", path.display()));
    }
    let run = run_of(cfg, &path);
    let id = path.display().to_string();
    Item::new(id, path.clone(), run, false).ok_or_else(|| format!("{} isn't audio or video adeck can play.", path.display()))
}

/// The run a local file sits in: the shortest folder under a root, on its way, that holds media.
fn run_of(cfg: &Config, path: &Path) -> Option<String> {
    for root in &cfg.run_roots {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        if let Ok(rel) = path.strip_prefix(&root) {
            let rel = rel.display().to_string();
            return split_run(cfg, &rel).map(|(run, _)| run);
        }
    }
    let cache = s3_run_cache("");
    if let Ok(rel) = path.strip_prefix(&cache) {
        return rel.components().next().map(|c| format!("s3:{}", c.as_os_str().to_string_lossy()));
    }
    None
}

/// `hill/lib1/a/x.wav` → (`hill/lib1`, `a/x.wav`): the shortest leading folder that holds media
/// itself, as `list_runs` names runs; else the file's own folder.
fn split_run(cfg: &Config, rest: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 2 {
        return None;
    }
    if let Some(name) = parts[0].strip_prefix("s3:") {
        return Some((format!("s3:{name}"), parts[1..].join("/")));
    }
    for n in 1..parts.len() {
        let run = parts[..n].join("/");
        if run_dir(cfg, &run).is_some_and(|d| holds_media(&d)) {
            return Some((run, parts[n..].join("/")));
        }
    }
    let n = parts.len() - 1;
    Some((parts[..n].join("/"), parts[n].to_owned()))
}

fn holds_media(dir: &Path) -> bool {
    std::fs::read_dir(dir).map(|rd| rd.flatten().any(|e| media_of(&e.path()).is_some())).unwrap_or(false)
}

/// Every local run: folders under each root that directly hold media, plus the pack and S3.
pub fn list_runs(cfg: &Config) -> Vec<Run> {
    let mut out = Vec::new();
    for root in &cfg.run_roots {
        walk_runs(root, root, 0, &mut out);
    }
    if let Some(dir) = &cfg.refs_dir {
        let n = media_files(dir).len();
        if n > 0 {
            out.push(Run { name: "refs".into(), items: n, source: "refs" });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn walk_runs(root: &Path, dir: &Path, depth: usize, out: &mut Vec<Run>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut direct = 0;
    let mut subdirs = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) && !crate::stems::is_stems_dir(&p) {
                subdirs.push(p);
            }
        } else if media_of(&p).is_some() {
            direct += 1;
        }
    }
    if direct > 0 && dir != root {
        let name = dir.strip_prefix(root).unwrap_or(dir).display().to_string();
        out.push(Run { name, items: media_files(dir).len(), source: "local" });
        return;
    }
    subdirs.sort();
    for d in subdirs {
        walk_runs(root, &d, depth + 1, out);
    }
}

/// Every media file under `dir`, sorted, sub-folders included.
pub fn media_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            // A track's stems are parts of it, not takes of their own.
            if p.is_dir() {
                if !crate::stems::is_stems_dir(&p) {
                    stack.push(p);
                }
            } else if media_of(&p).is_some() {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// A run's items. S3 runs list from the bucket and bring down only the metadata JSON; audio
/// comes down when an item opens.
pub fn list_items(cfg: &Config, run: &str) -> Result<Vec<Item>, String> {
    if let Some(name) = run.strip_prefix("s3:") {
        return list_s3_items(cfg, name);
    }
    let dir = run_dir(cfg, run).ok_or_else(|| format!("No run named {run}. list_runs shows them."))?;
    let is_ref = run == "refs";
    Ok(media_files(&dir)
        .into_iter()
        .filter_map(|p| {
            if is_ref {
                let rel = p.strip_prefix(&dir).ok()?.display().to_string();
                Item::new(format!("ref:{rel}"), p, Some("refs".into()), true)
            } else {
                Item::new(p.display().to_string(), p, Some(run.to_owned()), false)
            }
        })
        .collect())
}

// S3, through the aws CLI: no SDK, the profile and SSO session the person already has.

fn s3_root(cfg: &Config) -> String {
    let p = cfg.s3.prefix.trim_matches('/');
    if p.is_empty() { format!("s3://{}/", cfg.s3.bucket) } else { format!("s3://{}/{p}/", cfg.s3.bucket) }
}

fn s3_run_cache(name: &str) -> PathBuf {
    config::cache_dir().join("s3-runs").join(name)
}

fn s3_cache_path(url: &str) -> PathBuf {
    config::cache_dir().join("s3").join(url.trim_start_matches("s3://"))
}

fn s3_run_of(cfg: &Config, url: &str) -> Option<String> {
    let rest = url.strip_prefix(&s3_root(cfg))?;
    rest.split('/').next().filter(|s| !s.is_empty()).map(|s| format!("s3:{s}"))
}

fn aws(cfg: &Config, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new(&cfg.aws);
    cmd.args(args);
    if !cfg.s3.profile.is_empty() {
        cmd.args(["--profile", &cfg.s3.profile]);
    }
    if !cfg.s3.region.is_empty() {
        cmd.args(["--region", &cfg.s3.region]);
    }
    let out = cmd.output().map_err(|e| format!("Couldn't run {}: {e}", cfg.aws))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    Err(aws_error(cfg, &err))
}

/// An aws CLI failure in words a person can act on: an expired session says how to renew it.
pub fn aws_error(cfg: &Config, stderr: &str) -> String {
    let lower = stderr.to_ascii_lowercase();
    if ["token has expired", "expiredtoken", "sso session", "token is expired", "the sso session associated"].iter().any(|m| lower.contains(m)) {
        return format!("The AWS SSO session has expired: {}.", cfg.s3.hint());
    }
    let line = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
    format!("aws failed: {}", line.trim())
}

fn fetch_s3(cfg: &Config, url: &str, to: &Path) -> Result<(), String> {
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("Couldn't make {}: {e}", d.display()))?;
    }
    aws(cfg, &["s3", "cp", "--only-show-errors", url, &to.display().to_string()])?;
    // Its sidecar too, if there is one; missing is fine.
    if let (Some(stem), Some(json)) = (url.rsplit_once('.').map(|(s, _)| s), to.with_extension("json").to_str()) {
        let _ = aws(cfg, &["s3", "cp", "--only-show-errors", &format!("{stem}.json"), json]);
    }
    Ok(())
}

pub fn list_s3_runs(cfg: &Config) -> Result<Vec<Run>, String> {
    if !cfg.s3.enabled() {
        return Ok(Vec::new());
    }
    let out = aws(cfg, &["s3", "ls", &s3_root(cfg)])?;
    Ok(out.lines().filter_map(|l| l.trim().strip_prefix("PRE ")).map(|n| Run { name: format!("s3:{}", n.trim_end_matches('/')), items: 0, source: "s3" }).collect())
}

fn list_s3_items(cfg: &Config, name: &str) -> Result<Vec<Item>, String> {
    if !cfg.s3.enabled() {
        return Err("No S3 bucket is set in ~/.config/adeck/config.toml.".into());
    }
    let base = format!("{}{name}/", s3_root(cfg));
    let dir = s3_run_cache(name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Metadata first, in one call: every sidecar into the run's cache folder.
    aws(cfg, &["s3", "sync", "--only-show-errors", "--exclude", "*", "--include", "*.json", &base, &dir.display().to_string()])?;
    let listing = aws(cfg, &["s3", "ls", "--recursive", &base])?;
    let key_prefix = base.trim_start_matches("s3://").split_once('/').map_or("", |(_, k)| k).to_owned();
    let mut items = Vec::new();
    for line in listing.lines() {
        let Some(key) = line.split_whitespace().nth(3) else {
            continue;
        };
        let rel = key.strip_prefix(&key_prefix).unwrap_or(key);
        let local = dir.join(rel);
        if media_of(&local).is_none() {
            continue;
        }
        let id = format!("run:s3:{name}/{rel}");
        if let Some(it) = Item::new(id, local, Some(format!("s3:{name}")), false) {
            items.push(it);
        }
    }
    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expired_session_says_how_to_renew_it() {
        let mut cfg = Config::default();
        cfg.s3.expired_hint = "run `renew-me`".into();
        let msg = aws_error(&cfg, "Error when retrieving token from sso: Token has expired and refresh failed");
        assert_eq!(msg, "The AWS SSO session has expired: run `renew-me`.");
        assert!(aws_error(&cfg, "An error occurred (403)\nAccess Denied\n").contains("Access Denied"));
    }

    #[test]
    fn runs_are_folders_that_hold_media() {
        let root = std::env::temp_dir().join(format!("adeck-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("hill/run1/a")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("hill/run1/a/x.wav"), b"").unwrap();
        std::fs::write(root.join("hill/run1/y.wav"), b"").unwrap();
        let cfg = Config { run_roots: vec![root.clone()], ..Config::default() };
        let runs = list_runs(&cfg);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].name, "hill/run1");
        assert_eq!(runs[0].items, 2);
        assert_eq!(split_run(&cfg, "hill/run1/a/x.wav"), Some(("hill/run1".into(), "a/x.wav".into())));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn relative_paths_resolve_under_a_run_root_or_its_parent() {
        let repo = std::env::temp_dir().join(format!("adeck-rel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("work/hill/run1")).unwrap();
        std::fs::write(repo.join("work/hill/run1/x.wav"), b"").unwrap();
        let root = repo.join("work");
        let cfg = Config { run_roots: vec![root.clone()], ..Config::default() };
        let want = root.join("hill/run1/x.wav").canonicalize().unwrap();
        assert_eq!(resolve(&cfg, "hill/run1/x.wav").unwrap().path, want);
        assert_eq!(resolve(&cfg, "work/hill/run1/x.wav").unwrap().path, want);
        assert!(resolve(&cfg, "work/hill/run1/missing.wav").unwrap_err().starts_with("No file at"));
        let _ = std::fs::remove_dir_all(&repo);
    }
}
