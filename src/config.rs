//! `~/.config/adeck/config.toml`: where runs live, the reference pack, the S3 bucket and the
//! tools adeck shells out to. Nothing project-specific is compiled in; a missing file means an
//! app with no runs that still opens any file path it's given.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The MCP server's port on 127.0.0.1.
    pub port: u16,
    /// Folders whose sub-folders holding audio are runs, named by their path under the root.
    pub run_roots: Vec<PathBuf>,
    /// A local-only reference pack. Its items are `ref:<path under this folder>`.
    pub refs_dir: Option<PathBuf>,
    /// The earshot binary.
    pub earshot: String,
    /// The aws CLI.
    pub aws: String,
    pub s3: S3,
    pub share: ShareCfg,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct S3 {
    pub profile: String,
    pub bucket: String,
    /// The prefix runs sit under, e.g. `some/runs/`. Each "folder" below it is a run.
    pub prefix: String,
    pub region: String,
    /// What to tell a person whose SSO session has expired.
    pub expired_hint: String,
}

/// Sharing a tab as a page (see `share.rs`).
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ShareCfg {
    /// The publisher program: `publish <dir> <slug> <title>`, `remove <slug>`, `list`.
    pub command: String,
    /// Starts every share's slug, so they're easy to spot wherever they're hosted.
    pub prefix: String,
    /// `bundle` (audio inside the page) or `s3`.
    pub audio: String,
    pub s3: ShareS3,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ShareS3 {
    pub bucket: String,
    pub prefix: String,
    pub profile: String,
    pub region: String,
    /// The URL that serves the bucket's objects: `<public_url>/<key>`.
    pub public_url: String,
}

impl Default for ShareCfg {
    fn default() -> Self {
        ShareCfg { command: String::new(), prefix: "adeck-".into(), audio: "bundle".into(), s3: ShareS3::default() }
    }
}

impl ShareCfg {
    pub fn command(&self) -> Option<PathBuf> {
        (!self.command.is_empty()).then(|| expand(Path::new(&self.command)))
    }
}

impl Default for Config {
    fn default() -> Self {
        Config { port: 7717, run_roots: Vec::new(), refs_dir: None, earshot: "earshot".into(), aws: "aws".into(), s3: S3::default(), share: ShareCfg::default() }
    }
}

impl S3 {
    pub fn enabled(&self) -> bool {
        !self.bucket.is_empty()
    }
    pub fn hint(&self) -> &str {
        if self.expired_hint.is_empty() { "run `aws sso login`" } else { &self.expired_hint }
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}

/// `~/.config/adeck`, or `ADECK_HOME` when set (tests, headless shots).
pub fn dir() -> PathBuf {
    std::env::var_os("ADECK_HOME").map_or_else(|| home().join(".config/adeck"), PathBuf::from)
}

/// `~/Library/Caches/adeck`: decoded analysis, spectrograms, earshot output, S3 downloads.
pub fn cache_dir() -> PathBuf {
    std::env::var_os("ADECK_CACHE").map_or_else(|| home().join("Library/Caches/adeck"), PathBuf::from)
}

pub fn expand(p: &Path) -> PathBuf {
    match p.to_str() {
        Some("~") => home(),
        Some(s) if s.starts_with("~/") => home().join(&s[2..]),
        _ => p.to_path_buf(),
    }
}

impl Config {
    pub fn load() -> (Config, Option<String>) {
        let path = dir().join("config.toml");
        let Ok(src) = std::fs::read_to_string(&path) else {
            return (Config::default(), None);
        };
        match toml::from_str::<Config>(&src) {
            Ok(mut c) => {
                c.run_roots = c.run_roots.iter().map(|p| expand(p)).collect();
                c.refs_dir = c.refs_dir.as_deref().map(expand);
                (c, None)
            }
            Err(e) => (Config::default(), Some(format!("{} has a mistake, so adeck started with defaults: {e}", path.display()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_file_keeps_the_other_defaults() {
        let c: Config = toml::from_str("run_roots = [\"~/runs\"]\n[s3]\nbucket = \"b\"").unwrap();
        assert_eq!(c.port, 7717);
        assert_eq!(c.earshot, "earshot");
        assert!(c.s3.enabled());
        assert_eq!(c.s3.hint(), "run `aws sso login`");
    }
}
