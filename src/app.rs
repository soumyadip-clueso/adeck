//! The app's state: tabs, the items they show, what's been worked out about each, playback,
//! verdicts, and the requests the MCP server hands in.
//!
//! Everything slow (resolving an S3 item, decoding and analysing, earshot) runs on a worker and
//! comes back through `events`, drained in `poll`. MCP tool calls arrive on their own channel
//! and are answered from `poll` too, so a tool sees exactly what the window shows.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use rune::{Pixels, TextEdit, Tx, Waker};
use rune_kit::TableState;
use rune_kit::shell::TabDragState;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::Config;
use crate::derive::{self, Analysis, Earshot};
use crate::feedback::{self, Row};
use crate::item::{Item, Media};
use crate::jobs::{JobKind, Jobs, Outcome};
use crate::mcp;
use crate::mixer::{self, Mixer};
use crate::source;

/// The loudness every member is brought to when the match is on.
pub const MATCH_LUFS: f64 = -16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabKind {
    Single,
    Compare,
    Render,
    Run,
}

/// What a tab is, as the session file and the tools see it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TabSpec {
    pub id: u64,
    pub kind: TabKind,
    #[serde(default)]
    pub items: Vec<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub blind: bool,
    #[serde(default = "yes")]
    pub loudness_match: bool,
    /// The LLM's note pinned in the tab: what to listen for.
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub run: Option<String>,
    #[serde(default)]
    pub filter: Option<String>,
    /// A blind tab's shuffle: shown position → member index.
    #[serde(default)]
    pub order: Vec<usize>,
    #[serde(default)]
    pub revealed: bool,
}

fn yes() -> bool {
    true
}

pub struct Tab {
    pub spec: TabSpec,
    /// The member you hear, by member index.
    pub active: usize,
    pub loop_secs: Option<(f64, f64)>,
    /// Run tabs: the rows, as item ids in table order.
    pub rows: Vec<String>,
    pub table: TableState,
    pub filter: TextEdit,
    pub unscored_only: bool,
    /// The feedback note being typed.
    pub note: TextEdit,
    pub error: Option<String>,
    pub loading: bool,
    /// Render tabs: one muted video per member, following the mixer's clock.
    pub videos: Vec<rune::Player>,
    /// The member a blind pick chose, by member index, once picked.
    pub picked: Option<usize>,
}

impl Tab {
    pub fn new(spec: TabSpec) -> Tab {
        let filter = TextEdit::new(spec.filter.clone().unwrap_or_default());
        Tab {
            spec,
            active: 0,
            loop_secs: None,
            rows: Vec::new(),
            table: TableState::new().multi_select(true),
            filter,
            unscored_only: false,
            note: TextEdit::new(""),
            error: None,
            loading: false,
            videos: Vec::new(),
            picked: None,
        }
    }
    pub fn id(&self) -> u64 {
        self.spec.id
    }
    pub fn is_blind_hidden(&self) -> bool {
        self.spec.blind && !self.spec.revealed
    }
    /// Members in the order they're shown.
    pub fn shown(&self) -> Vec<usize> {
        if self.spec.blind && self.spec.order.len() == self.spec.items.len() { self.spec.order.clone() } else { (0..self.spec.items.len()).collect() }
    }
    /// A member's label: A, B, C… or X, Y, Z… while blind.
    pub fn member_label(&self, member: usize) -> String {
        let pos = self.shown().iter().position(|&m| m == member).unwrap_or(member);
        let base = if self.is_blind_hidden() { b'X' } else { b'A' };
        let letter = if pos < 3 || !self.is_blind_hidden() { (base + pos as u8) as char } else { (b'P' + (pos as u8 - 3)) as char };
        letter.to_string()
    }
}

/// An item and what's known about it so far.
pub struct Entry {
    pub item: Item,
    pub analysis: Option<Analysis>,
    pub analysing: bool,
    pub failed: Option<String>,
    pub wave: Pixels,
    pub spec: Pixels,
    pub earshot: HashMap<&'static str, Value>,
    pub earshot_busy: HashSet<&'static str>,
    pub sheet: Option<Pixels>,
}

impl Entry {
    fn new(item: Item) -> Entry {
        let mut e = Entry {
            item,
            analysis: None,
            analysing: false,
            failed: None,
            wave: Pixels::new(),
            spec: Pixels::new(),
            earshot: HashMap::new(),
            earshot_busy: HashSet::new(),
            sheet: None,
        };
        for cmd in [Earshot::Describe, Earshot::Measure, Earshot::Critique] {
            if let Some(v) = derive::earshot_cached(&e.item.path, cmd) {
                e.earshot.insert(cmd.name(), v);
            }
        }
        e
    }
}

/// What workers send back.
pub enum Event {
    Resolved { tab: u64, id: String, result: Result<Item, String> },
    Analysed { id: String, result: Result<(Analysis, Vec<u8>), String> },
    Listed { tab: u64, result: Result<Vec<Item>, String> },
    Earshot { id: String, cmd: Earshot, result: Result<Value, String> },
    Sheet { id: String, result: Result<PathBuf, String> },
}

