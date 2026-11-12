//! The MCP server: streamable HTTP on 127.0.0.1, JSON responses (no SSE stream).
//!
//! Each request runs on its own thread. Tools that read or change what the window shows are
//! handed to the UI thread as a `Call` and answered from `App::poll`, so a tool and the window
//! never disagree. Three run here instead, because they wait: `wait_for_feedback` blocks on the
//! feedback signal, `screenshot_tab` renders the tab in a headless copy of adeck, and `analyze`
//! runs earshot.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::config::Config;
use crate::derive::{self, Earshot};
use crate::feedback::{self, Signal};

pub struct Call {
    pub tool: String,
    pub args: Value,
    pub reply: Sender<Result<Value, String>>,
}

const PROTOCOL: &str = "2025-06-18";

const INSTRUCTIONS: &str = "adeck is a listening bench on the user's Mac. Open tabs to put sounds in front of them: \
open_single for one file (waveform, spectrogram, all metadata, earshot numbers), open_compare for an A/B/N \
compare (synced playback, instant switching, loudness-matched by default, optional blind mode), open_render for \
videos, open_run for a whole run as a table. Item ids are absolute paths, run:<run>/<path>, ref:<path> (the \
local-only reference pack) or s3:// URLs; list_runs and list_items find them. Pin what to listen for with \
annotate_tab. The user scores (0-3), picks and writes notes in the window; read them with get_feedback, or block \
until they've judged with wait_for_feedback. screenshot_tab shows you what they see.";

/// What the agent is up to, for the Dock icon: when it last called a tool, and how many calls are
/// blocked waiting on the user's verdict.
#[derive(Default)]
pub struct Activity {
    last_ms: std::sync::atomic::AtomicU64,
    waiting: std::sync::atomic::AtomicUsize,
}

impl Activity {
    fn touch(&self) {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        self.last_ms.store(ms, std::sync::atomic::Ordering::Relaxed);
    }
    /// Seconds since the last tool call.
    pub fn idle_secs(&self) -> f64 {
        let last = self.last_ms.load(std::sync::atomic::Ordering::Relaxed);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        if last == 0 { f64::INFINITY } else { now.saturating_sub(last) as f64 / 1000.0 }
    }
    pub fn waiting(&self) -> bool {
        self.waiting.load(std::sync::atomic::Ordering::Relaxed) > 0
    }
}

pub fn serve(cfg: Config, feedback_path: PathBuf, signal: Signal, activity: Arc<Activity>, waker: rune::Waker) -> Result<Receiver<Call>, String> {
    let addr = format!("127.0.0.1:{}", cfg.port);
    let server = tiny_http::Server::http(&addr).map_err(|e| e.to_string())?;
    let (tx, rx) = channel::<Call>();
    std::thread::Builder::new()
        .name("adeck-mcp".into())
        .spawn(move || {
            for req in server.incoming_requests() {
                let ctx = Ctx { cfg: cfg.clone(), tx: tx.clone(), waker: waker.clone(), feedback_path: feedback_path.clone(), signal: signal.clone(), activity: activity.clone() };
                std::thread::spawn(move || handle(req, &ctx));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}

#[derive(Clone)]
struct Ctx {
    cfg: Config,
    tx: Sender<Call>,
    waker: rune::Waker,
    feedback_path: PathBuf,
    signal: Signal,
    activity: Arc<Activity>,
}

impl Ctx {
    /// Asks the UI thread to run a tool and waits for its answer.
    fn ui(&self, tool: &str, args: Value) -> Result<Value, String> {
        let (reply, answer) = channel();
        self.tx.send(Call { tool: tool.to_owned(), args, reply }).map_err(|_| "adeck is closing.".to_owned())?;
        (self.waker)();
        answer.recv_timeout(Duration::from_secs(120)).map_err(|_| "adeck didn't answer in time.".to_owned())?
    }
}

fn handle(mut req: tiny_http::Request, ctx: &Ctx) {
    let header = |k: &str, v: &str| tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()).expect("ascii header");
    let path = req.url().split('?').next().unwrap_or("").to_owned();
    if path != "/mcp" && path != "/" {
        let _ = req.respond(tiny_http::Response::from_string("adeck MCP lives at /mcp").with_status_code(404));
        return;
    }
    match req.method() {
        tiny_http::Method::Post => {}
        tiny_http::Method::Delete => {
            let _ = req.respond(tiny_http::Response::empty(200));
            return;
        }
        _ => {
            let _ = req.respond(tiny_http::Response::empty(405).with_header(header("Allow", "POST, DELETE")));
            return;
        }
    }
    let mut body = String::new();
    if req.as_reader().read_to_string(&mut body).is_err() {
        let _ = req.respond(tiny_http::Response::from_string("bad body").with_status_code(400));
        return;
    }
    let parsed: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            let err = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}});
            let _ = req.respond(tiny_http::Response::from_string(err.to_string()).with_status_code(400).with_header(header("Content-Type", "application/json")));
            return;
        }
    };
    let replies: Vec<Value> = match &parsed {
        Value::Array(msgs) => msgs.iter().filter_map(|m| rpc(m, ctx)).collect(),
        m => rpc(m, ctx).into_iter().collect(),
    };
    let is_init = parsed.get("method").and_then(Value::as_str) == Some("initialize");
    if replies.is_empty() {
        let _ = req.respond(tiny_http::Response::empty(202));
        return;
    }
    let out = if parsed.is_array() { Value::Array(replies) } else { replies.into_iter().next().unwrap_or(Value::Null) };
    let mut resp = tiny_http::Response::from_string(out.to_string()).with_header(header("Content-Type", "application/json"));
    if is_init {
        resp = resp.with_header(header("Mcp-Session-Id", &format!("adeck-{}", std::process::id())));
    }
    let _ = req.respond(resp);
}

