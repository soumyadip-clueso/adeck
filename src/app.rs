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

use crate::board::{self, Board, Diff, Group};
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
    Board,
    Diff,
    /// Every page shared from adeck, to open or take down.
    Shared,
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
    /// Board tabs: the groups, and the one in focus (its members are `items`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<Board>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Blind boards: groups already picked, so shown with their names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revealed_groups: Vec<String>,
    /// Seeds a blind board's per-group shuffle.
    #[serde(default)]
    pub salt: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<Diff>,
    /// Board tabs: the stem lanes, while open, so a restart or a screenshot shows them too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stems: Option<StemView>,
}

/// Which member's stem lanes are open, and how they're set.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StemView {
    pub member: usize,
    /// Hearing the stems (true) or the full mix.
    pub mode: bool,
    pub solo: [bool; 4],
    pub mute: [bool; 4],
}

fn yes() -> bool {
    true
}

pub struct Tab {
    pub spec: TabSpec,
    /// The member you hear, by member index.
    pub active: usize,
    pub loop_secs: Option<(f64, f64)>,
    /// A shift-dragged stretch: Space plays just it, L loops it.
    pub selection: Option<(f64, f64)>,
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
    /// Board tabs: groups whose ranked rest is unfolded.
    pub more_open: HashSet<String>,
    pub details_open: bool,
    /// Board tabs: strips show just the sound, not the silence around it.
    pub zoom_sound: bool,
    pub move_open: bool,
    /// Pads: show the focused group's waveform, spectrogram and target bars.
    pub sheet_open: bool,
    /// The stretch of time the strips show, when zoomed in; `None` fits the whole file.
    pub view: Option<(f64, f64)>,
    /// Strip height, as a multiple of each strip's own.
    pub strip_h: f32,
    pub annotation_folded: bool,
    /// Scrubbing: whether it was playing when the drag began.
    pub scrub_resume: Option<bool>,
    /// Board tabs: stem lanes open under the tile you're hearing.
    pub stems_open: bool,
    /// Hearing the stems (summed, soloed, muted) rather than the full mix: the A/B.
    pub stem_mode: bool,
    /// Drums, bass, other, vocals.
    pub stem_solo: [bool; 4],
    pub stem_mute: [bool; 4],
}