pub struct App {
    pub cfg: Config,
    pub tabs: Vec<Tab>,
    pub active: Option<u64>,
    next_tab: u64,
    pub items: HashMap<String, Entry>,
    /// Ids being resolved, so two tabs asking for one item resolve it once.
    resolving: HashSet<String>,
    pub jobs: Jobs,
    pub mixer: Mixer,
    pub feedback: feedback::Store,
    pub scores: HashMap<String, i64>,
    waker: Option<Waker>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    pub calls: Option<Receiver<mcp::Call>>,
    pub tab_drag: TabDragState,
    pub activity_open: bool,
    /// The job whose log is open in the Activity panel.
    pub log_open: Option<crate::jobs::JobId>,
    /// A message along the bottom: (text, is_error).
    pub toast: Option<(String, bool)>,
    pub mcp_status: String,
    dirty: bool,
    pub headless: bool,
}

impl App {
    pub fn new() -> App {
        let (cfg, cfg_error) = Config::load();
        let (tx, rx) = channel();
        let headless = rune::tooling::headless_run().is_some() || std::env::var_os("ADECK_HEADLESS").is_some();
        let feedback = feedback::Store::new(crate::config::dir().join("feedback.jsonl"));
        let scores = feedback::latest_scores(&feedback.read_all());
        let mut jobs = Jobs::default();
        if let Some(p) = crate::jobs::log_path() {
            jobs.log_to(&p);
        }
        let mut app = App {
            mcp_status: format!("MCP :{}", cfg.port),
            cfg,
            tabs: Vec::new(),
            active: None,
            next_tab: 1,
            items: HashMap::new(),
            resolving: HashSet::new(),
            jobs,
            mixer: Mixer::default(),
            feedback,
            scores,
            waker: None,
            tx,
            rx,
            calls: None,
            tab_drag: TabDragState::default(),
            activity_open: false,
            log_open: None,
            toast: cfg_error.map(|e| (e, true)),
            dirty: false,
            headless,
        };
        app.restore();
        if headless {
            app.settle();
        }
        app
    }

    pub fn setup(&mut self, w: Waker) {
        self.waker = Some(w.clone());
        self.jobs.set_waker(w.clone());
        self.mixer.set_waker(w.clone());
        if !self.headless {
            match mcp::serve(self.cfg.clone(), self.feedback.path.clone(), self.feedback.signal.clone(), w) {
                Ok(rx) => self.calls = Some(rx),
                Err(e) => {
                    self.mcp_status = "MCP off".into();
                    self.toast = Some((format!("The MCP server couldn't start on port {}: {e}", self.cfg.port), true));
                }
            }
        }
    }