/// One JSON-RPC message. Notifications get no reply.
fn rpc(m: &Value, ctx: &Ctx) -> Option<Value> {
    let id = m.get("id").cloned();
    let method = m.get("method").and_then(Value::as_str).unwrap_or("");
    let params = m.get("params").cloned().unwrap_or(Value::Null);
    let id = id?;
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL),
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "adeck", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            Ok(call_tool(name, &args, ctx))
        }
        "resources/list" => Ok(json!({"resources": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        _ => Err((-32601, format!("no method {method}"))),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, msg)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
        }
    })
}

fn text_result(v: Result<Value, String>) -> Value {
    match v {
        Ok(v) => {
            json!({"content": [{"type": "text", "text": serde_json::to_string(&lean(v)).unwrap_or_default()}]})
        }
        Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
    }
}

/// Replies are read by a model: compact JSON with the nulls left out, since every byte is a token.
fn lean(v: Value) -> Value {
    match v {
        Value::Object(o) => Value::Object(o.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, lean(v))).collect()),
        Value::Array(a) => Value::Array(a.into_iter().map(lean).collect()),
        v => v,
    }
}

fn call_tool(name: &str, args: &Value, ctx: &Ctx) -> Value {
    ctx.activity.touch();
    (ctx.waker)();
    match name {
        "wait_for_feedback" => {
            ctx.activity.waiting.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let r = wait_for_feedback(args, ctx);
            ctx.activity.waiting.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            ctx.activity.touch();
            (ctx.waker)();
            text_result(r)
        }
        "screenshot_tab" => match screenshot(args, ctx) {
            Ok(png) => {
                json!({"content": [{"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(png), "mimeType": "image/png"}]})
            }
            Err(e) => text_result(Err(e)),
        },
        "analyze" => text_result(analyze(args, ctx)),
        "share_tab" => text_result(share_tab(args, ctx)),
        "unshare" => text_result(unshare(args, ctx)),
        "list_runs" | "list_items" | "get_item" => {
            let scores = feedback::latest_scores(&feedback::read(&ctx.feedback_path));
            text_result(crate::app::read_tool(&ctx.cfg, &scores, name, args))
        }
        _ if tools().iter().any(|t| t["name"] == name) => text_result(ctx.ui(name, args.clone())),
        _ => text_result(Err(format!("No tool {name}."))),
    }
}

fn wait_for_feedback(args: &Value, ctx: &Ctx) -> Result<Value, String> {
    let tab = args.get("tab_id").and_then(Value::as_u64);
    let timeout = Duration::from_secs_f64(args.get("timeout_s").and_then(Value::as_f64).unwrap_or(300.0).clamp(1.0, 3600.0));
    let start = feedback::now();
    let deadline = std::time::Instant::now() + timeout;
    let mut seen = ctx.signal.count();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Ok(json!({"rows": [], "timed_out": true}));
        }
        let now = ctx.signal.wait_past(seen, left);
        if now > seen {
            seen = now;
            let rows = feedback::select(&feedback::read(&ctx.feedback_path), tab, &[], start - 0.001);
            if !rows.is_empty() {
                return Ok(json!({"rows": rows, "timed_out": false}));
            }
        }
    }
}

/// Renders a tab as the window would, in a headless copy of adeck reading the same session.
fn screenshot(args: &Value, ctx: &Ctx) -> Result<Vec<u8>, String> {
    let saved = ctx.ui("_save", json!({}))?;
    let tab = args.get("tab_id").and_then(Value::as_u64).or_else(|| saved["active"].as_u64()).ok_or("No tab is open.")?;
    let size = args.get("size").and_then(Value::as_str).unwrap_or("1280x800").to_owned();
    let theme = args.get("theme").and_then(Value::as_str).unwrap_or("dark");
    if args.get("pretty").and_then(Value::as_bool).unwrap_or(false) {
        let png = crate::shot::frame(&crate::shot::render(tab, &size, theme, 2)?, theme == "dark", 2)?;
        crate::shot::keep(&png, &format!("tab {tab}"))?;
        return Ok(png);
    }
    crate::shot::render(tab, &size, theme, 1)
}

/// Publishes a tab as a page, on this thread (it uploads), and records it in the app.
fn share_tab(args: &Value, ctx: &Ctx) -> Result<Value, String> {
    let v = ctx.ui("_export", args.clone())?;
    let files: Vec<(PathBuf, String)> = serde_json::from_value(v["files"].clone()).map_err(|e| e.to_string())?;
    let stems: Vec<Vec<(PathBuf, String)>> = serde_json::from_value(v["stems"].clone()).unwrap_or_default();
    let x = crate::share::Export { title: v["title"].as_str().unwrap_or("adeck").to_owned(), page: v["page"].clone(), files, stems };
    let sh = crate::share::publish(x, &ctx.cfg.share, &ctx.cfg.aws)?;
    ctx.ui("_shared", serde_json::to_value(&sh).map_err(|e| e.to_string())?)?;
    Ok(json!({"url": sh.url, "slug": sh.slug, "files": sh.files}))
}

fn unshare(args: &Value, ctx: &Ctx) -> Result<Value, String> {
    let slug = args.get("slug").and_then(Value::as_str).ok_or("slug is required")?;
    let all = ctx.ui("list_shares", json!({}))?;
    let sh: crate::share::Share = all["shares"]
        .as_array()
        .and_then(|a| a.iter().find(|s| s["slug"] == slug))
        .and_then(|s| serde_json::from_value(s.clone()).ok())
        .ok_or_else(|| format!("No share {slug}."))?;
    crate::share::remove(&sh, &ctx.cfg.share, &ctx.cfg.aws)?;
    ctx.ui("_unshared", json!({"slug": slug}))?;
    Ok(json!({"removed": slug}))
}

fn analyze(args: &Value, ctx: &Ctx) -> Result<Value, String> {
    let id = args.get("item").and_then(Value::as_str).ok_or("item is required")?;
    let kind = args.get("kind").and_then(Value::as_str).unwrap_or("describe");
    let cmd = Earshot::parse(kind).ok_or_else(|| format!("kind is one of describe, measure, critique, timeline; not {kind}."))?;
    let path = ctx.ui("_item_path", json!({"id": id}))?;
    let path = PathBuf::from(path["path"].as_str().ok_or("no path")?);
    let v = derive::earshot(&ctx.cfg.earshot, &path, cmd)?;
    let _ = ctx.ui("_earshot_done", json!({"id": id, "kind": kind, "result": v}));
    let full = args.get("full").and_then(Value::as_bool).unwrap_or(false);
    Ok(if full { v } else { crate::app::summarise_earshot(cmd, &v) })
}

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({"name": name, "description": description, "inputSchema": {"type": "object", "properties": props, "required": required}})
}

fn metrics() -> Value {
    json!({"type": "array", "description": "Labels for the numbers, and which way is better.", "items": {"type": "object", "properties": {
        "key": {"type": "string"}, "label": {"type": "string"}, "better": {"type": "string", "enum": ["higher", "lower"]}, "help": {"type": "string"}
    }, "required": ["key"]}})
}

fn board_groups() -> Value {
    let chip = json!({"type": "object", "properties": {"text": {"type": "string"}, "tone": {"type": "string", "enum": ["good", "close", "bad", "other"]}}});
    let member = json!({"type": "object", "properties": {
        "item": {"type": "string", "description": "Item id."},
        "role": {"type": "string", "description": "pick | before | ref | anything."},
        "label": {"type": "string"},
        "numbers": {"type": "object", "description": "Your scores, shown in this order: {\"vibe\": 1.05, \"shape\": 0.51}."},
        "chip": chip,
        "meta": {"type": "object", "description": "prompt, model, seed, chain… shown under Details."}
    }, "required": ["item"]});
    json!({"type": "array", "items": {"type": "object", "properties": {
        "key": {"type": "string"},
        "title": {"type": "string"},
        "subtitle": {"type": "string"},
        "sort": {"type": "number", "description": "Bigger first, e.g. search count."},
        "chip": chip,
        "members": {"type": "array", "items": member},
        "more": {"type": "array", "items": member, "description": "The rest, best first, folded."},
        "note": {"type": "string"}
    }, "required": ["key"]}})
}

// open_board and open_diff carry the full shapes; the tools that take the same thing say so instead
// of repeating them, which keeps tools/list small.
fn same_groups() -> Value {
    json!({"type": "array", "items": {"type": "object"}, "description": "Groups, the same shape as open_board's."})
}

fn same_metrics() -> Value {
    json!({"type": "array", "items": {"type": "object"}, "description": "Same shape as open_board's metrics."})
}

fn same_rows() -> Value {
    json!({"type": "array", "items": {"type": "object"}, "description": "Diff rows, the same shape as open_diff's."})
}

fn diff_rows() -> Value {
    json!({"type": "array", "items": {"type": "object", "properties": {
        "key": {"type": "string"}, "title": {"type": "string"}, "sort": {"type": "number"},
        "a": {"type": "object", "description": "Metric values for A."}, "b": {"type": "object"},
        "items": {"type": "object", "properties": {"a": {"type": "string"}, "b": {"type": "string"}}}
    }, "required": ["key"]}})
}

pub fn tools() -> Vec<Value> {
    let items = json!({"type": "array", "items": {"type": "string"}, "description": "Item ids: absolute paths, run:<run>/<path>, ref:<path>, or s3:// URLs."});
    let tab = json!({"type": "integer", "description": "A tab id from an open_* call or list_tabs."});
    let note = json!({"type": "string", "description": "Markdown pinned in the tab: what to listen for."});
    vec![
        tool(
            "list_runs",
            "Runs adeck knows: folders of generated audio under the configured run roots, the local-only reference pack (`refs`), and S3 runs when include_s3 is true.",
            json!({"include_s3": {"type": "boolean"}}),
            &[],
        ),
        tool(
            "list_items",
            "A run's items: id, name, a one-line summary (model, mood or type) and the user's latest score.",
            json!({"run": {"type": "string"}, "filter": {"type": "string", "description": "Case-insensitive match on name or any metadata value."}}),
            &["run"],
        ),
        tool(
            "get_item",
            "Everything about one item: flattened metadata (prompt, model, seed, flags…), native analysis (duration, LUFS, peak, BPM, key), cached earshot summaries and the latest score.",
            json!({"id": {"type": "string"}}),
            &["id"],
        ),
        tool(
            "open_single",
            "Open one file in a tab: waveform with beat grid, spectrogram, all metadata and earshot numbers. The user can score it 0-3.",
            json!({"item": {"type": "string"}, "label": {"type": "string"}, "note": note}),
            &["item"],
        ),
        tool(
            "open_compare",
            "Open 2-9 files side by side. They play in sync; keys 1-9 switch instantly at the same moment. Loudness-matched by default. Blind mode shuffles them as X/Y/Z and reveals after the user picks.",
            json!({"items": items, "blind": {"type": "boolean"}, "loudness_match": {"type": "boolean"}, "label": {"type": "string"}, "note": note}),
            &["items"],
        ),
        tool(
            "open_render",
            "Open one or more videos (renders): picture, earshot critique scorecard and sheet. Several renders switch like a compare.",
            json!({"items": items, "label": {"type": "string"}, "note": note}),
            &["items"],
        ),
        tool(
            "open_run",
            "Open a whole run as a sortable, filterable table. The user opens rows or compares a selection.",
            json!({"run": {"type": "string"}, "filter": {"type": "string"}, "label": {"type": "string"}}),
            &["run"],
        ),
        tool(
            "open_board",
            "Open a board: one group per category, each a row of members (picks, a raw \"before\", reference files) with your numbers and a verdict chip on each, a target the analysis bars compare against, and a ranked rest kept folded. Clicking a tile plays it; one plays at a time; switching keeps the position. Fill it now or later with board_upsert as results land. Reference ids (ref:) are played from this machine and never leave it.",
            json!({
                "label": {"type": "string"},
                "note": note,
                "groups": board_groups(),
                "metrics": metrics(),
                "choices": {"type": "array", "items": {"type": "string"}, "description": "Decisions offered on each file, e.g. [\"keep\", \"drop\"]. Read them back with get_decisions."},
                "move_to": {"type": "array", "items": {"type": "string"}, "description": "Groups a file can be moved to; recorded as move:<group>."},
                "blind": {"type": "boolean", "description": "Hide which member is which (X/Y/Z, shuffled) in each group until the user picks."},
                "layout": {"type": "string", "enum": ["board", "pads"], "description": "pads: big play buttons for one-shot effects, strips only on request."},
                "loudness_match": {"type": "boolean", "description": "Default true."}
            }),
            &[],
        ),
        tool(
            "open_pads",
            "Open sound pads for one-shot effects: each file is a big button that plays it from the top on a click or its key (1-9, 0, then Q-P, A-L), with 0-3 score dots and keep/drop. No strips unless the user asks for them. Give items (one row of pads) or groups (a row per category, same shape as open_board). board_upsert adds more.",
            json!({
                "label": {"type": "string"}, "note": note, "items": items, "groups": same_groups(), "metrics": same_metrics(),
                "choices": {"type": "array", "items": {"type": "string"}}, "move_to": {"type": "array", "items": {"type": "string"}},
                "blind": {"type": "boolean"}, "loudness_match": {"type": "boolean"}
            }),
            &[],
        ),
        tool(
            "board_upsert",
            "Add groups to an open board, or replace them by key, while the user watches; or rows to an open diff. remove drops groups by key.",
            json!({"tab_id": tab, "groups": same_groups(), "rows": same_rows(), "metrics": same_metrics(), "remove": {"type": "array", "items": {"type": "string"}}}),
            &["tab_id"],
        ),
        tool(
            "open_diff",
            "Open run A against run B: one row per group, one column per metric, B minus A coloured by which way is better, with a count of how many got better and worse. Rows with items open as a compare when clicked.",
            json!({"label": {"type": "string"}, "note": note, "a": {"type": "string", "description": "What A is, e.g. \"chain v1\"."}, "b": {"type": "string"}, "metrics": metrics(), "rows": diff_rows()}),
            &["a", "b"],
        ),
        tool("get_decisions", "The user's latest keep/drop/move:<group> on each file of a board (tab_id) or of the given items.", json!({"tab_id": tab, "items": items}), &[]),
        tool(
            "get_selection",
            "What the user pointed you at: they double-click a strip, tile or member (or press S / Send to agent) and it lands here with the tab, group, member, item id (absent while blind), the loop, shift-dragged selection or zoomed range and the playhead, plus any note. Newest first. wait_for_feedback also wakes on one.",
            json!({"limit": {"type": "integer", "description": "Default 1."}, "since": {"type": "number"}}),
            &[],
        ),
        tool("list_tabs", "Open tabs, one line each, and which is in front. Pass tab_id for one tab in full (members, ids, loudness).", json!({"tab_id": tab}), &[]),
        tool("focus_tab", "Bring a tab to the front.", json!({"tab_id": tab}), &["tab_id"]),
        tool("close_tab", "Close a tab.", json!({"tab_id": tab}), &["tab_id"]),
        tool(
            "annotate_tab",
            "Pin markdown in a tab (what to listen for, what changed). Empty clears it.",
            json!({"tab_id": tab, "markdown": {"type": "string"}}),
            &["tab_id", "markdown"],
        ),
        tool(
            "play",
            "Play a tab (the front one by default), optionally a board group, a member (index into the group's members then its rest) and a start time. On a board, stems: true opens that member's stem lanes and plays the stems (splitting it first if need be); solo names the stems to hear alone, e.g. [\"bass\"] (whatever the track's stems.json lists: drums, bass, other, vocals, and guitar and piano in a 6-stem split); mute names stems to leave out; hear: \"mix\" plays the full mix with the lanes open (the A/B), \"stems\" the stems; stem_set picks one of the track's splits when it has more than one, by model or folder suffix, e.g. \"htdemucs_6s\" (list_tabs shows them).",
            json!({"tab_id": tab, "group": {"type": "string"}, "member": {"type": "integer"}, "at_s": {"type": "number"}, "stems": {"type": "boolean"}, "solo": {"type": "array", "items": {"type": "string"}}, "mute": {"type": "array", "items": {"type": "string"}}, "hear": {"type": "string", "enum": ["mix", "stems"]}, "stem_set": {"type": "string"}}),
            &[],
        ),
        tool(
            "split_stems",
            "Split tracks into stems with the configured model ([stems] model: htdemucs_ft by default, for drums, bass, other and vocals; htdemucs_6s or BS-Roformer-SW add guitar and piano), in the background, once per track; the stems land beside each track in <name>.stems/ (another model's in <name>.stems-<model>/) with each stem's loudness and share. Tracks that already have any stems are skipped. Defaults to every track on the tab. The user then opens them as lanes under a board tile (solo, mute, A/B against the mix), and shared pages carry them. list_tabs shows each member's stem levels once split.",
            json!({"tab_id": tab, "items": items}),
            &[],
        ),
        tool("stop", "Pause playback.", json!({}), &[]),
        tool("set_loop", "Loop a region of a tab. Omit start_s/end_s to clear.", json!({"tab_id": tab, "start_s": {"type": "number"}, "end_s": {"type": "number"}}), &[]),
        tool(
            "screenshot_tab",
            "A PNG of a tab as the user sees it.",
            json!({"tab_id": tab, "size": {"type": "string", "description": "WxH in points, default 1280x800."}, "theme": {"type": "string", "enum": ["dark", "light"]}, "pretty": {"type": "boolean", "description": "Framed at 2x on a backdrop for sharing; also saved to ~/Pictures/adeck and copied."}}),
            &[],
        ),
        tool(
            "share_tab",
            "Publish a tab as a web page (players, waveforms, numbers) and return its url. Reference-pack audio is never uploaded; those members show as local-only. Needs [share] in the config.",
            json!({"tab_id": tab}),
            &[],
        ),
        tool("list_shares", "Every page shared from adeck: slug, title, url, when, where its audio went, and whether it's been taken down.", json!({}), &[]),
        tool("open_shared", "Open the Shared tab: every shared page, to open or take down.", json!({}), &[]),
        tool("unshare", "Take a shared page down: deletes the page and its audio.", json!({"slug": {"type": "string"}}), &["slug"]),
        tool(
            "get_feedback",
            "The user's verdicts: scores, picks (with the blind reveal), ranks and notes. Filter by tab, run or items; since is unix seconds.",
            json!({"tab_id": tab, "run": {"type": "string"}, "items": items, "since": {"type": "number"}}),
            &[],
        ),
        tool(
            "wait_for_feedback",
            "Block until the user scores, picks or notes something (in this tab, if given), then return the new rows. Use it to loop: open, wait, read, generate, open again.",
            json!({"tab_id": tab, "timeout_s": {"type": "number", "description": "Default 300."}}),
            &[],
        ),
        tool(
            "analyze",
            "Run earshot on an item (cached): describe (tags, labels), measure (loudness, peaks, spectrum), critique (scorecard vs pro films, for renders) or timeline. Returns a summary unless full is true.",
            json!({"item": {"type": "string"}, "kind": {"type": "string", "enum": ["describe", "measure", "critique", "timeline"]}, "full": {"type": "boolean"}}),
            &["item"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_schema_and_unique_name() {
        let t = tools();
        let mut names: Vec<&str> = t.iter().map(|x| x["name"].as_str().unwrap()).collect();
        assert!(t.iter().all(|x| x["inputSchema"]["type"] == "object"));
        names.sort_unstable();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
    }
}