impl Tab {
    pub fn new(spec: TabSpec) -> Tab {
        let sheet = spec.board.as_ref().is_some_and(|b| !b.is_pads());
        let filter = TextEdit::new(spec.filter.clone().unwrap_or_default());
        let sv = spec.stems.clone();
        Tab {
            spec,
            active: sv.as_ref().map_or(0, |v| v.member),
            loop_secs: None,
            selection: None,
            rows: Vec::new(),
            table: TableState::new().multi_select(true),
            filter,
            unscored_only: false,
            note: TextEdit::new(""),
            error: None,
            loading: false,
            videos: Vec::new(),
            picked: None,
            more_open: HashSet::new(),
            details_open: false,
            zoom_sound: true,
            move_open: false,
            sheet_open: sheet,
            view: None,
            strip_h: 1.0,
            annotation_folded: false,
            scrub_resume: None,
            stems_open: sv.is_some(),
            stem_mode: sv.as_ref().is_none_or(|v| v.mode),
            stem_solo: sv.as_ref().map_or([false; 4], |v| v.solo),
            stem_mute: sv.as_ref().map_or([false; 4], |v| v.mute),
        }
    }
    pub fn id(&self) -> u64 {
        self.spec.id
    }
    pub fn is_blind_hidden(&self) -> bool {
        self.spec.blind && !self.spec.revealed
    }
    pub fn board(&self) -> Option<&Board> {
        self.spec.board.as_ref()
    }
    /// The group in focus on a board.
    pub fn focused(&self) -> Option<&Group> {
        let b = self.board()?;
        self.spec.group.as_deref().and_then(|k| b.group(k)).or_else(|| b.sorted().first().copied())
    }
    /// Whether a board group hides who's who.
    pub fn group_hidden(&self, key: &str) -> bool {
        self.spec.blind && !self.spec.revealed_groups.iter().any(|g| g == key)
    }
    /// A board group's members in the order shown: shuffled while blind.
    pub fn group_order(&self, g: &Group) -> Vec<usize> {
        let n = g.members.len();
        if !self.spec.blind {
            return (0..n).collect();
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(&(self.spec.salt, self.spec.id, &g.key), &mut h);
        stable_shuffle(n, std::hash::Hasher::finish(&h))
    }
    /// A board member's label: its own, or its role and number; X, Y, Z while blind.
    pub fn board_label(&self, g: &Group, idx: usize) -> String {
        if idx < g.members.len() && self.group_hidden(&g.key) {
            let pos = self.group_order(g).iter().position(|&m| m == idx).unwrap_or(idx);
            return ((if pos < 3 { b'X' + pos as u8 } else { b'P' + (pos as u8 - 3) }) as char).to_string();
        }
        let all = g.playable();
        let Some(m) = all.get(idx) else { return String::new() };
        if let Some(l) = &m.label {
            return l.clone();
        }
        if idx >= g.members.len() {
            return format!("#{}", idx - g.members.len() + 1);
        }
        let role = if m.role.is_empty() { "take" } else { m.role.as_str() };
        let n = g.members[..idx].iter().filter(|x| x.role == m.role).count() + 1;
        let same = g.members.iter().filter(|x| x.role == m.role).count();
        let mut r = role.to_owned();
        if let Some(c) = r.get_mut(..1) {
            c.make_ascii_uppercase();
        }
        if same > 1 { format!("{r} {n}") } else { r }
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
    /// Where the sound is, and its waveform drawn over just that stretch.
    pub sound: Option<(f64, f64)>,
    pub zoom: Pixels,
    /// Small whole-file waveform for tiles and pads; `wave` and `spec` are drawn full size only
    /// while a strip shows this file, and dropped when it leaves (see `App::trim_pixels`).
    pub thumb: Pixels,
    pub full: std::cell::Cell<bool>,
    /// The waveform redrawn for a zoomed view, and which view (start and end ms, width).
    pub view_px: Pixels,
    pub view_key: std::cell::Cell<(u64, u64, u32, u32)>,
    /// The cached spectrogram picture, and its LED redraw for the view on screen.
    pub spec_raw: std::cell::RefCell<Vec<u8>>,
    pub spec_view: Pixels,
    pub spec_key: std::cell::Cell<(u64, u64, u32, u32)>,
    /// A board tile's waveform at the tile's size on screen, and for which (w, h, range ms).
    pub tile_px: Pixels,
    pub tile_key: std::cell::Cell<(u32, u32, u64, u64)>,
    /// A stem lane's waveform, and for which (w, h, range ms, shared peak bits).
    pub lane_px: Pixels,
    pub lane_key: std::cell::Cell<(u32, u32, u64, u64, u32)>,
}

impl Entry {
    /// Draws the full-size waveform and loads the spectrogram from the cache, once, the first
    /// time a strip needs them. Safe from the view: pictures fill in place.
    pub fn ensure_full(&self) {
        if self.full.get() {
            return;
        }
        let Some(a) = &self.analysis else { return };
        if let Some(rgba) = derive::cached_spec(&self.item.path, a) {
            *self.spec_raw.borrow_mut() = rgba;
        }
        self.full.set(true);
    }

    /// Lets the full-size pictures go.
    pub fn drop_full(&self) {
        if self.full.replace(false) {
            self.wave.set(1, 1, vec![0; 4]);
            self.spec.set(1, 1, vec![0; 4]);
            self.view_px.set(1, 1, vec![0; 4]);
            self.view_key.set((0, 0, 0, 0));
            self.spec_raw.borrow_mut().clear();
            self.spec_view.set(1, 1, vec![0; 4]);
            self.spec_key.set((0, 0, 0, 0));
        }
    }

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
            sound: None,
            zoom: Pixels::new(),
            thumb: Pixels::new(),
            full: std::cell::Cell::new(false),
            view_px: Pixels::new(),
            view_key: std::cell::Cell::new((0, 0, 0, 0)),
            spec_raw: std::cell::RefCell::new(Vec::new()),
            spec_view: Pixels::new(),
            spec_key: std::cell::Cell::new((0, 0, 0, 0)),
            tile_px: Pixels::new(),
            tile_key: std::cell::Cell::new((0, 0, 0, 0)),
            lane_px: Pixels::new(),
            lane_key: std::cell::Cell::new((0, 0, 0, 0, 0)),
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
    Resolved {
        tab: u64,
        id: String,
        result: Result<Item, String>,
    },
    Analysed {
        id: String,
        result: Result<Analysis, String>,
    },
    Listed {
        tab: u64,
        result: Result<Vec<Item>, String>,
    },
    Earshot {
        id: String,
        cmd: Earshot,
        result: Result<Value, String>,
    },
    Sheet {
        id: String,
        result: Result<PathBuf, String>,
    },
    Shot {
        result: Result<PathBuf, String>,
    },
    Shared {
        result: Result<crate::share::Share, String>,
    },
    Unshared {
        slug: String,
        result: Result<(), String>,
    },
    Live {
        result: Result<Vec<String>, String>,
    },
    /// The splitter finished (or skipped) one track.
    Split {
        path: PathBuf,
        error: Option<String>,
    },
}

/// How long a finished job stays in the footer (it stays in Activity until cleared).
pub const FOOTER_SECS: f64 = 6.0;

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
    /// The latest keep/drop/move on each item.
    pub decisions: HashMap<String, String>,
    /// Stems on disk, by item id.
    pub stems: HashMap<String, crate::stems::Stems>,
    /// Tracks queued for or being split.
    pub splitting: HashSet<PathBuf>,
    waker: Option<Waker>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    pub calls: Option<Receiver<mcp::Call>>,
    pub activity: std::sync::Arc<mcp::Activity>,
    pub dock: crate::dock::Dock,
    pub tab_drag: TabDragState,
    pub activity_open: bool,
    /// The job whose log is open in the Activity panel.
    pub log_open: Option<crate::jobs::JobId>,
    /// A message along the bottom: (text, is_error).
    pub toast: Option<(String, bool)>,
    /// The toast on screen and when it went up, to clear it after a few seconds.
    /// The toast on screen and when it first showed. Set while drawing, so a toast raised
    /// anywhere (a click, a job, the agent) starts its clock the moment you can see it.
    toast_seen: std::cell::RefCell<Option<(String, std::time::Instant)>>,
    /// The poll clock, so the footer knows how long ago a job finished.
    pub clock: f64,
    /// How many finished jobs were still in the footer at the last poll.
    fresh_jobs: usize,
    pub mcp_status: String,
    /// The new-tab menu, and the runs it offers (listed when it opens).
    pub new_menu: bool,
    pub menu_runs: Vec<String>,
    /// The pane's width, kept across tabs.
    pub sheet_w: f32,
    /// Where the pane sits: left, right, or hidden to a tab on the edge.
    pub pane: crate::ui::px::Dock,
    /// Which of the pane's sections is showing (META, EARSHOT, ...), by its label.
    pub pane_tab: String,
    /// Where the pane was before it was hidden, so `\\` brings it back there.
    pub pane_last: crate::ui::px::Dock,
    /// While the pane's grip is dragged: how far, so the view can show where it will dock.
    pub pane_drag: Option<f32>,
    /// While the pane's edge is dragged.
    pub pane_resizing: bool,
    /// The window's size and theme as last drawn, for screenshots.
    pub win: std::cell::Cell<(f32, f32, bool)>,
    pub shooting: bool,
    /// ⌘K, while open.
    pub palette: Option<crate::ui::palette::State>,
    pub keys_help: bool,
    /// Pages shared from adeck, newest first.
    pub shares: Vec<crate::share::Share>,
    /// A share being published; slugs being taken down; the slug whose take-down awaits a second press.
    pub sharing: bool,
    pub unsharing: HashSet<String>,
    pub confirm_unshare: Option<String>,
    /// Whether the publisher has been asked which shares are still up, and what it said.
    pub shares_checked: Option<Result<(), String>>,
    dirty: bool,
    pub headless: bool,
}

impl App {
    pub fn new() -> App {
        let (cfg, cfg_error) = Config::load();
        let (tx, rx) = channel();
        let headless = rune::tooling::headless_run().is_some() || std::env::var_os("ADECK_HEADLESS").is_some();
        let feedback = feedback::Store::new(crate::config::dir().join("feedback.jsonl"));
        let rows = feedback.read_all();
        let scores = feedback::latest_scores(&rows);
        let decisions = board::decisions(&rows);
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
            decisions,
            stems: HashMap::new(),
            splitting: HashSet::new(),
            waker: None,
            tx,
            rx,
            calls: None,
            activity: std::sync::Arc::default(),
            dock: crate::dock::Dock::default(),
            tab_drag: TabDragState::default(),
            activity_open: false,
            log_open: None,
            toast: cfg_error.map(|e| (e, true)),
            toast_seen: std::cell::RefCell::new(None),
            clock: 0.0,
            fresh_jobs: 0,
            new_menu: false,
            menu_runs: Vec::new(),
            sheet_w: 360.0,
            pane: crate::ui::px::Dock::Right,
            pane_tab: String::new(),
            pane_last: crate::ui::px::Dock::Right,
            pane_drag: None,
            pane_resizing: false,
            win: std::cell::Cell::new((1280.0, 800.0, true)),
            shooting: false,
            palette: None,
            keys_help: false,
            shares: crate::share::load(),
            sharing: false,
            unsharing: HashSet::new(),
            confirm_unshare: None,
            shares_checked: None,
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
            match mcp::serve(self.cfg.clone(), self.feedback.path.clone(), self.feedback.signal.clone(), self.activity.clone(), w) {
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
        let spec = TabSpec {
            id,
            kind,
            items,
            label,
            blind,
            loudness_match,
            note: None,
            run,
            filter,
            order,
            revealed: false,
            board: None,
            group: None,
            revealed_groups: Vec::new(),
            salt: feedback::now().to_bits(),
            diff: None,
            stems: None,
        };
        self.tabs.push(Tab::new(spec));
        self.load_tab(id);
        self.focus(id);
        self.dirty = true;
        id
    }

    /// Starts resolving and analysing everything a tab shows.
    fn load_tab(&mut self, id: u64) {
        let Some(t) = self.tab(id) else { return };
        let (kind, mut items, run) = (t.spec.kind, t.spec.items.clone(), t.spec.run.clone());
        if matches!(kind, TabKind::Diff | TabKind::Shared) {
            if kind == TabKind::Shared {
                self.check_shares();
            }
            return;
        }
        if let Some(b) = t.board() {
            items = b.items();
        }
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
        let name = e.item.name.clone();
        let title = format!("earshot {} · {name}", cmd.name());
        self.jobs.spawn(JobKind::Earshot, title, move |_| {
            let result = derive::earshot(&bin, &path, cmd);
            let outcome = match &result {
                Ok(_) => Outcome::Completed(format!("earshot {} · {name}", cmd.name())),
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

    /// Frees the full-size pictures of every file the front tab isn't showing in a strip.
    pub fn trim_pixels(&self) {
        let keep: HashSet<&String> = self.current().map(|t| t.spec.items.iter().collect()).unwrap_or_default();
        for (id, e) in &self.items {
            if !keep.contains(id) {
                e.drop_full();
            }
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
        self.trim_pixels();
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
        if matches!(t.spec.kind, TabKind::Run | TabKind::Diff | TabKind::Shared) || self.headless {
            return;
        }
        let tab = t.id();
        let slots = slots(t);
        let (active, loop_secs, selection) = (t.active, t.loop_secs, t.selection);
        let paths: Option<Vec<(String, PathBuf)>> = slots.iter().map(|&m| t.spec.items.get(m).and_then(|i| self.items.get(i).map(|e| (i.clone(), e.item.path.clone())))).collect();
        let Some(mut paths) = paths else { return };
        if paths.is_empty() {
            return;
        }
        // The stems of the one you hear ride along after the members, on the same clock.
        let n = paths.len();
        if let Some(st) = self.open_stems(t) {
            let sp: Option<Vec<(String, PathBuf)>> = st.files.iter().map(|p| self.items.get(&stem_id(p)).map(|e| (stem_id(p), e.item.path.clone()))).collect();
            paths.extend(sp.unwrap_or_default());
        }
        // Stems coming or going, members unchanged: keep the place and keep playing.
        let carry = self
            .mixer
            .loaded
            .as_ref()
            .is_some_and(|(o, ids)| *o == tab && ids.len() >= n && ids.len() != paths.len() && ids[..n].iter().zip(&paths[..n]).all(|(a, (b, _))| a == b));
        let (at, was) = (self.mixer.secs(), self.mixer.is_playing() || self.mixer.wants_play());
        self.mixer.load(tab, paths);
        if carry {
            self.mixer.seek_secs(at);
            if was {
                self.mixer.play();
            }
        }
        self.mixer.set_active(slots.iter().position(|&m| m == active).unwrap_or(0));
        self.mixer.set_loop_secs(loop_secs);
        self.mixer.set_range_secs(selection);
        self.apply_trims();
        self.apply_mix();
    }

    /// The stems of the board member you're hearing, while its lanes are open and they're on disk.
    pub fn open_stems(&self, t: &Tab) -> Option<&crate::stems::Stems> {
        if !t.stems_open || t.board().is_none_or(Board::is_pads) {
            return None;
        }
        self.stems.get(t.spec.items.get(t.active)?)
    }

    /// Whether the mixer has the open stems loaded after the members (so the lanes can play).
    pub fn stems_loaded(&self, t: &Tab) -> bool {
        let n = slots(t).len();
        self.mixer_is_for(t.id()) && self.open_stems(t).is_some() && self.mixer.loaded.as_ref().is_some_and(|(_, ids)| ids.len() == n + 4)
    }

    /// The A/B and solo/mute as mixer levels: the stems at their gains and the members silent,
    /// or (on the mix side) just the member you hear.
    pub fn apply_mix(&mut self) {
        // Keep the lanes' state with the tab, for the session and screenshots.
        if let Some(t) = self.current_mut() {
            let v = t.stems_open.then_some(StemView { member: t.active, mode: t.stem_mode, solo: t.stem_solo, mute: t.stem_mute });
            if t.spec.stems != v {
                t.spec.stems = v;
                self.dirty = true;
            }
        }
        let Some(t) = self.current() else { return };
        let mix = (t.stem_mode && self.stems_loaded(t)).then(|| {
            let mut v = vec![0.0; slots(t).len()];
            v.extend(crate::stems::gains(t.stem_solo, t.stem_mute));
            v
        });
        self.mixer.set_mix(mix);
    }

    /// Opens or closes the stem lanes under a tile; opening one that isn't split yet splits it.
    pub fn toggle_stems(&mut self, key: &str, m: usize) {
        self.focus_group(key);
        let Some(t) = self.current() else { return };
        let opening = !(t.stems_open && t.active == m);
        if t.active != m {
            self.set_active_member(m);
        }
        if let Some(t) = self.current_mut() {
            t.stems_open = opening;
            t.stem_mode = true;
            t.sheet_open = true;
        }
        if opening {
            self.prepare_stems();
        }
        self.sync_mixer();
        self.dirty = true;
    }

    /// The stems of the one you hear: resolved and analysed if they're on disk, split if not.
    fn prepare_stems(&mut self) {
        let Some(t) = self.current() else { return };
        let (tab, id) = (t.id(), t.spec.items.get(t.active).cloned());
        let Some(id) = id else { return };
        match self.stems.get(&id).map(|st| st.files.clone()) {
            Some(files) => {
                for p in &files {
                    self.ensure(tab, &stem_id(p));
                }
            }
            None => {
                let _ = self.split_stems(vec![id]);
            }
        }
    }

    pub fn toggle_stems_active(&mut self) {
        let Some(t) = self.current() else { return };
        let Some(key) = t.focused().map(|g| g.key.clone()) else { return };
        let m = t.active;
        self.toggle_stems(&key, m);
    }

    /// The A/B: the stems (true) or the full mix.
    pub fn set_stem_mode(&mut self, on: bool) {
        if let Some(t) = self.current_mut() {
            t.stem_mode = on;
        }
        self.apply_mix();
    }

    pub fn toggle_stem_mode(&mut self) {
        let on = self.current().is_some_and(|t| !t.stem_mode);
        self.set_stem_mode(on);
    }

    /// Solo (or mute, with `mute`) one stem; either puts you on the stems side of the A/B.
    pub fn stem_press(&mut self, i: usize, mute: bool) {
        if let Some(t) = self.current_mut() {
            if mute {
                t.stem_mute[i] ^= true;
            } else {
                t.stem_solo[i] ^= true;
            }
            t.stem_mode = true;
        }
        self.apply_mix();
    }

    /// Splits these items into stems in the background, skipping references, ones already
    /// split and ones on their way. Returns (queued, already split).
    pub fn split_stems(&mut self, ids: Vec<String>) -> Result<(usize, usize), String> {
        let mut files = Vec::new();
        let mut have = 0;
        for id in &ids {
            let Some(e) = self.items.get(id) else { continue };
            if e.item.is_ref || e.item.media != Media::Audio {
                continue;
            }
            if self.stems.contains_key(id) {
                have += 1;
            } else if !self.splitting.contains(&e.item.path) {
                files.push(e.item.path.clone());
            }
        }
        if files.is_empty() || self.headless {
            return Ok((0, have));
        }
        crate::stems::uv(&self.cfg).ok_or("Splitting stems needs uv (https://docs.astral.sh/uv/), or [stems] uv = \"/path/to/uv\" in the config.")?;
        self.splitting.extend(files.iter().cloned());
        let n = files.len();
        let (cfg, tx, waker) = (self.cfg.clone(), self.tx.clone(), self.waker.clone());
        let title =
            if n == 1 { format!("Stems · {}", files[0].file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned())) } else { format!("Stems · {n} tracks") };
        self.jobs.spawn(JobKind::Stems, title, move |rep| {
            let mut i = 0;
            let mut left: HashSet<PathBuf> = files.iter().cloned().collect();
            let cancel = rep.token().flag();
            let res = crate::stems::split(&cfg, &files, &cancel, |p| {
                if p.status == "model" {
                    rep.info("Model loaded");
                    return;
                }
                let Some(path) = p.file else { return };
                i += 1;
                let name = path.file_name().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                rep.progress(i as u64, Some(n as u64), name.clone());
                match &p.error {
                    Some(e) => rep.warn(format!("{name}: {e}")),
                    None => rep.info(format!("{name}: {}", p.status)),
                }
                left.remove(&path);
                let _ = tx.send(Event::Split { path, error: p.error });
                if let Some(w) = &waker {
                    w();
                }
            });
            // Anything the splitter never reported (it died) is no longer on its way.
            for path in left {
                let _ = tx.send(Event::Split { path, error: Some("not split".into()) });
            }
            if let Some(w) = &waker {
                w();
            }
            match res {
                Ok((done, 0)) => Outcome::Completed(format!("Stems for {done} track{}", if done == 1 { "" } else { "s" })),
                Ok((done, failed)) => Outcome::Failed(format!("{failed} of {} couldn't be split; the log says why", done + failed)),
                Err(e) => Outcome::Failed(e),
            }
        });
        Ok((n, have))
    }

    /// The loudness match's trims for the active tab (or unity when it's off).
    pub fn apply_trims(&mut self) {
        let Some(t) = self.current() else { return };
        let on = t.spec.loudness_match && t.spec.items.len() > 1;
        let mut trims: Vec<f32> = slots(t).iter().filter_map(|&m| t.spec.items.get(m)).map(|i| if on { mixer::trim_for(self.lufs(i), MATCH_LUFS) } else { 1.0 }).collect();
        // Stems take their track's trim, so the A/B compares like with like.
        if self.stems_loaded(t) {
            let tr = t.spec.items.get(t.active).filter(|_| on).map_or(1.0, |i| mixer::trim_for(self.lufs(i), MATCH_LUFS));
            trims.extend([tr; 4]);
        }
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
        // Same members: load is a no-op and this only moves the gain. A folded take swaps in.
        self.sync_mixer();
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
        let mut paths = Vec::new();
        for p in files {
            if p.is_dir() {
                paths.extend(source::media_files(&p));
            } else {
                paths.push(p);
            }
        }
        let ids: Vec<String> = paths.into_iter().filter(|p| crate::item::media_of(p).is_some()).map(|p| p.display().to_string()).collect();
        // Onto a single file or a compare: they join it, side by side.
        if !ids.is_empty() && self.current().is_some_and(|t| matches!(t.spec.kind, TabKind::Single | TabKind::Compare)) {
            self.add_members(ids);
            return;
        }
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

    // Boards.

    /// Opens a board, or a diff when `diff` is given.
    pub fn open_page(&mut self, label: Option<String>, board: Option<Board>, diff: Option<Diff>, blind: bool, loudness_match: bool) -> u64 {
        let kind = if diff.is_some() { TabKind::Diff } else { TabKind::Board };
        let id = self.open(kind, Vec::new(), label, blind, loudness_match, None, None);
        if let Some(t) = self.tab_mut(id) {
            t.sheet_open = board.as_ref().is_some_and(|b| !b.is_pads());
            t.spec.board = board;
            t.spec.diff = diff;
        }
        self.refresh_board(id);
        self.load_tab(id);
        id
    }

    /// Puts a board's focused group on `items`, so playback, the loudness match and the strips
    /// work as they do on a compare.
    pub fn refresh_board(&mut self, tab: u64) {
        let Some(t) = self.tab(tab) else { return };
        let Some(g) = t.focused() else {
            if let Some(t) = self.tab_mut(tab) {
                t.spec.items.clear();
            }
            return;
        };
        let key = g.key.clone();
        let ids: Vec<String> = g.playable().iter().map(|m| m.item.clone()).collect();
        let Some(t) = self.tab_mut(tab) else { return };
        t.spec.group = Some(key);
        if t.spec.items != ids {
            let was = t.spec.items.get(t.active).cloned();
            t.active = was.and_then(|w| ids.iter().position(|i| *i == w)).unwrap_or(0);
            t.spec.items = ids;
            t.loop_secs = None;
            t.move_open = false;
            t.view = None;
            if self.active == Some(tab) {
                self.sync_mixer();
                self.trim_pixels();
            }
        }
        self.dirty = true;
    }

    pub fn focus_group(&mut self, key: &str) {
        let Some(id) = self.active else { return };
        if let Some(t) = self.tab_mut(id) {
            if t.spec.group.as_deref() == Some(key) {
                return;
            }
            t.spec.group = Some(key.to_owned());
        }
        self.refresh_board(id);
    }

    /// Plays one member of a group: the first press starts it, a press on another member while
    /// playing switches at the same moment, a press on the one playing starts it again.
    pub fn board_play(&mut self, key: &str, idx: usize) {
        self.focus_group(key);
        if let Some(t) = self.current_mut() {
            t.sheet_open = true;
        }
        let Some(t) = self.current() else { return };
        if idx >= t.spec.items.len() {
            return;
        }
        let playing = self.mixer_is_for(t.id()) && self.mixer.is_playing();
        let same = t.active == idx;
        self.set_active_member(idx);
        if !playing || same {
            let from = self.view_start();
            self.seek_secs(from);
        }
        self.sync_mixer();
        self.mixer.play();
    }

    /// A pad: plays this member from the very top, whatever was playing.
    pub fn pad_play(&mut self, key: &str, idx: usize) {
        self.focus_group(key);
        let Some(t) = self.current() else { return };
        if idx >= t.spec.items.len() {
            return;
        }
        self.set_active_member(idx);
        self.set_loop(None);
        self.seek_secs(0.0);
        self.sync_mixer();
        self.mixer.play();
    }

    /// Space on pads: stop, or play the last pad from the top again.
    pub fn pad_replay(&mut self) {
        if self.mixer.is_playing() {
            self.mixer.pause();
        } else {
            self.seek_secs(0.0);
            self.sync_mixer();
            self.mixer.play();
        }
    }

    /// Scores one file (0-3) without it having to be the one you're hearing.
    pub fn score_item(&mut self, id: &str, value: i64) {
        let Some(t) = self.current() else { return };
        let row = Row {
            ts: feedback::now(),
            tab_id: t.id(),
            kind: "score".into(),
            items: vec![id.to_owned()],
            value: Value::from(value),
            note: t.note.text.trim().to_owned(),
            blind: t.spec.blind,
            reveal_map: None,
        };
        if self.write(row) {
            self.scores.insert(id.to_owned(), value);
            if let Some(t) = self.current_mut() {
                t.note = TextEdit::new("");
            }
        }
    }

    /// Scores the pad you played last.
    pub fn score_active(&mut self, value: i64) {
        if let Some(id) = self.current().and_then(|t| t.spec.items.get(t.active).cloned()) {
            self.score_item(&id, value);
        }
    }

    /// Where playback starts over: the start of the sound when zoomed to it, else the start.
    pub fn view_start(&self) -> f64 {
        self.view_range().map_or(0.0, |(a, _)| a)
    }

    /// The stretch of the active member a board's strips show: just the sound while zoomed.
    pub fn view_range(&self) -> Option<(f64, f64)> {
        let t = self.current()?;
        let e = self.entry(t.spec.items.get(t.active)?)?;
        let secs = e.analysis.as_ref()?.secs;
        if !t.zoom_sound {
            return Some((0.0, secs));
        }
        e.sound.or(Some((0.0, secs)))
    }

    pub fn board_replay(&mut self) {
        let from = self.view_start();
        self.seek_secs(from);
        self.mixer.play();
    }

    /// The next or previous group, keeping playback going if it was.
    pub fn board_step_group(&mut self, by: i64) {
        let Some(t) = self.current() else { return };
        let Some(b) = t.board() else { return };
        let keys: Vec<String> = b.sorted().iter().map(|g| g.key.clone()).collect();
        if keys.is_empty() {
            return;
        }
        let at = t.spec.group.as_ref().and_then(|k| keys.iter().position(|x| x == k)).unwrap_or(0) as i64;
        let next = (at + by).rem_euclid(keys.len() as i64) as usize;
        let playing = self.mixer.is_playing();
        let key = keys[next].clone();
        self.focus_group(&key);
        if playing {
            let first = self.current().and_then(|t| t.focused().map(|g| t.group_order(g))).and_then(|o| o.first().copied()).unwrap_or(0);
            self.board_play(&key, first);
        }
    }

    /// The next or previous member of the focused group, in shown order, played.
    pub fn board_step_member(&mut self, by: i64) {
        let Some(t) = self.current() else { return };
        let Some(g) = t.focused() else { return };
        let mut order = t.group_order(g);
        order.extend(g.members.len()..g.playable().len());
        if order.is_empty() {
            return;
        }
        let at = order.iter().position(|&m| m == t.active).unwrap_or(0) as i64;
        let next = order[(at + by).rem_euclid(order.len() as i64) as usize];
        let key = g.key.clone();
        if t.board().is_some_and(Board::is_pads) {
            self.pad_play(&key, next);
        } else {
            self.board_play(&key, next);
        }
    }

    /// Loops what the strip shows (the sound, or the whole file), or stops looping.
    pub fn board_toggle_loop(&mut self) {
        if self.current().is_some_and(|t| t.loop_secs.is_some()) {
            self.set_loop(None);
        } else if let Some(r) = self.view_range() {
            self.set_loop(Some(r));
        }
    }

    pub fn toggle_zoom(&mut self) {
        if let Some(t) = self.current_mut() {
            t.zoom_sound ^= true;
            t.view = None;
        }
    }

    /// Records keep, drop or `move:<group>` for one file.
    pub fn decide(&mut self, id: &str, value: &str) {
        let Some(t) = self.current() else { return };
        let note = t.note.text.trim().to_owned();
        let row = Row { ts: feedback::now(), tab_id: t.id(), kind: "decision".into(), items: vec![id.to_owned()], value: Value::from(value), note, blind: false, reveal_map: None };
        if self.write(row) {
            self.decisions.insert(id.to_owned(), value.to_owned());
            if let Some(t) = self.current_mut() {
                t.note = TextEdit::new("");
                t.move_open = false;
            }
        }
    }

    /// Decides on the member you're hearing.
    pub fn decide_active(&mut self, value: &str) {
        let Some(id) = self.current().and_then(|t| t.spec.items.get(t.active).cloned()) else { return };
        self.decide(&id, value);
    }

    /// Picks the member you're hearing as the best of its group, revealing a blind group.
    pub fn board_pick(&mut self) {
        let Some(t) = self.current() else { return };
        let Some(g) = t.focused() else { return };
        let Some(picked) = t.spec.items.get(t.active).cloned() else { return };
        let items: Vec<String> = g.members.iter().map(|m| m.item.clone()).collect();
        let hidden = t.group_hidden(&g.key);
        let reveal = hidden.then(|| Value::Object((0..g.members.len()).map(|i| (t.board_label(g, i), Value::String(g.members[i].item.clone()))).collect()));
        let key = g.key.clone();
        let row = Row {
            ts: feedback::now(),
            tab_id: t.id(),
            kind: "pick".into(),
            items,
            value: Value::String(picked),
            note: t.note.text.trim().to_owned(),
            blind: t.spec.blind,
            reveal_map: reveal,
        };
        if self.write(row)
            && let Some(t) = self.current_mut()
        {
            if !t.spec.revealed_groups.contains(&key) {
                t.spec.revealed_groups.push(key);
            }
            t.note = TextEdit::new("");
            self.dirty = true;
        }
    }

    pub fn toggle_more(&mut self, key: &str) {
        if let Some(t) = self.current_mut()
            && !t.more_open.remove(key)
        {
            t.more_open.insert(key.to_owned());
        }
    }

    /// Adds or replaces groups (boards) or rows (diffs) by key, while the tab is open.
    pub fn upsert(&mut self, tab: u64, a: &Value) -> Result<Value, String> {
        let t = self.tab_mut(tab).ok_or_else(|| format!("No tab {tab}."))?;
        let mut out = json!({ "tab_id": tab });
        if let Some(d) = t.spec.diff.as_mut() {
            let rows: Vec<board::DiffRow> = serde_json::from_value(a.get("rows").cloned().unwrap_or(json!([]))).map_err(|e| format!("rows: {e}"))?;
            if let Some(m) = a.get("metrics") {
                d.metrics = serde_json::from_value(m.clone()).map_err(|e| format!("metrics: {e}"))?;
            }
            let n = rows.len();
            let fresh = board::upsert(&mut d.rows, rows, |r| &r.key);
            out["added"] = json!(fresh);
            out["replaced"] = json!(n - fresh);
            out["rows"] = json!(d.rows.len());
        } else {
            let b = t.spec.board.get_or_insert_with(Board::default);
            let groups: Vec<Group> = serde_json::from_value(a.get("groups").cloned().unwrap_or(json!([]))).map_err(|e| format!("groups: {e}"))?;
            if let Some(m) = a.get("metrics") {
                b.metrics = serde_json::from_value(m.clone()).map_err(|e| format!("metrics: {e}"))?;
            }
            let n = groups.len();
            let fresh = board::upsert(&mut b.groups, groups, |g| &g.key);
            if let Some(keys) = a.get("remove").and_then(Value::as_array) {
                b.groups.retain(|g| !keys.iter().any(|k| k.as_str() == Some(g.key.as_str())));
            }
            out["added"] = json!(fresh);
            out["replaced"] = json!(n - fresh);
            out["groups"] = json!(b.groups.len());
        }
        self.dirty = true;
        self.refresh_board(tab);
        self.load_tab(tab);
        Ok(out)
    }

    // Editing a compare from the UI.

    /// Adds files to the front single or compare, which becomes a compare (up to 9).
    pub fn add_members(&mut self, ids: Vec<String>) {
        let Some(tab) = self.active else {
            self.open_files(ids.into_iter().map(PathBuf::from).collect());
            return;
        };
        let Some(t) = self.tab_mut(tab) else { return };
        let mut added = 0;
        for id in ids {
            if t.spec.items.len() >= 9 {
                break;
            }
            if !t.spec.items.contains(&id) {
                t.spec.items.push(id);
                added += 1;
            }
        }
        if t.spec.items.len() > 1 {
            t.spec.kind = TabKind::Compare;
        }
        if t.spec.blind {
            t.spec.order = shuffle(t.spec.items.len(), t.spec.id);
        }
        let full = t.spec.items.len() >= 9;
        self.toast = Some((if added == 0 && full { "A compare holds 9 files.".into() } else { format!("Added {added} file{}", if added == 1 { "" } else { "s" }) }, false));
        self.load_tab(tab);
        self.sync_mixer();
        self.dirty = true;
    }

    pub fn remove_member(&mut self, m: usize) {
        let Some(t) = self.current_mut() else { return };
        if m >= t.spec.items.len() {
            return;
        }
        t.spec.items.remove(m);
        t.active = t.active.min(t.spec.items.len().saturating_sub(1));
        if t.spec.blind {
            t.spec.order = shuffle(t.spec.items.len(), t.spec.id);
        }
        self.sync_mixer();
        self.dirty = true;
    }

    /// An empty compare to drop or add files into.
    pub fn new_compare(&mut self) {
        self.open(TabKind::Compare, Vec::new(), Some("Compare".into()), false, true, None, None);
    }

    pub fn open_new_menu(&mut self) {
        self.menu_runs = source::list_runs(&self.cfg).into_iter().map(|r| r.name).take(24).collect();
        self.new_menu = true;
    }

    /// A folder from the UI: a run table of everything in it.
    pub fn open_folder(&mut self, dir: PathBuf) {
        let run = dir.display().to_string();
        let label = dir.file_name().map(|n| n.to_string_lossy().into_owned());
        self.open(TabKind::Run, Vec::new(), label, false, true, Some(run), None);
    }

    // Handing the agent what you're on.

    /// Sends the agent a reference to what you're on: the member (`pick`, or the one you're
    /// hearing), its group, the loop or zoomed stretch, and the playhead. It lands as a
    /// `reference` row (so `wait_for_feedback` wakes) and as text on the clipboard to paste.
    pub fn send_to_agent(&mut self, pick: Option<usize>, tx: &mut Tx) {
        if let Some(text) = self.reference(pick) {
            tx.copy(text);
            self.toast = Some(("Sent to the agent, and copied to paste".into(), false));
        }
    }

    /// Writes the reference row and returns the text to paste.
    pub fn reference(&mut self, pick: Option<usize>) -> Option<String> {
        let t = self.current()?;
        let label = crate::ui::tab_title(self, t);
        let mut v = json!({ "tab_id": t.id(), "tab": label, "kind": t.spec.kind });
        let mut items: Vec<String> = Vec::new();
        let mut words = vec![format!("adeck tab {} \u{201c}{label}\u{201d}", t.id())];
        if t.spec.kind == TabKind::Run {
            items = t.table.selected().map(str::to_owned).collect();
            if items.is_empty() {
                items = t.rows.iter().take(1).cloned().collect();
            }
            words.push(format!("{} selected", items.len()));
        } else if let Some(m) = pick.or(Some(t.active)).filter(|m| *m < t.spec.items.len()) {
            let id = t.spec.items[m].clone();
            let name = self.entry(&id).map_or_else(|| id.rsplit('/').next().unwrap_or(&id).to_owned(), |e| e.item.name.clone());
            let shown = match t.focused() {
                Some(g) => {
                    v["group"] = json!(g.key);
                    words.push(g.title().to_owned());
                    t.board_label(g, m)
                }
                None => t.member_label(m),
            };
            let hidden = t.focused().map_or(t.is_blind_hidden(), |g| m < g.members.len() && t.group_hidden(&g.key));
            v["member"] = json!(m);
            v["label"] = json!(shown);
            v["item"] = if hidden { Value::Null } else { json!(id) };
            words.push(if hidden { shown } else { format!("{shown} \u{2014} {name}") });
            items.push(id);
        }
        let range = t.loop_secs.or(t.selection).or(t.view);
        if let Some((a, b)) = range {
            v["range_s"] = json!([(a * 1000.0).round() / 1000.0, (b * 1000.0).round() / 1000.0]);
            v["range_is"] = json!(if t.loop_secs.is_some() {
                "loop"
            } else if t.selection.is_some() {
                "selection"
            } else {
                "zoom"
            });
            words.push(format!("{}\u{2013}{}", crate::ui::widgets::clock(a), crate::ui::widgets::clock(b)));
        }
        if self.mixer_is_for(t.id()) {
            v["playhead_s"] = json!((self.mixer.secs() * 1000.0).round() / 1000.0);
        }
        let note = t.note.text.trim().to_owned();
        let row = Row { ts: feedback::now(), tab_id: t.id(), kind: "reference".into(), items, value: v, note, blind: t.spec.blind, reveal_map: None };
        let text = words.join(" \u{b7} ") + " (adeck get_selection)";
        if !self.write(row) {
            return None;
        }
        if let Some(t) = self.current_mut() {
            t.note = TextEdit::new("");
        }
        Some(text)
    }

    // The time axis: zoom, scrub, strip height.

    /// The stretch the strips show for the front tab.
    pub fn window(&self, t: &Tab) -> (f64, f64) {
        let total = crate::ui::widgets::tab_secs(self, t);
        if let Some((a, b)) = t.view {
            return (a.max(0.0), b.min(total.max(b)));
        }
        if t.board().is_some()
            && t.zoom_sound
            && let Some(r) = self.view_range()
        {
            return r;
        }
        (0.0, total)
    }

    /// Zooms the time axis by `factor` (below 1 zooms in), around the playhead when it's in view.
    pub fn zoom(&mut self, factor: f64) {
        let Some(t) = self.current() else { return };
        let total = crate::ui::widgets::tab_secs(self, t);
        if total <= 0.0 {
            return;
        }
        let (lo, hi) = self.window(t);
        let ph = crate::ui::widgets::playhead(self, t);
        let mid = if ph >= lo && ph <= hi { ph } else { (lo + hi) / 2.0 };
        let span = ((hi - lo) * factor).clamp(0.01_f64.min(total), total);
        let a = (mid - (mid - lo) / (hi - lo).max(1e-9) * span).clamp(0.0, total - span);
        let view = if span >= total - 1e-6 { None } else { Some((a, a + span)) };
        if let Some(t) = self.current_mut() {
            t.view = view;
            if view.is_none() {
                t.zoom_sound = false;
            }
        }
    }

    /// Fits the whole file, or the loop when there is one and it isn't already shown.
    pub fn zoom_fit(&mut self) {
        let Some(t) = self.current_mut() else { return };
        t.view = match t.loop_secs.or(t.selection) {
            Some(l) if t.view != Some(l) => Some(l),
            _ => None,
        };
        t.zoom_sound = false;
    }

    /// Shifts a zoomed view by a fraction of its width.
    pub fn pan(&mut self, frac: f64) {
        let Some(t) = self.current() else { return };
        let total = crate::ui::widgets::tab_secs(self, t);
        let Some((a, b)) = t.view else { return };
        let span = b - a;
        let na = (a + span * frac).clamp(0.0, (total - span).max(0.0));
        if let Some(t) = self.current_mut() {
            t.view = Some((na, na + span));
        }
    }

    /// A press or drag across a strip: it scrubs, playing from wherever the pointer is, and
    /// goes back to how it was when the drag ends.
    /// Dragging moves the playhead silently: playback pauses on press and picks up from where you
    /// let go, only if it was playing.
    fn toast_life(err: bool) -> std::time::Duration {
        std::time::Duration::from_secs(if err { 8 } else { 4 })
    }

    /// Called while drawing a toast: the first time this one shows, start its clock and ask
    /// for a wake when it's due, so it goes on time even if nothing else happens.
    pub fn toast_shown(&self) {
        let mut seen = self.toast_seen.borrow_mut();
        let Some((msg, err)) = &self.toast else {
            // Gone (timed out or closed): the next toast, even the same words, starts afresh.
            *seen = None;
            return;
        };
        if seen.as_ref().is_some_and(|(m, _)| m == msg) {
            return;
        }
        *seen = Some((msg.clone(), std::time::Instant::now()));
        if let Some(w) = self.waker.clone() {
            let life = Self::toast_life(*err) + std::time::Duration::from_millis(30);
            std::thread::spawn(move || {
                std::thread::sleep(life);
                w();
            });
        }
    }

    /// Shift-drag on a strip selects a stretch; nothing plays. Space then plays just the
    /// stretch and stops at its end, and L loops it. A shift-click with no stretch clears it.
    pub fn select_stretch(&mut self, from: f64, at: f64, phase: rune::Phase) {
        let (a, b) = (from.min(at).max(0.0), from.max(at));
        let real = b - a >= 0.01;
        if real {
            // A new stretch replaces any loop that was running on the old one.
            if self.current().is_some_and(|t| t.loop_secs.is_some()) {
                self.set_loop(None);
            }
            self.set_selection(Some((a, b)));
            if matches!(phase, rune::Phase::Up) {
                // Parked at its start, ready for Space; nothing plays until you ask.
                let playing = self.mixer.is_playing();
                self.seek_secs(a);
                if !playing {
                    self.mixer.pause();
                }
            }
        } else if matches!(phase, rune::Phase::Up) {
            self.set_selection(None);
            self.set_loop(None);
        }
    }

    pub fn set_selection(&mut self, sel: Option<(f64, f64)>) {
        if let Some(t) = self.current_mut() {
            t.selection = sel;
        }
        self.mixer.set_range_secs(sel);
    }

    /// L: loop the selection (or, on a board, what the strip shows); again to stop looping.
    pub fn toggle_loop(&mut self) {
        let Some(t) = self.current() else { return };
        if t.loop_secs.is_some() {
            self.set_loop(None);
        } else if let Some(sel) = t.selection {
            self.set_loop(Some(sel));
        } else if t.spec.kind == TabKind::Board {
            self.board_toggle_loop();
        }
    }

    /// Esc: drop the loop and the selection.
    pub fn clear_loop(&mut self) {
        self.set_loop(None);
        self.set_selection(None);
    }

    pub fn scrub(&mut self, at: f64, phase: rune::Phase) {
        let playing = self.current().is_some_and(|t| self.mixer_is_for(t.id())) && self.mixer.is_playing();
        match phase {
            rune::Phase::Down => {
                if let Some(t) = self.current_mut() {
                    t.scrub_resume = Some(playing);
                }
                if playing {
                    self.mixer.pause();
                }
                self.seek_secs(at);
            }
            rune::Phase::Up => {
                self.seek_secs(at);
                if self.current_mut().and_then(|t| t.scrub_resume.take()).unwrap_or(false) {
                    self.mixer.play();
                }
            }
            _ => self.seek_secs(at),
        }
    }

    /// Publishes the front tab as a page. Reference-pack audio never leaves: those members show
    /// as local-only on the page.
    pub fn share(&mut self) -> Result<(), String> {
        if self.sharing {
            return Err("A share is already on its way.".into());
        }
        let x = self.export().ok_or("This tab can't be shared: open a compare, a single file, a render or a board.")?;
        self.sharing = true;
        let (cfg, aws) = (self.cfg.share.clone(), self.cfg.aws.clone());
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        let work = move || {
            let result = crate::share::publish(x, &cfg, &aws);
            let _ = tx.send(Event::Shared { result });
            if let Some(w) = waker {
                w();
            }
        };
        if self.headless {
            work();
        } else {
            std::thread::spawn(work);
        }
        Ok(())
    }

    /// A board's groups in order, narrowed by the tab's search: a group stays when its title,
    /// key, subtitle or chip matches, or any of its sounds does (label, role or file name).
    pub fn shown_groups<'a>(&self, t: &'a Tab) -> Vec<&'a crate::board::Group> {
        let Some(b) = t.board() else { return Vec::new() };
        let q = t.filter.text.trim().to_lowercase();
        if q.is_empty() {
            return b.sorted();
        }
        let has = |s: &str| s.to_lowercase().contains(&q);
        b.sorted()
            .into_iter()
            .filter(|g| {
                has(g.title()) || has(&g.key) || g.subtitle.as_deref().is_some_and(has) || g.chip.as_ref().is_some_and(|c| has(&c.text)) || {
                    let hidden = t.group_hidden(&g.key);
                    g.playable()
                        .iter()
                        .enumerate()
                        .any(|(i, m)| has(&t.board_label(g, i)) || (!hidden && (has(&m.role) || self.entry(&m.item).is_some_and(|e| !e.item.is_ref && has(&e.item.name)))))
                }
            })
            .collect()
    }

    /// Whether the front tab is a kind that shares (cheap: for the footer button).
    pub fn can_share(&self) -> bool {
        self.current().is_some_and(|t| !matches!(t.spec.kind, TabKind::Run | TabKind::Diff | TabKind::Shared))
    }

    /// The front tab as page data: one group per board group (or the tab's members as one).
    pub fn export(&self) -> Option<crate::share::Export> {
        let t = self.current()?;
        if matches!(t.spec.kind, TabKind::Run | TabKind::Diff | TabKind::Shared) {
            return None;
        }
        let title = crate::ui::tab_title(self, t);
        let mut files: Vec<(PathBuf, String)> = Vec::new();
        let mut stem_files: Vec<(PathBuf, String)> = Vec::new();
        let mut member = |id: &str, label: String, role: &str, numbers: Value, chip: Value, meta: serde_json::Map<String, Value>| {
            let e = self.items.get(id);
            let is_ref = id.starts_with("ref:") || e.is_some_and(|e| e.item.is_ref);
            let (src, base) = match e.filter(|_| !is_ref) {
                Some(e) => {
                    let name = format!("{:02}-{}", files.len(), crate::share::slug("", &e.item.name).rsplit_once('-').map_or("", |x| x.0));
                    files.push((e.item.path.clone(), name.clone()));
                    (Value::String(name.clone()), Some(name))
                }
                None => (Value::Null, None),
            };
            let role = if is_ref && role.is_empty() { "ref" } else { role };
            // A pack file's own name stays home too.
            let label = if is_ref { label.split_once(" · ").map_or_else(|| "Reference".to_owned(), |(l, _)| format!("{l} · Reference")) } else { label };
            let mut v = serde_json::json!({"label": label, "role": role, "numbers": numbers, "chip": chip, "meta": meta, "src": src});
            // Its stems, four to a track, named after it.
            if let (Some(base), Some(st)) = (base, self.stems.get(id)) {
                let parts: Vec<Value> = crate::stems::NAMES
                    .iter()
                    .enumerate()
                    .map(|(i, n)| {
                        let name = format!("{base}-{n}");
                        stem_files.push((st.files[i].clone(), name.clone()));
                        serde_json::json!({"stem": n, "src": name, "lufs": st.lufs[i], "share": st.share[i]})
                    })
                    .collect();
                v["stems"] = Value::Array(parts);
                v["stem_model"] = serde_json::json!(st.model);
            }
            v
        };
        let (groups, layout, labels) = match t.board() {
            Some(b) => {
                let layout = if b.is_pads() { "pads" } else { "strips" };
                let groups: Vec<Value> = b
                    .sorted()
                    .into_iter()
                    .map(|g| {
                        let hidden = t.group_hidden(&g.key);
                        let members: Vec<Value> = t
                            .group_order(g)
                            .into_iter()
                            .map(|i| {
                                let m = &g.members[i];
                                let meta = if hidden {
                                    serde_json::Map::new()
                                } else {
                                    crate::share::public_meta(m.meta.iter().map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), str::to_owned))))
                                };
                                let nums = if hidden { Value::Object(serde_json::Map::new()) } else { serde_json::to_value(&m.numbers).unwrap_or_default() };
                                let chip = if hidden { Value::Null } else { serde_json::to_value(&m.chip).unwrap_or_default() };
                                member(&m.item, t.board_label(g, i), if hidden { "" } else { &m.role }, nums, chip, meta)
                            })
                            .collect();
                        serde_json::json!({"title": g.title(), "subtitle": g.subtitle, "chip": g.chip, "layout": layout, "members": members})
                    })
                    .collect();
                let labels: serde_json::Map<String, Value> = b.metrics.iter().filter_map(|m| m.label.clone().map(|l| (m.key.clone(), Value::String(l)))).collect();
                (groups, layout, labels)
            }
            None => {
                let members: Vec<Value> = t
                    .shown()
                    .into_iter()
                    .filter_map(|i| {
                        let id = t.spec.items.get(i)?;
                        let name = self.items.get(id).map_or_else(|| id.rsplit('/').next().unwrap_or(id).to_owned(), |e| e.item.name.clone());
                        let label = if t.spec.items.len() > 1 { if t.is_blind_hidden() { t.member_label(i) } else { format!("{} · {name}", t.member_label(i)) } } else { name };
                        let meta = if t.is_blind_hidden() {
                            serde_json::Map::new()
                        } else {
                            crate::share::public_meta(self.items.get(id).map(|e| e.item.meta.clone()).unwrap_or_default())
                        };
                        Some(member(id, label, "", Value::Object(serde_json::Map::new()), Value::Null, meta))
                    })
                    .collect();
                (vec![serde_json::json!({"title": title, "layout": "strips", "members": members})], "strips", serde_json::Map::new())
            }
        };
        let created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let kind = match (t.board(), layout) {
            (Some(_), "pads") => "pads",
            (Some(_), _) => "board",
            _ => "compare",
        };
        let page = serde_json::json!({"title": title, "kind": kind, "note": t.spec.note, "created": created, "layout": layout, "labels": labels, "groups": groups});
        Some(crate::share::Export { title, page, files, stems: stem_files })
    }