    pub fn tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id() == id)
    }
    pub fn tab_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id() == id)
    }
    pub fn current(&self) -> Option<&Tab> {
        self.active.and_then(|a| self.tab(a))
    }
    pub fn current_mut(&mut self) -> Option<&mut Tab> {
        let a = self.active?;
        self.tab_mut(a)
    }
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.items.get(id)
    }

    // Opening tabs.

    #[allow(clippy::too_many_arguments)]
    pub fn open(&mut self, kind: TabKind, items: Vec<String>, label: Option<String>, blind: bool, loudness_match: bool, run: Option<String>, filter: Option<String>) -> u64 {
        let id = self.next_tab;
        self.next_tab += 1;
        let order = if blind { shuffle(items.len(), id) } else { Vec::new() };
        let spec = TabSpec { id, kind, items, label, blind, loudness_match, note: None, run, filter, order, revealed: false };
        self.tabs.push(Tab::new(spec));
        self.load_tab(id);
        self.focus(id);
        self.dirty = true;
        id
    }

    /// Starts resolving and analysing everything a tab shows.
    fn load_tab(&mut self, id: u64) {
        let Some(t) = self.tab(id) else { return };
        let (kind, items, run) = (t.spec.kind, t.spec.items.clone(), t.spec.run.clone());
        if kind == TabKind::Run {
            if let Some(run) = run {
                if let Some(t) = self.tab_mut(id) {
                    t.loading = true;
                }
                let cfg = self.cfg.clone();
                let tx = self.tx.clone();
                let waker = self.waker.clone();
                let work = move || {
                    let result = source::list_items(&cfg, &run);
                    let _ = tx.send(Event::Listed { tab: id, result });
                    if let Some(w) = waker {
                        w();
                    }
                };
                if self.headless {
                    work()
                } else {
                    std::thread::spawn(work);
                }
            }
            return;
        }
        for item in items {
            self.ensure(id, &item);
        }
    }

    /// Resolves an item if it's new and analyses it if it hasn't been.
    fn ensure(&mut self, tab: u64, id: &str) {
        if let Some(e) = self.items.get(id) {
            if e.analysis.is_none() && !e.analysing && e.failed.is_none() {
                self.analyse(id);
            }
            return;
        }
        if !self.resolving.insert(id.to_owned()) {
            return;
        }
        let cfg = self.cfg.clone();
        let tx = self.tx.clone();
        let waker = self.waker.clone();
        let item_id = id.to_owned();
        let slow = id.starts_with("s3://") || id.starts_with("run:s3:");
        let work = move || {
            let result = source::resolve(&cfg, &item_id);
            let _ = tx.send(Event::Resolved { tab, id: item_id, result });
            if let Some(w) = waker {
                w();
            }
        };
        if self.headless {
            work();
        } else if slow {
            let label = id.rsplit('/').next().unwrap_or(id).to_owned();
            self.jobs.spawn(JobKind::Fetch, format!("Downloading {label}"), move |_| {
                work();
                Outcome::Completed("Downloaded".into())
            });
        } else {
            std::thread::spawn(work);
        }
    }

    fn analyse(&mut self, id: &str) {
        let Some(e) = self.items.get_mut(id) else {
            return;
        };
        e.analysing = true;
        let path = e.item.path.clone();
        let tx = self.tx.clone();
        let waker = self.waker.clone();
        let item_id = id.to_owned();
        let work = move || {
            let result = derive::analyse(&path);
            let _ = tx.send(Event::Analysed { id: item_id, result });
            if let Some(w) = waker {
                w();
            }
        };
        if self.headless {
            work();
        } else {
            std::thread::spawn(work);
        }
    }

    /// Runs earshot on an item in the background; the tab shows it when it lands.
    pub fn run_earshot(&mut self, id: &str, cmd: Earshot) {
        let Some(e) = self.items.get_mut(id) else {
            return;
        };
        if e.earshot.contains_key(cmd.name()) || !e.earshot_busy.insert(cmd.name()) {
            return;
        }
        let path = e.item.path.clone();
        let bin = self.cfg.earshot.clone();
        let tx = self.tx.clone();
        let item_id = id.to_owned();
        let title = format!("earshot {} · {}", cmd.name(), e.item.name);
        self.jobs.spawn(JobKind::Earshot, title, move |_| {
            let result = derive::earshot(&bin, &path, cmd);
            let outcome = match &result {
                Ok(_) => Outcome::Completed(format!("earshot {} done", cmd.name())),
                Err(e) => Outcome::Failed(e.clone()),
            };
            let _ = tx.send(Event::Earshot { id: item_id, cmd, result });
            outcome
        });
    }

    pub fn run_sheet(&mut self, id: &str) {
        let Some(e) = self.items.get_mut(id) else {
            return;
        };
        if e.sheet.is_some() || !e.earshot_busy.insert("sheet") {
            return;
        }
        let path = e.item.path.clone();
        let bin = self.cfg.earshot.clone();
        let tx = self.tx.clone();
        let item_id = id.to_owned();
        let work = move || {
            let result = derive::sheet(&bin, &path);
            let _ = tx.send(Event::Sheet { id: item_id, result });
        };
        if self.headless {
            work();
        } else {
            let title = format!("earshot sheet · {}", e.item.name);
            self.jobs.spawn(JobKind::Earshot, title, move |_| {
                work();
                Outcome::Completed("Sheet drawn".into())
            });
        }
    }

    pub fn focus(&mut self, id: u64) {
        if self.tab(id).is_none() {
            return;
        }
        if self.active != Some(id) {
            self.mixer.pause();
        }
        self.active = Some(id);
        self.dirty = true;
        self.sync_mixer();
    }

    /// Moves a tab to `to` in the strip.
    pub fn reorder(&mut self, id: &str, to: usize) {
        let Ok(id) = id.parse::<u64>() else { return };
        let Some(from) = self.tabs.iter().position(|t| t.id() == id) else {
            return;
        };
        let t = self.tabs.remove(from);
        self.tabs.insert(to.min(self.tabs.len()), t);
        self.dirty = true;
    }

    /// Opens the Activity panel on one job's log.
    pub fn open_job(&mut self, id: crate::jobs::JobId) {
        self.activity_open = true;
        self.log_open = Some(id);
    }

    pub fn close(&mut self, id: u64) {
        let Some(i) = self.tabs.iter().position(|t| t.id() == id) else {
            return;
        };
        self.tabs.remove(i);
        if self.mixer.owner() == Some(id) {
            self.mixer.unload();
        }
        if self.active == Some(id) {
            self.active = self.tabs.get(i.min(self.tabs.len().saturating_sub(1))).map(Tab::id);
            self.sync_mixer();
        }
        self.dirty = true;
    }

    /// Puts the active tab's members on the mixer once they're all resolved.
    pub fn sync_mixer(&mut self) {
        let Some(t) = self.current() else { return };
        if t.spec.kind == TabKind::Run || self.headless {
            return;
        }
        let tab = t.id();
        let ids = t.spec.items.clone();
        let (active, loop_secs) = (t.active, t.loop_secs);
        let paths: Option<Vec<(String, PathBuf)>> = ids.iter().map(|i| self.items.get(i).map(|e| (i.clone(), e.item.path.clone()))).collect();
        let Some(paths) = paths else { return };
        if paths.is_empty() {
            return;
        }
        self.mixer.load(tab, paths);
        self.mixer.set_active(active);
        self.mixer.set_loop_secs(loop_secs);
        self.apply_trims();
    }

    /// The loudness match's trims for the active tab (or unity when it's off).
    pub fn apply_trims(&mut self) {
        let Some(t) = self.current() else { return };
        let on = t.spec.loudness_match && t.spec.items.len() > 1;
        let trims: Vec<f32> = t.spec.items.iter().map(|i| if on { mixer::trim_for(self.lufs(i), MATCH_LUFS) } else { 1.0 }).collect();
        self.mixer.set_trims(trims);
    }

    /// The loudness used for matching: ours, which is what the trims are computed from.
    pub fn lufs(&self, id: &str) -> Option<f64> {
        self.items.get(id).and_then(|e| e.analysis.as_ref()).and_then(|a| a.lufs)
    }

    /// The trim, in dB, the match applies to a member of the active tab.
    pub fn trim_db(&self, tab: &Tab, id: &str) -> Option<f64> {
        if !tab.spec.loudness_match || tab.spec.items.len() < 2 {
            return None;
        }
        self.lufs(id).map(|l| (MATCH_LUFS - l).min(12.0))
    }

    pub fn mixer_is_for(&self, tab: u64) -> bool {
        self.mixer.owner() == Some(tab)
    }

    pub fn toggle_play(&mut self) {
        self.sync_mixer();
        self.mixer.toggle();
        self.sync_videos(true);
    }

    pub fn set_active_member(&mut self, member: usize) {
        let Some(t) = self.current_mut() else { return };
        if member >= t.spec.items.len() {
            return;
        }
        t.active = member;
        self.mixer.set_active(member);
    }

    pub fn seek_secs(&mut self, s: f64) {
        self.sync_mixer();
        self.mixer.seek_secs(s);
        self.sync_videos(true);
    }

    pub fn set_loop(&mut self, l: Option<(f64, f64)>) {
        if let Some(t) = self.current_mut() {
            t.loop_secs = l;
        }
        self.mixer.set_loop_secs(l);
    }

    /// Keeps each render tab's videos on the mixer's clock: playing with it, and re-seeked when
    /// they drift more than 40 ms.
    fn sync_videos(&mut self, force: bool) {
        let Some(t) = self.current() else { return };
        if t.spec.kind != TabKind::Render || !self.mixer_is_for(t.id()) {
            return;
        }
        let pos = self.mixer.secs();
        let playing = self.mixer.is_playing();
        for v in &t.videos {
            let drift = (v.time() - pos).abs();
            if force || drift > 0.040 {
                v.seek(pos);
            }
            let state = v.state();
            let is_playing = matches!(state, rune::PlayerState::Playing);
            if playing && !is_playing {
                v.play();
            } else if !playing && is_playing {
                v.pause();
            }
        }
    }

    // Verdicts.

    pub fn score(&mut self, value: i64) {
        let Some(t) = self.current() else { return };
        let member = t.active;
        let Some(id) = t.spec.items.get(member).cloned() else {
            return;
        };
        let note = t.note.text.trim().to_owned();
        let row =
            Row { ts: feedback::now(), tab_id: t.id(), kind: "score".into(), items: vec![id.clone()], value: Value::from(value), note, blind: t.spec.blind, reveal_map: None };
        if self.write(row) {
            self.scores.insert(id, value);
            if let Some(t) = self.current_mut() {
                t.note = TextEdit::new("");
            }
        }
    }

    /// Picks the member you're hearing as the better one. A blind tab reveals itself.
    pub fn pick(&mut self) {
        let Some(t) = self.current() else { return };
        if t.spec.items.len() < 2 {
            return;
        }
        let member = t.active;
        let picked = t.spec.items[member].clone();
        let reveal = t.spec.blind.then(|| {
            let map: serde_json::Map<String, Value> = t.spec.items.iter().enumerate().map(|(m, id)| (t.member_label(m), Value::String(id.clone()))).collect();
            Value::Object(map)
        });
        let row = Row {
            ts: feedback::now(),
            tab_id: t.id(),
            kind: "pick".into(),
            items: t.spec.items.clone(),
            value: Value::String(picked),
            note: t.note.text.trim().to_owned(),
            blind: t.spec.blind,
            reveal_map: reveal,
        };
        if self.write(row)
            && let Some(t) = self.current_mut()
        {
            t.picked = Some(member);
            t.spec.revealed = true;
            t.note = TextEdit::new("");
            self.dirty = true;
        }
    }

    /// A note with no score or pick.
    pub fn note_only(&mut self) {
        let Some(t) = self.current() else { return };
        let note = t.note.text.trim().to_owned();
        if note.is_empty() {
            return;
        }
        let items = if t.spec.kind == TabKind::Run { t.table.selected().map(str::to_owned).collect() } else { t.spec.items.clone() };
        let row = Row { ts: feedback::now(), tab_id: t.id(), kind: "note".into(), items, value: Value::Null, note, blind: t.spec.blind, reveal_map: None };
        if self.write(row)
            && let Some(t) = self.current_mut()
        {
            t.note = TextEdit::new("");
        }
    }

    fn write(&mut self, row: Row) -> bool {
        let run_dir = self.common_run_dir(&row.items);
        match self.feedback.append(&row, run_dir) {
            Ok(()) => {
                self.toast = Some((format!("Saved {}", row.kind), false));
                true
            }
            Err(e) => {
                self.toast = Some((e, true));
                false
            }
        }
    }

    /// The folder of the one local run all these items share, if they share one.
    fn common_run_dir(&self, ids: &[String]) -> Option<PathBuf> {
        let runs: HashSet<&str> = ids.iter().filter_map(|i| self.items.get(i)).filter(|e| !e.item.is_ref).filter_map(|e| e.item.run.as_deref()).collect();
        if runs.len() != 1 {
            return None;
        }
        let run = *runs.iter().next()?;
        if run.starts_with("s3:") {
            return None;
        }
        source::run_dir(&self.cfg, run)
    }

    // Run tabs.

    /// The rows a run tab shows after its filter.
    pub fn refilter(&mut self, tab: u64) {
        let Some(t) = self.tab(tab) else { return };
        let needle = t.filter.text.trim().to_ascii_lowercase();
        let unscored = t.unscored_only;
        let run = t.spec.run.clone();
        let mut rows: Vec<String> = self
            .items
            .values()
            .filter(|e| e.item.run.as_deref() == run.as_deref())
            .filter(|e| !unscored || !self.scores.contains_key(&e.item.id))
            .filter(|e| needle.is_empty() || e.item.name.to_ascii_lowercase().contains(&needle) || e.item.meta.iter().any(|(_, v)| v.to_ascii_lowercase().contains(&needle)))
            .map(|e| e.item.id.clone())
            .collect();
        rows.sort();
        let sort = t.table.sort().map(|(c, d)| (c.to_owned(), d));
        if let Some((col, dir)) = sort {
            rows.sort_by(|a, b| {
                let (ea, eb) = (&self.items[a], &self.items[b]);
                let (va, vb) = (crate::ui::run::cell_value(self, ea, &col), crate::ui::run::cell_value(self, eb, &col));
                let o = match (va.parse::<f64>(), vb.parse::<f64>()) {
                    (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                    _ => va.cmp(&vb),
                };
                dir.apply(o)
            });
        }
        let filter_text = t.filter.text.clone();
        if let Some(t) = self.tab_mut(tab) {
            t.rows = rows;
            t.spec.filter = (!filter_text.is_empty()).then_some(filter_text);
        }
        self.dirty = true;
    }

    /// Opens what's selected in a run tab: one item alone, several side by side.
    pub fn open_selection(&mut self) {
        let Some(t) = self.current() else { return };
        let mut ids: Vec<String> = t.table.selected().map(str::to_owned).collect();
        ids.sort_by_key(|i| t.rows.iter().position(|r| r == i));
        match ids.len() {
            0 => {}
            1 => {
                self.open(TabKind::Single, ids, None, false, true, None, None);
            }
            _ => {
                ids.truncate(9);
                self.open(TabKind::Compare, ids, None, false, true, None, None);
            }
        }
    }

    /// Files dropped on the window: one opens alone, several side by side.
    pub fn open_files(&mut self, files: Vec<PathBuf>) {
        let ids: Vec<String> = files.into_iter().filter(|p| crate::item::media_of(p).is_some()).map(|p| p.display().to_string()).collect();
        let videos = ids.iter().all(|i| crate::item::media_of(std::path::Path::new(i)) == Some(Media::Video));
        match ids.len() {
            0 => self.toast = Some(("adeck opens audio and video files.".into(), true)),
            _ if videos => {
                self.open(TabKind::Render, ids, None, false, true, None, None);
            }
            1 => {
                self.open(TabKind::Single, ids, None, false, true, None, None);
            }
            _ => {
                self.open(TabKind::Compare, ids.into_iter().take(9).collect(), None, false, true, None, None);
            }
        }
    }

    // The frame.

    pub fn poll(&mut self, clock: f64, tx: &mut Tx) -> bool {
        let mut changed = self.drain();
        let (_, jobs_changed) = self.jobs.drain(clock);
        changed |= jobs_changed;
        changed |= self.mixer.tick();
        if let Some(rx) = self.calls.take() {
            while let Ok(call) = rx.try_recv() {
                let result = self.call(&call.tool, &call.args);
                let _ = call.reply.send(result);
                changed = true;
            }
            self.calls = Some(rx);
        }
        if self.mixer.is_playing() || self.mixer.is_loading() {
            self.sync_videos(false);
            tx.wake_after(std::time::Duration::from_millis(33));
            changed = true;
        }
        if self.dirty {
            self.dirty = false;
            self.save();
        }
        changed
    }

    fn drain(&mut self) -> bool {
        let mut changed = false;
        while let Ok(e) = self.rx.try_recv() {
            changed = true;
            self.apply(e);
        }
        changed
    }

    fn apply(&mut self, e: Event) {
        match e {
            Event::Resolved { tab, id, result } => {
                self.resolving.remove(&id);
                match result {
                    Ok(item) => {
                        self.items.insert(id.clone(), Entry::new(item));
                        self.analyse(&id);
                        if self.tab(tab).is_some_and(|t| t.spec.kind == TabKind::Render) {
                            self.make_videos(tab);
                        }
                        if self.active == Some(tab) {
                            self.sync_mixer();
                        }
                    }
                    Err(e) => {
                        if let Some(t) = self.tab_mut(tab) {
                            t.error = Some(e.clone());
                        }
                        self.toast = Some((e, true));
                    }
                }
            }
            Event::Analysed { id, result } => {
                let Some(entry) = self.items.get_mut(&id) else {
                    return;
                };
                entry.analysing = false;
                match result {
                    Ok((a, spec)) => {
                        let (w, h) = (2000usize, 160usize);
                        entry.wave.set(w as u32, h as u32, crate::ui::widgets::paint_wave(&a.envelope, w, h));
                        entry.spec.set(a.spec_w, a.spec_h, spec);
                        entry.analysis = Some(a);
                        if self.current().is_some_and(|t| t.spec.items.contains(&id)) {
                            self.apply_trims();
                        }
                    }
                    Err(e) => entry.failed = Some(e),
                }
            }
            Event::Listed { tab, result } => match result {
                Ok(items) => {
                    for it in items {
                        if !self.items.contains_key(&it.id) {
                            let id = it.id.clone();
                            self.items.insert(id, Entry::new(it));
                        }
                    }
                    if let Some(t) = self.tab_mut(tab) {
                        t.loading = false;
                    }
                    self.refilter(tab);
                }
                Err(e) => {
                    if let Some(t) = self.tab_mut(tab) {
                        t.loading = false;
                        t.error = Some(e.clone());
                    }
                    self.toast = Some((e, true));
                }
            },
            Event::Earshot { id, cmd, result } => {
                let Some(entry) = self.items.get_mut(&id) else {
                    return;
                };
                entry.earshot_busy.remove(cmd.name());
                match result {
                    Ok(v) => {
                        entry.earshot.insert(cmd.name(), v);
                    }
                    Err(e) => self.toast = Some((e, true)),
                }
            }
            Event::Sheet { id, result } => {
                let Some(entry) = self.items.get_mut(&id) else {
                    return;
                };
                entry.earshot_busy.remove("sheet");
                match result.and_then(|p| derive::read_png(&p).ok_or_else(|| "The sheet PNG didn't decode.".to_owned())) {
                    Ok((w, h, rgba)) => {
                        let px = Pixels::new();
                        px.set(w, h, rgba);
                        entry.sheet = Some(px);
                    }
                    Err(e) => self.toast = Some((e, true)),
                }
            }
        }
    }

    fn make_videos(&mut self, tab: u64) {
        let paths: Vec<Option<PathBuf>> = match self.tab(tab) {
            Some(t) => t.spec.items.iter().map(|i| self.items.get(i).map(|e| e.item.path.clone())).collect(),
            None => return,
        };
        if paths.iter().any(Option::is_none) {
            return;
        }
        let videos: Vec<rune::Player> = paths
            .into_iter()
            .flatten()
            .map(|p| {
                let v = rune::Player::file(p.display().to_string());
                v.set_muted(true);
                v
            })
            .collect();
        if let Some(t) = self.tab_mut(tab) {
            t.videos = videos;
        }
    }

    /// In a headless run, waits for every worker so the first frame is complete.
    fn settle(&mut self) {
        for _ in 0..4 {
            self.drain();
        }
    }

    /// `settle`, for scenes and tests.
    pub fn settle_now(&mut self) {
        self.settle();
    }

    /// Blind on: shuffle and hide. Off (only before a pick): show names again.
    pub fn toggle_blind(&mut self) {
        let Some(t) = self.current_mut() else { return };
        if t.spec.revealed {
            return;
        }
        t.spec.blind ^= true;
        if t.spec.blind {
            t.spec.order = shuffle(t.spec.items.len(), t.spec.id);
        }
        self.dirty = true;
    }

    pub fn toggle_loudness_match(&mut self) {
        if let Some(t) = self.current_mut() {
            t.spec.loudness_match ^= true;
        }
        self.apply_trims();
        self.dirty = true;
    }

    // The session: every tab's spec and the active one, saved on each change.

    fn session_path() -> PathBuf {
        std::env::var_os("ADECK_SESSION").map_or_else(|| crate::config::dir().join("session.json"), PathBuf::from)
    }

    pub fn save(&self) {
        if self.headless {
            return;
        }
        let specs: Vec<&TabSpec> = self.tabs.iter().map(|t| &t.spec).collect();
        let v = json!({ "tabs": specs, "active": self.active, "next_tab": self.next_tab });
        let path = App::session_path();
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&v).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    fn restore(&mut self) {
        let Ok(bytes) = std::fs::read(App::session_path()) else {
            return;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            return;
        };
        let specs: Vec<TabSpec> = serde_json::from_value(v["tabs"].clone()).unwrap_or_default();
        self.next_tab = v["next_tab"].as_u64().unwrap_or(1).max(specs.iter().map(|s| s.id + 1).max().unwrap_or(1));
        // A headless shot of one tab loads only that tab.
        let focus: Option<u64> = std::env::var("ADECK_FOCUS").ok().and_then(|s| s.parse().ok());
        for spec in specs {
            if self.headless && focus.is_some_and(|f| f != spec.id) {
                continue;
            }
            let id = spec.id;
            self.tabs.push(Tab::new(spec));
            self.load_tab(id);
        }
        self.active = focus.or_else(|| v["active"].as_u64()).filter(|a| self.tab(*a).is_some()).or_else(|| self.tabs.first().map(Tab::id));
    }

    // Tools: what the MCP server asks for, answered on the UI thread.

    pub fn call(&mut self, tool: &str, a: &Value) -> Result<Value, String> {
        let s = |k: &str| a.get(k).and_then(Value::as_str).map(str::to_owned);
        let ids = |k: &str| -> Vec<String> { a.get(k).and_then(Value::as_array).map(|xs| xs.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default() };
        let tab_arg = || a.get("tab_id").and_then(Value::as_u64);
        match tool {
            "list_runs" => {
                let mut runs = source::list_runs(&self.cfg);
                if a.get("include_s3").and_then(Value::as_bool).unwrap_or(false) {
                    runs.extend(source::list_s3_runs(&self.cfg)?);
                }
                Ok(json!({ "runs": runs }))
            }
            "list_items" => {
                let run = s("run").ok_or("run is required")?;
                let filter = s("filter").map(|f| f.to_ascii_lowercase());
                let items = source::list_items(&self.cfg, &run)?;
                let rows: Vec<Value> = items
                    .iter()
                    .filter(|i| filter.as_ref().is_none_or(|f| i.name.to_ascii_lowercase().contains(f) || i.meta.iter().any(|(_, v)| v.to_ascii_lowercase().contains(f))))
                    .map(|i| json!({ "id": i.id, "name": i.name, "summary": i.subtitle(), "score": self.scores.get(&i.id) }))
                    .collect();
                Ok(json!({ "run": run, "count": rows.len(), "items": rows }))
            }
            "get_item" => {
                let id = s("id").ok_or("id is required")?;
                let item = match self.items.get(&id) {
                    Some(e) => e.item.clone(),
                    None => source::resolve(&self.cfg, &id)?,
                };
                let analysis = self.items.get(&id).and_then(|e| e.analysis.clone()).or_else(|| derive::analyse(&item.path).ok().map(|(a, _)| a));
                let earshot: serde_json::Map<String, Value> = [Earshot::Describe, Earshot::Measure, Earshot::Critique]
                    .into_iter()
                    .filter_map(|c| derive::earshot_cached(&item.path, c).map(|v| (c.name().to_owned(), summarise_earshot(c, &v))))
                    .collect();
                let mut v = item.to_json();
                v["analysis"] = analysis.map_or(Value::Null, |a| a.to_json());
                v["earshot"] = Value::Object(earshot);
                v["score"] = self.scores.get(&id).map_or(Value::Null, |s| Value::from(*s));
                Ok(v)
            }
            "open_single" => {
                let item = s("item").ok_or("item is required")?;
                let id = self.open(TabKind::Single, vec![item], s("label"), false, true, None, None);
                if let Some(n) = s("note") {
                    self.annotate(id, n);
                }
                Ok(self.tab_json(id))
            }
            "open_compare" => {
                let items = ids("items");
                if items.len() < 2 {
                    return Err("open_compare needs at least 2 items.".into());
                }
                if items.len() > 9 {
                    return Err("open_compare takes at most 9 items (keys 1-9 switch).".into());
                }
                let blind = a.get("blind").and_then(Value::as_bool).unwrap_or(false);
                let lm = a.get("loudness_match").and_then(Value::as_bool).unwrap_or(true);
                let id = self.open(TabKind::Compare, items, s("label"), blind, lm, None, None);
                if let Some(n) = s("note") {
                    self.annotate(id, n);
                }
                Ok(self.tab_json(id))
            }
            "open_render" => {
                let items = ids("items");
                if items.is_empty() {
                    return Err("open_render needs at least 1 item.".into());
                }
                let id = self.open(TabKind::Render, items, s("label"), false, a.get("loudness_match").and_then(Value::as_bool).unwrap_or(false), None, None);
                if let Some(n) = s("note") {
                    self.annotate(id, n);
                }
                Ok(self.tab_json(id))
            }
            "open_run" => {
                let run = s("run").ok_or("run is required")?;
                let label = s("label").or_else(|| Some(run.clone()));
                let id = self.open(TabKind::Run, Vec::new(), label, false, true, Some(run), s("filter"));
                Ok(self.tab_json(id))
            }
            "list_tabs" => Ok(json!({ "active": self.active, "tabs": self.tabs.iter().map(|t| self.tab_json(t.id())).collect::<Vec<_>>() })),
            "focus_tab" => {
                let id = tab_arg().ok_or("tab_id is required")?;
                self.tab(id).ok_or_else(|| format!("No tab {id}."))?;
                self.focus(id);
                Ok(self.tab_json(id))
            }
            "close_tab" => {
                let id = tab_arg().ok_or("tab_id is required")?;
                self.tab(id).ok_or_else(|| format!("No tab {id}."))?;
                self.close(id);
                Ok(json!({ "closed": id }))
            }
            "annotate_tab" => {
                let id = tab_arg().ok_or("tab_id is required")?;
                self.tab(id).ok_or_else(|| format!("No tab {id}."))?;
                self.annotate(id, s("markdown").unwrap_or_default());
                Ok(self.tab_json(id))
            }
            "play" => {
                let id = tab_arg().or(self.active).ok_or("No tab is open.")?;
                self.tab(id).ok_or_else(|| format!("No tab {id}."))?;
                self.focus(id);
                if let Some(m) = a.get("member").and_then(Value::as_u64) {
                    self.set_active_member(m as usize);
                }
                if let Some(at) = a.get("at_s").and_then(Value::as_f64) {
                    self.seek_secs(at);
                }
                self.sync_mixer();
                self.mixer.play();
                self.sync_videos(true);
                Ok(json!({ "playing": true, "tab_id": id, "loading": self.mixer.is_loading() }))
            }
            "stop" => {
                self.mixer.pause();
                self.sync_videos(true);
                Ok(json!({ "playing": false, "at_s": self.mixer.secs() }))
            }
            "set_loop" => {
                let id = tab_arg().or(self.active).ok_or("No tab is open.")?;
                self.focus(id);
                match (a.get("start_s").and_then(Value::as_f64), a.get("end_s").and_then(Value::as_f64)) {
                    (Some(x), Some(y)) if y > x => self.set_loop(Some((x, y))),
                    _ => self.set_loop(None),
                }
                Ok(self.tab_json(id))
            }
            "get_feedback" => {
                let rows = self.feedback.read_all();
                let mut items = ids("items");
                if let Some(run) = s("run") {
                    items.extend(self.items.values().filter(|e| e.item.run.as_deref() == Some(run.as_str())).map(|e| e.item.id.clone()));
                    if items.is_empty() {
                        items.extend(source::list_items(&self.cfg, &run).unwrap_or_default().into_iter().map(|i| i.id));
                    }
                }
                let since = a.get("since").and_then(Value::as_f64).unwrap_or(0.0);
                Ok(json!({ "rows": feedback::select(&rows, tab_arg(), &items, since) }))
            }
            "_item_path" => {
                let id = s("id").ok_or("id is required")?;
                let path = match self.items.get(&id) {
                    Some(e) => e.item.path.clone(),
                    None => source::resolve(&self.cfg, &id)?.path,
                };
                Ok(json!({ "path": path }))
            }
            "_earshot_done" => {
                if let (Some(id), Some(cmd)) = (s("id"), s("kind").as_deref().and_then(Earshot::parse))
                    && let (Some(e), Some(v)) = (self.items.get_mut(&id), a.get("result"))
                {
                    e.earshot.insert(cmd.name(), v.clone());
                }
                Ok(Value::Null)
            }
            "_save" => {
                self.save();
                Ok(json!({ "active": self.active }))
            }
            other => Err(format!("No tool {other}.")),
        }
    }

    pub fn annotate(&mut self, id: u64, md: String) {
        if let Some(t) = self.tab_mut(id) {
            t.spec.note = (!md.trim().is_empty()).then_some(md);
        }
        self.dirty = true;
    }

    pub fn tab_json(&self, id: u64) -> Value {
        let Some(t) = self.tab(id) else {
            return Value::Null;
        };
        let members: Vec<Value> = t
            .spec
            .items
            .iter()
            .enumerate()
            .map(|(m, i)| {
                let e = self.items.get(i);
                let hidden = t.is_blind_hidden();
                json!({
                    "member": m,
                    "label": t.member_label(m),
                    "id": if hidden { Value::Null } else { Value::String(i.clone()) },
                    "ready": e.is_some_and(|e| e.analysis.is_some()),
                    "lufs": e.and_then(|e| e.analysis.as_ref()).and_then(|a| a.lufs).map(|l| (l * 10.0).round() / 10.0),
                    "trim_db": self.trim_db(t, i).map(|d| (d * 10.0).round() / 10.0),
                })
            })
            .collect();
        json!({
            "tab_id": t.id(),
            "kind": t.spec.kind,
            "label": t.spec.label,
            "blind": t.spec.blind,
            "revealed": t.spec.revealed,
            "loudness_match": t.spec.loudness_match,
            "members": members,
            "run": t.spec.run,
            "rows": if t.spec.kind == TabKind::Run { Value::from(t.rows.len()) } else { Value::Null },
            "active_member": t.active,
            "loop": t.loop_secs,
            "note": t.spec.note,
            "error": t.error,
        })
    }

    /// The status bar's left side.
    pub fn summary(&self) -> String {
        match self.current() {
            None => "No tabs. Ask your LLM to open something, or drop files here.".into(),
            Some(t) if t.spec.kind == TabKind::Run => format!("{} items", t.rows.len()),
            Some(t) => {
                let ready = t.spec.items.iter().filter(|i| self.items.get(*i).is_some_and(|e| e.analysis.is_some())).count();
                if ready < t.spec.items.len() { format!("Analysing {}/{}…", ready, t.spec.items.len()) } else { format!("{} file{}", ready, if ready == 1 { "" } else { "s" }) }
            }
        }
    }
}

/// A fixed shuffle for a blind tab, seeded by its id so a restored session keeps it.
fn shuffle(n: usize, seed: u64) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ feedback::now().to_bits();
    for i in (1..n).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let j = (x % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

/// The few numbers of an earshot result worth handing a model.
pub fn summarise_earshot(cmd: Earshot, v: &Value) -> Value {
    match cmd {
        Earshot::Critique => json!({
            "verdict": v.get("verdict"), "score": v.get("score"),
            "critical": v.get("critical_issues"), "major": v.get("major_issues"),
            "actions": v.get("actions").and_then(Value::as_array).map(|a| a.iter().take(5).filter_map(|x| x.get("action").cloned()).collect::<Vec<_>>()),
        }),
        Earshot::Describe => {
            let mut out = serde_json::Map::new();
            if let Some(o) = v.as_object() {
                for (k, x) in o {
                    if matches!(k.as_str(), "tags" | "labels" | "summary" | "description" | "caption" | "audioset" | "categories") {
                        out.insert(k.clone(), x.clone());
                    }
                }
            }
            if out.is_empty() { v.clone() } else { Value::Object(out) }
        }
        _ => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shuffle_is_a_permutation() {
        for n in 1..9 {
            let mut s = shuffle(n, n as u64);
            s.sort_unstable();
            assert_eq!(s, (0..n).collect::<Vec<_>>());
        }
    }

    #[test]
    fn blind_labels_are_letters_in_shown_order() {
        let spec = TabSpec {
            id: 1,
            kind: TabKind::Compare,
            items: vec!["a".into(), "b".into(), "c".into()],
            label: None,
            blind: true,
            loudness_match: true,
            note: None,
            run: None,
            filter: None,
            order: vec![2, 0, 1],
            revealed: false,
        };
        let t = Tab::new(spec);
        assert_eq!(t.member_label(2), "X");
        assert_eq!(t.member_label(0), "Y");
        assert_eq!(t.member_label(1), "Z");
    }
}
