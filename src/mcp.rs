//! The MCP server: streamable HTTP on 127.0.0.1, JSON responses (no SSE stream).
//!
//! Each request runs on its own thread. Tools that read or change what the window shows are
//! handed to the UI thread as a `Call` and answered from `App::poll`, so a tool and the window
//! never disagree. Three run here instead, because they wait: `wait_for_feedback` blocks on the
//! feedback signal, `screenshot_tab` renders the tab in a headless copy of adeck, and `analyze`
//! runs earshot.

use std::io::Read as _;
use std::path::PathBuf;
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

pub fn serve(cfg: Config, feedback_path: PathBuf, signal: Signal, waker: rune::Waker) -> Result<Receiver<Call>, String> {
    let addr = format!("127.0.0.1:{}", cfg.port);
    let server = tiny_http::Server::http(&addr).map_err(|e| e.to_string())?;
    let (tx, rx) = channel::<Call>();
    std::thread::Builder::new()
        .name("adeck-mcp".into())
        .spawn(move || {
            for req in server.incoming_requests() {
                let ctx = Ctx { cfg: cfg.clone(), tx: tx.clone(), waker: waker.clone(), feedback_path: feedback_path.clone(), signal: signal.clone() };
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
        Err((code, msg)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}}),
    })
}

fn text_result(v: Result<Value, String>) -> Value {
    match v {
        Ok(v) => json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default()}], "structuredContent": if v.is_object() { v.clone() } else { json!({"result": v}) }}),
        Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
    }
}

fn call_tool(name: &str, args: &Value, ctx: &Ctx) -> Value {
    match name {
        "wait_for_feedback" => text_result(wait_for_feedback(args, ctx)),
        "screenshot_tab" => match screenshot(args, ctx) {
            Ok(png) => json!({"content": [{"type": "image", "data": base64::engine::general_purpose::STANDARD.encode(png), "mimeType": "image/png"}]}),
            Err(e) => text_result(Err(e)),
        },
        "analyze" => text_result(analyze(args, ctx)),
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
    let out = std::env::temp_dir().join(format!("adeck-shot-{}-{tab}.png", std::process::id()));
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let theme = args.get("theme").and_then(Value::as_str).unwrap_or("dark");
    let status = std::process::Command::new(exe)
        .args(["--snap", &out.display().to_string(), "--size", &size, "--scale", "1", "--theme", theme])
        .env("ADECK_FOCUS", tab.to_string())
        .env("ADECK_HEADLESS", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("Couldn't start the headless render: {e}"))?;
    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        return Err(format!("The headless render failed: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output")));
    }
    let png = std::fs::read(&out).map_err(|e| format!("No screenshot was written: {e}"))?;
    let _ = std::fs::remove_file(&out);
    Ok(png)
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

pub fn tools() -> Vec<Value> {
    let items = json!({"type": "array", "items": {"type": "string"}, "description": "Item ids: absolute paths, run:<run>/<path>, ref:<path>, or s3:// URLs."});
    let tab = json!({"type": "integer", "description": "A tab id from an open_* call or list_tabs."});
    let note = json!({"type": "string", "description": "Markdown pinned in the tab: what to listen for."});
    vec![
        tool("list_runs", "Runs adeck knows: folders of generated audio under the configured run roots, the local-only reference pack (`refs`), and S3 runs when include_s3 is true.", json!({"include_s3": {"type": "boolean"}}), &[]),
        tool("list_items", "A run's items: id, name, a one-line summary (model, mood or type) and the user's latest score.", json!({"run": {"type": "string"}, "filter": {"type": "string", "description": "Case-insensitive match on name or any metadata value."}}), &["run"]),
        tool("get_item", "Everything about one item: flattened metadata (prompt, model, seed, flags…), native analysis (duration, LUFS, peak, BPM, key), cached earshot summaries and the latest score.", json!({"id": {"type": "string"}}), &["id"]),
        tool("open_single", "Open one file in a tab: waveform with beat grid, spectrogram, all metadata and earshot numbers. The user can score it 0-3.", json!({"item": {"type": "string"}, "label": {"type": "string"}, "note": note}), &["item"]),
        tool("open_compare", "Open 2-9 files side by side. They play in sync; keys 1-9 switch instantly at the same moment. Loudness-matched by default. Blind mode shuffles them as X/Y/Z and reveals after the user picks.", json!({"items": items, "blind": {"type": "boolean"}, "loudness_match": {"type": "boolean"}, "label": {"type": "string"}, "note": note}), &["items"]),
        tool("open_render", "Open one or more videos (renders): picture, earshot critique scorecard and sheet. Several renders switch like a compare.", json!({"items": items, "label": {"type": "string"}, "note": note}), &["items"]),
        tool("open_run", "Open a whole run as a sortable, filterable table. The user opens rows or compares a selection.", json!({"run": {"type": "string"}, "filter": {"type": "string"}, "label": {"type": "string"}}), &["run"]),
        tool("list_tabs", "Open tabs and which is in front.", json!({}), &[]),
        tool("focus_tab", "Bring a tab to the front.", json!({"tab_id": tab}), &["tab_id"]),
        tool("close_tab", "Close a tab.", json!({"tab_id": tab}), &["tab_id"]),
        tool("annotate_tab", "Pin markdown in a tab (what to listen for, what changed). Empty clears it.", json!({"tab_id": tab, "markdown": {"type": "string"}}), &["tab_id", "markdown"]),
        tool("play", "Play a tab (the front one by default), optionally a member and a start time.", json!({"tab_id": tab, "member": {"type": "integer"}, "at_s": {"type": "number"}}), &[]),
        tool("stop", "Pause playback.", json!({}), &[]),
        tool("set_loop", "Loop a region of a tab. Omit start_s/end_s to clear.", json!({"tab_id": tab, "start_s": {"type": "number"}, "end_s": {"type": "number"}}), &[]),
        tool("screenshot_tab", "A PNG of a tab as the user sees it.", json!({"tab_id": tab, "size": {"type": "string", "description": "WxH in points, default 1280x800."}, "theme": {"type": "string", "enum": ["dark", "light"]}}), &[]),
        tool("get_feedback", "The user's verdicts: scores, picks (with the blind reveal), ranks and notes. Filter by tab, run or items; since is unix seconds.", json!({"tab_id": tab, "run": {"type": "string"}, "items": items, "since": {"type": "number"}}), &[]),
        tool("wait_for_feedback", "Block until the user scores, picks or notes something (in this tab, if given), then return the new rows. Use it to loop: open, wait, read, generate, open again.", json!({"tab_id": tab, "timeout_s": {"type": "number", "description": "Default 300."}}), &[]),
        tool("analyze", "Run earshot on an item (cached): describe (tags, labels), measure (loudness, peaks, spectrum), critique (scorecard vs pro films, for renders) or timeline. Returns a summary unless full is true.", json!({"item": {"type": "string"}, "kind": {"type": "string", "enum": ["describe", "measure", "critique", "timeline"]}, "full": {"type": "boolean"}}), &["item"]),
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