    /// Asks the publisher which shares are still up; ones taken down elsewhere get marked and
    /// their S3 audio cleared.
    pub fn check_shares(&mut self) {
        if self.cfg.share.command().is_none() || self.headless {
            self.shares_checked = Some(Ok(()));
            return;
        }
        let cfg = self.cfg.share.clone();
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        self.shares_checked = None;
        std::thread::spawn(move || {
            let _ = tx.send(Event::Live { result: crate::share::live(&cfg) });
            if let Some(w) = waker {
                w();
            }
        });
    }

    /// Takes a share down: the page and its audio. The first press asks; the second does it.
    pub fn unshare(&mut self, slug: &str, confirmed: bool) {
        if !confirmed && self.confirm_unshare.as_deref() != Some(slug) {
            self.confirm_unshare = Some(slug.to_owned());
            return;
        }
        self.confirm_unshare = None;
        let Some(sh) = self.shares.iter().find(|s| s.slug == slug).cloned() else { return };
        self.unsharing.insert(slug.to_owned());
        let (cfg, aws) = (self.cfg.share.clone(), self.cfg.aws.clone());
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        let slug = slug.to_owned();
        std::thread::spawn(move || {
            let result = crate::share::remove(&sh, &cfg, &aws);
            let _ = tx.send(Event::Unshared { slug, result });
            if let Some(w) = waker {
                w();
            }
        });
    }

    pub fn open_shared(&mut self) {
        if let Some(id) = self.tabs.iter().find(|t| t.spec.kind == TabKind::Shared).map(Tab::id) {
            self.focus(id);
            self.check_shares();
        } else {
            self.open(TabKind::Shared, Vec::new(), Some("Shared".into()), false, false, None, None);
        }
    }

    /// A framed 2x screenshot of the front tab, saved to ~/Pictures/adeck and copied.
    pub fn screenshot(&mut self) {
        let Some(t) = self.current() else { return };
        if self.shooting {
            return;
        }
        let (tab, label) = (t.id(), crate::ui::tab_title(self, t));
        let (w, h, dark) = self.win.get();
        self.save();
        self.shooting = true;
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let result = crate::shot::render(tab, &format!("{}x{}", w.round(), h.round()), if dark { "dark" } else { "light" }, 2)
                .and_then(|png| crate::shot::frame(&png, dark, 2))
                .and_then(|png| crate::shot::keep(&png, &label));
            let _ = tx.send(Event::Shot { result });
            if let Some(w) = waker {
                w();
            }
        });
    }

    pub fn set_volume(&mut self, v: f32) {
        self.mixer.set_volume(v);
    }

    /// Flips playback between 1× and 2×.
    pub fn toggle_speed(&mut self) {
        let v = if self.mixer.speed > 1.0 { 1.0 } else { 2.0 };
        self.mixer.set_speed(v);
        self.toast = Some((format!("Playing at {v:.0}×"), false));
    }

    /// Saves a copy of a file wherever you pick, Downloads to start with.
    pub fn download(&mut self, id: &str, tx: &mut rune::Tx) {
        let Some(e) = self.entry(id) else { return };
        let src = e.item.path.clone();
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned());
        let opts = rune::FileDialogOptions {
            title: "Save a copy".into(),
            directory: std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Downloads")),
            file_name: name,
            ..Default::default()
        };
        tx.save_file(opts, move |s: &mut App, r, _tx: &mut rune::Tx| match r {
            Ok(Some(dst)) => {
                s.toast = Some(match std::fs::copy(&src, &dst) {
                    Ok(_) => (format!("Saved {}", dst.display()), false),
                    Err(e) => (format!("Couldn't save: {e}"), true),
                });
            }
            Ok(None) => {}
            Err(e) => s.toast = Some((format!("Couldn't open the save dialog: {e:?}"), true)),
        });
    }

    /// Keeps a zoomed view on the playhead while it plays: it turns the page when the playhead
    /// runs off the right edge.
    fn follow_playhead(&mut self) {
        let Some(t) = self.current() else { return };
        let Some((a, b)) = t.view else { return };
        if !self.mixer_is_for(t.id()) || !self.mixer.is_playing() {
            return;
        }
        let ph = self.mixer.secs();
        if ph >= a && ph <= b {
            return;
        }
        let total = crate::ui::widgets::tab_secs(self, t);
        let span = b - a;
        let na = (ph - span * 0.05).clamp(0.0, (total - span).max(0.0));
        if let Some(t) = self.current_mut() {
            t.view = Some((na, na + span));
        }
    }

    // The frame.

    pub fn poll(&mut self, clock: f64, tx: &mut Tx) -> bool {
        let mut changed = self.drain();
        let (_, jobs_changed) = self.jobs.drain(clock);
        changed |= jobs_changed;
        changed |= self.mixer.tick();
        if self.mixer.take_lost() {
            // The output went away or changed: reload onto the new one, at the same spot.
            let (at, was) = (self.mixer.secs(), self.mixer.is_playing() || self.mixer.wants_play());
            self.mixer.unload();
            self.sync_mixer();
            self.mixer.seek_secs(at);
            if was {
                self.mixer.play();
            }
            changed = true;
        }
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
            self.follow_playhead();
            // 60 fps while sound plays, so the playhead glides; nothing at rest.
            tx.wake_after(std::time::Duration::from_millis(16));
            changed = true;
        }
        if self.calls.is_some()
            && let Some(wait) = self.dock.show(self.mood())
        {
            tx.wake_after(wait);
        }
        if let Some((_, err)) = &self.toast {
            let life = Self::toast_life(*err);
            let up = self.toast_seen.borrow().as_ref().map(|(_, at)| at.elapsed());
            match up {
                Some(up) if up >= life => {
                    self.toast = None;
                    *self.toast_seen.borrow_mut() = None;
                    changed = true;
                }
                Some(up) => tx.wake_after(life - up),
                None => {}
            }
        }
        // Finished jobs leave the footer after a few seconds; come back to fold the next one,
        // and redraw when one goes.
        self.clock = clock;
        let fresh = self
            .jobs
            .list()
            .iter()
            .filter(
                |j| matches!(j.phase, crate::jobs::Phase::Done { outcome: crate::jobs::Outcome::Completed(_) | crate::jobs::Outcome::Stopped(_), at } if at + FOOTER_SECS > clock),
            )
            .count();
        if fresh != self.fresh_jobs {
            self.fresh_jobs = fresh;
            changed = true;
        }
        if let Some(next) = self
            .jobs
            .list()
            .iter()
            .filter_map(|j| match j.phase {
                crate::jobs::Phase::Done { outcome: crate::jobs::Outcome::Completed(_) | crate::jobs::Outcome::Stopped(_), at } if at + FOOTER_SECS > clock => {
                    Some(at + FOOTER_SECS - clock)
                }
                _ => None,
            })
            .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.min(x))))
        {
            tx.wake_after(std::time::Duration::from_secs_f64(next.max(0.05)));
        }
        if self.dirty {
            self.dirty = false;
            self.save();
        }
        changed
    }

    /// What the Dock icon shows: sound playing, the agent waiting on a verdict, the agent at work
    /// (a tool call in the last few seconds, or analysis running), or nothing.
    fn mood(&self) -> crate::dock::Mood {
        use crate::dock::Mood;
        if self.mixer.is_playing() {
            Mood::Playing
        } else if self.activity.waiting() {
            Mood::Waiting
        } else if self.activity.idle_secs() < 4.0 || self.jobs.list().iter().any(crate::jobs::Job::is_active) {
            Mood::Working
        } else {
            Mood::Idle
        }
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
                        if let Some(st) = crate::stems::find(&item.path) {
                            self.stems.insert(id.clone(), st);
                        }
                        self.items.insert(id.clone(), Entry::new(item));
                        // The track whose lanes were open when the session was saved.
                        if self.current().is_some_and(|t| t.stems_open && t.spec.items.get(t.active) == Some(&id)) {
                            self.prepare_stems();
                        }
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
                    Ok(a) => {
                        entry.thumb.set(360, 48, crate::ui::widgets::paint_wave(&a.envelope, 360, 48));
                        entry.full.set(false);
                        entry.tile_key.set((0, 0, 0, 0));
                        let (lo, hi) = board::sound_range(&a.envelope, a.secs, -45.0);
                        entry.sound = Some((lo, hi));
                        let n = a.envelope.len();
                        let (ca, cb) = ((lo / a.secs * n as f64) as usize, ((hi / a.secs * n as f64).ceil() as usize).min(n));
                        if cb > ca {
                            entry.zoom.set(360, 48, crate::ui::widgets::paint_wave(&a.envelope[ca..cb], 360, 48));
                        }
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
                    // A measure's job row already says what failed; no toast per take.
                    Err(e) if cmd != Earshot::Measure => self.toast = Some((e, true)),
                    Err(_) => {}
                }
            }
            Event::Shared { result } => {
                self.sharing = false;
                match result {
                    Ok(sh) => {
                        let url = sh.url.clone();
                        self.shares.insert(0, sh);
                        crate::share::save(&self.shares);
                        let _ = std::process::Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn().and_then(|mut c| {
                            use std::io::Write;
                            c.stdin.take().map_or(Ok(()), |mut i| i.write_all(url.as_bytes()))?;
                            c.wait().map(|_| ())
                        });
                        self.toast = Some((format!("Shared · link copied · {url}"), false));
                    }
                    Err(e) => self.toast = Some((e, true)),
                }
            }
            Event::Unshared { slug, result } => {
                self.unsharing.remove(&slug);
                match result {
                    Ok(()) => {
                        self.shares.retain(|s| s.slug != slug);
                        crate::share::save(&self.shares);
                        self.toast = Some(("Taken down, audio and all".into(), false));
                    }
                    Err(e) => self.toast = Some((e, true)),
                }
            }
            Event::Live { result } => match result {
                Ok(live) => {
                    // Taken down elsewhere: mark it, and clear its S3 audio so nothing lingers.
                    let mut cleaned = Vec::new();
                    for sh in &mut self.shares {
                        let gone = !live.contains(&sh.slug);
                        if gone && !sh.gone {
                            sh.gone = true;
                            cleaned.push(sh.clone());
                        }
                    }
                    crate::share::save(&self.shares);
                    self.shares_checked = Some(Ok(()));
                    let (cfg, aws) = (self.cfg.share.clone(), self.cfg.aws.clone());
                    if !cleaned.is_empty() {
                        std::thread::spawn(move || {
                            for sh in cleaned {
                                let _ = crate::share::clean_audio(&sh, &cfg, &aws);
                            }
                        });
                    }
                }
                Err(e) => self.shares_checked = Some(Err(e)),
            },
            Event::Split { path, error } => {
                self.splitting.remove(&path);
                if error.is_none()
                    && let Some(st) = crate::stems::find(&path)
                {
                    let ids: Vec<String> = self.items.iter().filter(|(_, e)| e.item.path == path).map(|(id, _)| id.clone()).collect();
                    for id in ids {
                        self.stems.insert(id, st.clone());
                    }
                    // The one whose lanes are waiting: load its stems now.
                    if self.current().is_some_and(|t| self.open_stems(t).is_some_and(|o| o.files == st.files)) {
                        self.prepare_stems();
                        self.sync_mixer();
                    }
                }
            }
            Event::Shot { result } => {
                self.shooting = false;
                self.toast = Some(match result {
                    Ok(p) => (format!("Screenshot copied · saved to {}", p.display().to_string().replace(&std::env::var("HOME").unwrap_or_default(), "~")), false),
                    Err(e) => (e, true),
                });
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
        if t.spec.board.is_some() {
            t.spec.blind ^= true;
            t.spec.salt = feedback::now().to_bits();
            t.spec.revealed_groups.clear();
            self.dirty = true;
            return;
        }
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

    /// Saves the session on the next poll.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Opens the pane on your note.
    pub fn show_note(&mut self) {
        self.pane_tab = "NOTE".into();
        if self.pane == crate::ui::px::Dock::Hidden {
            self.pane = self.pane_last;
        }
    }

    /// ⌘1 to ⌘5: opens the pane on its nth section (bringing it back if hidden).
    pub fn pane_section(&mut self, n: usize) -> bool {
        let Some(name) = self.current().and_then(|t| crate::ui::pane::sections(t).get(n).copied()) else { return false };
        self.pane_tab = name.to_owned();
        if self.pane == crate::ui::px::Dock::Hidden {
            self.pane = self.pane_last;
        }
        true
    }

    /// Docks the pane, or hides it.
    pub fn set_pane(&mut self, d: crate::ui::px::Dock) {
        self.pane = d;
        self.dirty = true;
    }

    /// `\\`: hides the pane, or brings it back where it was.
    pub fn toggle_pane(&mut self) {
        self.pane = if self.pane == crate::ui::px::Dock::Hidden {
            self.pane_last
        } else {
            self.pane_last = self.pane;
            crate::ui::px::Dock::Hidden
        };
        self.dirty = true;
    }

    pub fn save(&self) {
        if self.headless {
            return;
        }
        let specs: Vec<&TabSpec> = self.tabs.iter().map(|t| &t.spec).collect();
        let v = json!({ "tabs": specs, "active": self.active, "next_tab": self.next_tab, "pane": self.pane, "pane_w": self.sheet_w });
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
        self.pane = serde_json::from_value(v["pane"].clone()).unwrap_or_default();
        if let Some(w) = v["pane_w"].as_f64() {
            self.sheet_w = (w as f32).clamp(280.0, 900.0);
        }
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
            "list_runs" | "list_items" | "get_item" => read_tool(&self.cfg, &self.scores, tool, a),
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
            "open_board" | "open_diff" => {
                let blind = a.get("blind").and_then(Value::as_bool).unwrap_or(false);
                let lm = a.get("loudness_match").and_then(Value::as_bool).unwrap_or(true);
                let (b, d) = if tool == "open_diff" {
                    let d: Diff = serde_json::from_value(json!({
                        "a": a.get("a").cloned().unwrap_or(json!("A")), "b": a.get("b").cloned().unwrap_or(json!("B")),
                        "metrics": a.get("metrics").cloned().unwrap_or(json!([])), "rows": a.get("rows").cloned().unwrap_or(json!([])),
                    }))
                    .map_err(|e| format!("Couldn't read the diff: {e}"))?;
                    (None, Some(d))
                } else {
                    let b: Board = serde_json::from_value(json!({
                        "groups": a.get("groups").cloned().unwrap_or(json!([])), "metrics": a.get("metrics").cloned().unwrap_or(json!([])),
                        "choices": a.get("choices").cloned().unwrap_or(json!([])), "move_to": a.get("move_to").cloned().unwrap_or(json!([])),
                        "layout": a.get("layout").cloned().unwrap_or(Value::Null),
                    }))
                    .map_err(|e| format!("Couldn't read the board: {e}"))?;
                    (Some(b), None)
                };
                let id = self.open_page(s("label"), b, d, blind, lm);
                if let Some(n) = s("note") {
                    self.annotate(id, n);
                }
                Ok(self.tab_json(id))
            }
            "open_pads" => {
                let items = ids("items");
                let groups = match a.get("groups") {
                    Some(g) => g.clone(),
                    None if !items.is_empty() => {
                        json!([{ "key": "pads", "title": s("label").unwrap_or_else(|| "Pads".into()), "members": items.iter().map(|i| json!({ "item": i })).collect::<Vec<_>>() }])
                    }
                    None => return Err("open_pads needs items or groups.".into()),
                };
                let b: Board = serde_json::from_value(json!({
                    "groups": groups, "metrics": a.get("metrics").cloned().unwrap_or(json!([])),
                    "choices": a.get("choices").cloned().unwrap_or(json!([])), "move_to": a.get("move_to").cloned().unwrap_or(json!([])), "layout": "pads",
                }))
                .map_err(|e| format!("Couldn't read the pads: {e}"))?;
                let blind = a.get("blind").and_then(Value::as_bool).unwrap_or(false);
                let lm = a.get("loudness_match").and_then(Value::as_bool).unwrap_or(true);
                let id = self.open_page(s("label").or_else(|| Some("Pads".into())), Some(b), None, blind, lm);
                if let Some(n) = s("note") {
                    self.annotate(id, n);
                }
                Ok(self.tab_json(id))
            }
            "board_upsert" => {
                let id = tab_arg().ok_or("tab_id is required")?;
                self.upsert(id, a)
            }
            "get_selection" => {
                let since = a.get("since").and_then(Value::as_f64).unwrap_or(0.0);
                let n = a.get("limit").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
                let rows: Vec<Row> = self.feedback.read_all().into_iter().filter(|r| r.kind == "reference" && r.ts > since).collect();
                let last: Vec<Value> = rows.iter().rev().take(n).map(|r| json!({ "ts": r.ts, "note": r.note, "ref": r.value })).collect();
                Ok(json!({ "references": last, "total": rows.len() }))
            }
            "get_decisions" => {
                let mut items = ids("items");
                if let Some(id) = tab_arg() {
                    let t = self.tab(id).ok_or_else(|| format!("No tab {id}."))?;
                    items.extend(t.board().map(Board::items).unwrap_or_else(|| t.spec.items.clone()));
                }
                let rows = self.feedback.read_all();
                let mut out = serde_json::Map::new();
                for r in rows.iter().filter(|r| r.kind == "decision") {
                    if let Some(i) = r.items.first().filter(|i| items.is_empty() || items.contains(i)) {
                        out.insert(i.clone(), json!({ "decision": r.value, "note": r.note, "ts": r.ts }));
                    }
                }
                Ok(json!({ "count": out.len(), "decisions": out }))
            }
            "list_tabs" => match tab_arg() {
                Some(id) => self.tab(id).map(|_| self.tab_json(id)).ok_or_else(|| format!("No tab {id}.")),
                None => Ok(json!({ "active": self.active, "tabs": self.tabs.iter().map(|t| self.tab_brief(t)).collect::<Vec<_>>() })),
            },
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
                if let Some(g) = s("group") {
                    if self.tab(id).and_then(Tab::board).and_then(|b| b.group(&g)).is_none() {
                        return Err(format!("No group {g} on tab {id}."));
                    }
                    self.focus_group(&g);
                }
                if let Some(m) = a.get("member").and_then(Value::as_u64) {
                    self.set_active_member(m as usize);
                }
                if let Some(at) = a.get("at_s").and_then(Value::as_f64) {
                    self.seek_secs(at);
                }
                // Stems: open the lanes of the member playing, soloing any named.
                let solo = ids("solo");
                if a.get("stems").and_then(Value::as_bool).unwrap_or(!solo.is_empty()) {
                    let t = self.tab(id).ok_or("No tab.")?;
                    let key = t.focused().map(|g| g.key.clone()).ok_or("stems are for board tabs")?;
                    let (m, open) = (t.active, t.stems_open);
                    if !open {
                        self.toggle_stems(&key, m);
                    }
                    if let Some(t) = self.current_mut() {
                        t.stem_solo = std::array::from_fn(|i| solo.iter().any(|x| x == crate::stems::NAMES[i]));
                        t.stem_mute = [false; 4];
                        t.stem_mode = true;
                    }
                    self.apply_mix();
                }
                self.sync_mixer();
                self.mixer.play();
                self.sync_videos(true);
                let stems = self.tab(id).filter(|t| t.stems_open).map(|t| json!({"loaded": self.stems_loaded(t), "on_disk": self.open_stems(t).is_some()}));
                Ok(json!({ "playing": true, "tab_id": id, "loading": self.mixer.is_loading(), "stems": stems }))
            }
            "split_stems" => {
                let id = tab_arg().or(self.active).ok_or("No tab is open.")?;
                let t = self.tab(id).ok_or_else(|| format!("No tab {id}."))?;
                let mut want = ids("items");
                if want.is_empty() {
                    want = t.board().map_or_else(|| t.spec.items.clone(), Board::items);
                }
                let (queued, have) = self.split_stems(want)?;
                Ok(json!({"queued": queued, "already_split": have, "splitting": self.splitting.len(), "model": self.cfg.stems.model}))
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
            "_export" => {
                if let Some(id) = tab_arg() {
                    self.focus(id);
                }
                let x = self.export().ok_or("That tab can't be shared: a compare, single, render or board can.")?;
                Ok(json!({"title": x.title, "page": x.page, "files": x.files, "stems": x.stems}))
            }
            "_shared" => {
                let sh: crate::share::Share = serde_json::from_value(a.clone()).map_err(|e| e.to_string())?;
                self.shares.insert(0, sh);
                crate::share::save(&self.shares);
                Ok(json!({}))
            }
            "_unshared" => {
                let slug = s("slug").ok_or("slug is required")?;
                self.shares.retain(|x| x.slug != slug);
                crate::share::save(&self.shares);
                Ok(json!({}))
            }
            "list_shares" => Ok(json!({"shares": self.shares})),
            "open_shared" => {
                self.open_shared();
                Ok(json!({"tab_id": self.active}))
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

    /// One line about a tab for list_tabs: enough to pick it, not its members.
    pub fn tab_brief(&self, t: &Tab) -> Value {
        let mut v = json!({ "tab_id": t.id(), "kind": t.spec.kind, "label": crate::ui::tab_title(self, t) });
        match (t.board(), t.spec.kind) {
            (Some(b), _) => v["groups"] = json!(b.groups.len()),
            (None, TabKind::Run) => v["rows"] = json!(t.rows.len()),
            (None, TabKind::Shared | TabKind::Diff) => {}
            _ => v["files"] = json!(t.spec.items.len()),
        }
        if t.spec.blind {
            v["blind"] = json!(true);
        }
        v
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
                    "stems": self.stems.get(i).map(|st| {
                        crate::stems::NAMES.iter().enumerate().map(|(k, n)| ((*n).to_owned(), json!({"lufs": st.lufs[k], "share": st.share[k]}))).collect::<serde_json::Map<String, Value>>()
                    }),
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
            "board": t.board().map(|b| json!({
                "groups": b.groups.len(),
                "focused": t.spec.group,
                "tally": b.tally().into_iter().map(|(k, n)| (k, Value::from(n))).collect::<serde_json::Map<String, Value>>(),
                "decided": b.items().iter().filter(|i| self.decisions.contains_key(*i)).count(),
            })),
            "diff": t.spec.diff.as_ref().map(|d| json!({ "a": d.a, "b": d.b, "rows": d.rows.len() })),
        })
    }
}

/// The read-only tools: they touch the disk, the bucket and the cache but nothing on screen, so
/// the MCP thread runs them and a slow S3 listing never stalls the window.
pub fn read_tool(cfg: &Config, scores: &HashMap<String, i64>, tool: &str, a: &Value) -> Result<Value, String> {
    let s = |k: &str| a.get(k).and_then(Value::as_str).map(str::to_owned);
    match tool {
        "list_runs" => {
            let mut runs = source::list_runs(cfg);
            if a.get("include_s3").and_then(Value::as_bool).unwrap_or(false) {
                runs.extend(source::list_s3_runs(cfg)?);
            }
            Ok(json!({ "runs": runs }))
        }
        "list_items" => {
            let run = s("run").ok_or("run is required")?;
            let filter = s("filter").map(|f| f.to_ascii_lowercase());
            let items = source::list_items(cfg, &run)?;
            let rows: Vec<Value> = items
                .iter()
                .filter(|i| filter.as_ref().is_none_or(|f| i.name.to_ascii_lowercase().contains(f) || i.meta.iter().any(|(_, v)| v.to_ascii_lowercase().contains(f))))
                .map(|i| json!({ "id": i.id, "name": i.name, "summary": i.subtitle(), "score": scores.get(&i.id) }))
                .collect();
            Ok(json!({ "run": run, "count": rows.len(), "items": rows }))
        }
        "get_item" => {
            let id = s("id").ok_or("id is required")?;
            let item = source::resolve(cfg, &id)?;
            let analysis = derive::analyse(&item.path).ok();
            let earshot: serde_json::Map<String, Value> = [Earshot::Describe, Earshot::Measure, Earshot::Critique]
                .into_iter()
                .filter_map(|c| derive::earshot_cached(&item.path, c).map(|v| (c.name().to_owned(), summarise_earshot(c, &v))))
                .collect();
            let mut v = item.to_json();
            v["analysis"] = analysis.map_or(Value::Null, |a| a.to_json());
            v["earshot"] = Value::Object(earshot);
            v["score"] = scores.get(&id).map_or(Value::Null, |s| Value::from(*s));
            Ok(v)
        }
        other => Err(format!("No tool {other}.")),
    }
}

/// A stem file's item id: its path.
pub fn stem_id(p: &std::path::Path) -> String {
    p.display().to_string()
}

/// The members a tab decodes for playback, by index into its items: all of them, except that a
/// board decodes its folded rest only one at a time, as each is played.
fn slots(t: &Tab) -> Vec<usize> {
    match t.focused() {
        Some(g) => {
            let mut v: Vec<usize> = (0..g.members.len().min(t.spec.items.len())).collect();
            if t.active >= g.members.len() && t.active < t.spec.items.len() {
                v.push(t.active);
            }
            v
        }
        None => (0..t.spec.items.len()).collect(),
    }
}

/// A shuffle that depends only on its seed.
fn stable_shuffle(n: usize, seed: u64) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    let mut x = seed | 1;
    for i in (1..n).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
    v
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
            board: None,
            group: None,
            revealed_groups: Vec::new(),
            salt: 0,
            diff: None,
            stems: None,
        };
        let t = Tab::new(spec);
        assert_eq!(t.member_label(2), "X");
        assert_eq!(t.member_label(0), "Y");
        assert_eq!(t.member_label(1), "Z");
    }
}
