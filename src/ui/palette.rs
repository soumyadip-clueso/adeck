//! ⌘K: one search box over everything. Commands (with their shortcuts), open tabs, runs, the
//! board's groups and sounds, and shared pages. Type to narrow, ↑↓ to move, ↵ to run, Esc to close.

use rune::{Color, El, Icon, Key, TextEdit, Tx, col, el, row, spacer, text};
use rune_kit::{Kit, TextInput};

use crate::app::{App, TabKind};
use crate::ui::{icons, pick_files, pick_folder};

/// The palette while it's open.
pub struct State {
    pub query: TextEdit,
    pub active: usize,
}

impl State {
    pub fn new() -> State {
        let mut query = TextEdit::new("");
        query.focused = true;
        State { query, active: 0 }
    }
}

/// What a row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Cmd(&'static str),
    Tab(u64),
    Run(String),
    Group(String),
    Member(String, usize),
    Shared(String),
}

#[derive(Clone)]
pub struct Hit {
    pub section: &'static str,
    pub title: String,
    pub detail: String,
    pub keys: &'static str,
    pub icon: &'static Icon,
    pub act: Act,
}

fn cmd(title: &str, detail: &str, keys: &'static str, icon: &'static Icon, id: &'static str) -> Hit {
    Hit { section: "Commands", title: title.into(), detail: detail.into(), keys, icon, act: Act::Cmd(id) }
}

/// Everything the palette can reach right now, in the order it shows them unfiltered.
pub fn everything(s: &App) -> Vec<Hit> {
    let t = s.current();
    let kind = t.map(|t| t.spec.kind);
    let media = matches!(kind, Some(TabKind::Single | TabKind::Compare | TabKind::Render | TabKind::Board));
    let mut v = vec![
        cmd("Open file…", "Audio or video, in a new tab", "⌘O", &icons::OPEN_FILE, "open-file"),
        cmd("Open folder…", "Every file in it, as a table", "", &icons::FOLDER, "open-folder"),
        cmd("New compare", "Then add or drop files into it", "", &icons::COMPARE, "new-compare"),
        cmd("Shared pages", "Every page you've shared; take them down", "", &icons::SHARE, "shared"),
    ];
    if s.can_share() {
        v.push(cmd("Share this tab", "Publish it as a web page and copy the link", "⌘⇧E", &icons::SHARE, "share"));
    }
    if t.is_some() {
        v.push(cmd("Screenshot this tab", "Framed, to ~/Pictures/adeck and the clipboard", "⌘⇧S", &icons::CAMERA, "screenshot"));
    }
    if media {
        v.push(cmd("Play / pause", "", "Space", &icons::PLAY, "play"));
        v.push(cmd("Send to agent", "Hand the agent what you're on", "S", &icons::SPARK, "send"));
        v.push(cmd("Note for the agent", "", "N", &icons::SPARK, "note"));
        v.push(cmd(if t.is_some_and(|t| t.spec.blind) { "Blind off" } else { "Blind on" }, "Hide who's who until you pick", "B", &icons::COMPARE, "blind"));
        v.push(cmd(if t.is_some_and(|t| t.spec.loudness_match) { "Match loudness off" } else { "Match loudness on" }, "", "M", &icons::SPEAKER, "match"));
    }
    if matches!(kind, Some(TabKind::Single | TabKind::Compare | TabKind::Render)) {
        v.push(cmd("Zoom to fit", "Show the whole sound", "F", &icons::WAVE, "fit"));
    }
    if kind == Some(TabKind::Board) {
        v.push(cmd("Waveforms sheet", "Waveform, spectrogram and targets for what's playing", "Esc", &icons::WAVE, "sheet"));
    }
    v.push(cmd("Keyboard shortcuts", "Every key, this tab's first", "?", &icons::SPARK, "keys"));
    if t.is_some() {
        v.push(cmd("Close tab", "", "⌘W", &icons::CLOSE, "close"));
    }
    for tab in &s.tabs {
        if Some(tab.id()) == s.active {
            continue;
        }
        v.push(Hit {
            section: "Tabs",
            title: crate::ui::tab_title(s, tab),
            detail: format!("{:?}", tab.spec.kind).to_lowercase(),
            keys: "",
            icon: &icons::BOARD,
            act: Act::Tab(tab.id()),
        });
    }
    if let Some((t, b)) = t.and_then(|t| t.board().map(|b| (t, b))) {
        for g in b.sorted() {
            v.push(Hit {
                section: "Groups",
                title: g.title().to_owned(),
                detail: g.subtitle.clone().unwrap_or_default(),
                keys: "",
                icon: &icons::BOARD,
                act: Act::Group(g.key.clone()),
            });
        }
        for g in b.sorted() {
            for i in 0..g.playable().len() {
                let label = t.board_label(g, i);
                let name = s.entry(&g.playable()[i].item).filter(|e| !t.group_hidden(&g.key) && !e.item.is_ref).map(|e| e.item.name.clone()).unwrap_or_default();
                v.push(Hit { section: "Sounds", title: format!("{label} · {}", g.title()), detail: name, keys: "", icon: &icons::PLAY, act: Act::Member(g.key.clone(), i) });
            }
        }
    }
    for sh in s.shares.iter().filter(|x| !x.gone) {
        v.push(Hit { section: "Shared pages", title: sh.title.clone(), detail: sh.url.clone(), keys: "", icon: &icons::SHARE, act: Act::Shared(sh.url.clone()) });
    }
    for r in &s.menu_runs {
        v.push(Hit { section: "Runs", title: r.clone(), detail: "Open as a table".into(), keys: "", icon: &icons::TABLE, act: Act::Run(r.clone()) });
    }
    v
}

/// How well `q` matches `s`. A plain substring wins outright (more at a word start); otherwise
/// the letters must come in order, and every skipped letter costs, so scattered hits sink or drop.
pub fn score(q: &str, s: &str) -> Option<i32> {
    let q: String = q.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let low = s.to_lowercase();
    if let Some(i) = low.find(&q) {
        let start = i == 0 || !low[..i].chars().last().is_some_and(char::is_alphanumeric);
        return Some(100 + if start { 40 } else { 0 } - (i as i32).min(30) - low.len() as i32 / 10);
    }
    let s: Vec<char> = low.chars().collect();
    let mut total = 0;
    let mut at = 0usize;
    for qc in q.chars() {
        let i = (at..s.len()).find(|&i| s[i] == qc)?;
        let start = i == 0 || !s[i - 1].is_alphanumeric();
        let gap = (i - at) as i32;
        // Jumping to a word start is free (acronyms); skipping into the middle of a word isn't.
        total += if start { 8 } else { 2 - if at > 0 { gap.min(8) * 2 } else { 0 } };
        at = i + 1;
    }
    (total > q.len() as i32 * 2).then_some(total)
}

/// The rows for the query, best first (unfiltered: in their natural order, a few per section).
pub fn results(s: &App, q: &str) -> Vec<Hit> {
    let all = everything(s);
    if q.trim().is_empty() {
        let mut out = Vec::new();
        for sec in ["Commands", "Tabs", "Groups", "Shared pages", "Runs"] {
            out.extend(all.iter().filter(|h| h.section == sec).take(if sec == "Commands" { 12 } else { 5 }).cloned());
        }
        return out;
    }
    let mut scored: Vec<(i32, Hit)> = all
        .into_iter()
        .filter_map(|h| {
            let a = score(q, &h.title).map(|x| x + 8);
            let b = score(q, &format!("{} {}", h.title, h.detail));
            a.max(b).map(|x| (x, h))
        })
        .collect();
    // Best hits first, but each section stays together, sections ordered by their best hit.
    scored.sort_by_key(|x| std::cmp::Reverse(x.0));
    let mut order: Vec<&'static str> = Vec::new();
    for (_, h) in &scored {
        if !order.contains(&h.section) {
            order.push(h.section);
        }
    }
    scored.sort_by_key(|(_, h)| order.iter().position(|x| *x == h.section));
    scored.into_iter().take(40).map(|(_, h)| h).collect()
}

pub fn open(s: &mut App) {
    s.menu_runs = crate::source::list_runs(&s.cfg).into_iter().map(|r| r.name).take(40).collect();
    s.palette = Some(State::new());
}

fn run(s: &mut App, act: &Act, tx: &mut Tx) {
    s.palette = None;
    match act {
        Act::Cmd(id) => match *id {
            "open-file" => pick_files(tx, false),
            "open-folder" => pick_folder(tx),
            "new-compare" => s.new_compare(),
            "shared" => s.open_shared(),
            "share" => {
                if let Err(e) = s.share() {
                    s.toast = Some((e, true));
                }
            }
            "screenshot" => s.screenshot(),
            "play" => s.toggle_play(),
            "send" => s.send_to_agent(None, tx),
            "note" => s.toggle_note(),
            "blind" => s.toggle_blind(),
            "match" => s.toggle_loudness_match(),
            "fit" => s.zoom_fit(),
            "sheet" => s.toggle_sheet(),
            "keys" => crate::ui::keys::toggle(s),
            "close" => {
                if let Some(a) = s.active {
                    s.close(a);
                }
            }
            _ => {}
        },
        Act::Tab(id) => s.focus(*id),
        Act::Run(r) => {
            s.open(TabKind::Run, Vec::new(), Some(r.clone()), false, true, Some(r.clone()), None);
        }
        Act::Group(k) => s.focus_group(k),
        Act::Member(k, i) => {
            if s.current().and_then(|t| t.board()).is_some_and(crate::board::Board::is_pads) {
                s.pad_play(k, *i);
            } else {
                s.board_play(k, *i);
            }
        }
        Act::Shared(url) => {
            let _ = tx.open_url(url);
        }
    }
}

const ROW: f32 = 40.0;

pub fn build(s: &App, width: f32, k: &Kit, layer: &mut Vec<El>) {
    let Some(p) = &s.palette else { return };
    let c = &k.c;
    let q = p.query.text.clone();
    let hits = results(s, &q);
    let active = p.active.min(hits.len().saturating_sub(1));
    let keyed: Vec<Act> = hits.iter().map(|h| h.act.clone()).collect();
    let field = TextInput::new(&p.query)
        .plain()
        .large()
        .leading_icon(&icons::SEARCH)
        .width(None)
        .placeholder("Search commands, tabs, sounds, runs…")
        .label("Command palette")
        .active_descendant(Some(format!("pal-{active}")))
        .on_edit(|s: &mut App| &mut s.palette.as_mut().expect("palette open").query)
        .on_change(|s: &mut App, tx: &mut Tx| {
            if let Some(p) = s.palette.as_mut() {
                p.active = 0;
            }
            tx.scroll_into_view("pal-0");
        })
        .on_key(move |s: &mut App, ev: &rune::KeyEvent, tx: &mut Tx| {
            let n = keyed.len();
            match ev.key {
                Key::Escape => s.palette = None,
                Key::Down | Key::Up => {
                    if let Some(p) = s.palette.as_mut().filter(|_| n > 0) {
                        p.active = if ev.key == Key::Down { (p.active + 1) % n } else { (p.active + n - 1) % n };
                        tx.scroll_into_view(format!("pal-{}", p.active));
                    }
                }
                Key::Enter => {
                    let i = s.palette.as_ref().map_or(0, |p| p.active.min(n.saturating_sub(1)));
                    if let Some(act) = keyed.get(i) {
                        run(s, act, tx);
                    }
                }
                _ => return false,
            }
            true
        })
        .build(k)
        .key("pal-query");

    let mut list = col().key("pal-list").w_full().gap(1.0).px(6.0).py(6.0);
    let mut section = "";
    for (i, h) in hits.iter().enumerate() {
        if h.section != section {
            section = h.section;
            list = list.child(text(section).font(k.ty.small_medium).color(c.text_faint).px(10.0).pt(if i == 0 { 2.0 } else { 10.0 }).pb(4.0));
        }
        list = list.child(hit_row(h, i, i == active, k));
    }
    if hits.is_empty() {
        list = list.child(text(format!("Nothing matches “{q}”")).font(k.ty.ui).color(c.text_muted).px(12.0).py(18.0));
    }
    let foot = row()
        .w_full()
        .gap(14.0)
        .px(14.0)
        .py(8.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], c.border_weak)
        .child(hint("↑↓", "move", k))
        .child(hint("↵", "run", k))
        .child(hint("esc", "close", k))
        .child(spacer())
        .child(text(format!("{} result{}", hits.len(), if hits.len() == 1 { "" } else { "s" })).font(k.ty.small).color(c.text_faint));
    let panel_w = (width - 32.0).min(640.0);
    let panel = col()
        .key("palette")
        .w(panel_w)
        .radius(14.0)
        .fill(c.layer_01)
        .border(1.0, c.border_strong)
        .shadow(18.0, 48.0, Color::BLACK.alpha(0.45))
        .clip_rounded(14.0)
        .transition(rune::Transition::offset(0.0, -10.0).combined(rune::Transition::opacity()).combined(rune::Transition::scale(0.98)))
        .child(row().w_full().px(12.0).py(6.0).border_sides([0.0, 0.0, 1.0, 0.0], c.border_weak).child(field))
        .child(col().w_full().max_h(420.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
        .child(foot);
    let panel = panel.modal(true, |s: &mut App| s.palette = None);
    layer.push(col().key("pal-scrim").absolute(0.0, 0.0).w_full().h_full().fill(Color::BLACK.alpha(0.4)).transition(rune::Transition::opacity()).a11y_hidden());
    layer.push(col().key("pal-layer").absolute(0.0, 0.0).w_full().h_full().child(el().h(90.0)).child(row().w_full().child(spacer()).child(panel).child(spacer())));
}

fn hit_row(h: &Hit, i: usize, on: bool, k: &Kit) -> El {
    let c = &k.c;
    let act = h.act.clone();
    let mut r = row()
        .key(format!("pal-{i}"))
        .w_full()
        .h(ROW)
        .gap(10.0)
        .px(10.0)
        .center_y()
        .radius(8.0)
        .fill(if on { c.text_base.alpha(0.16) } else { Color::TRANSPARENT })
        .cursor(rune::Cursor::Pointer)
        .role("option")
        .label(h.title.clone())
        .child(rune::icon(h.icon, 16.0).color(if on { c.text_base } else { c.icon_muted }))
        .child(
            row()
                .gap(8.0)
                .min_w(0.0)
                .shrink()
                .center_y()
                .child(text(h.title.clone()).font(k.ty.ui).color(c.text_base).truncate())
                .child(text(h.detail.clone()).font(k.ty.small).color(c.text_faint).truncate()),
        )
        .child(spacer());
    if !h.keys.is_empty() {
        let mut keys = row().gap(3.0);
        for part in h.keys.split_inclusive(|ch: char| "⌘⇧⌥".contains(ch)) {
            keys = keys.child(keycap(part, k));
        }
        r = r.child(keys);
    }
    r.on_press_tx(move |s: &mut App, tx: &mut Tx| run(s, &act, tx))
}

pub(crate) fn keycap(t: &str, k: &Kit) -> El {
    let c = &k.c;
    row().px(5.0).h(20.0).min_w(20.0).center().radius(5.0).border(1.0, c.border_weak).fill(c.layer_02).child(text(t.to_owned()).font(k.ty.small_medium).color(c.text_muted))
}

pub(crate) fn hint(key: &str, what: &str, k: &Kit) -> El {
    row().gap(6.0).center_y().child(keycap(key, k)).child(text(what.to_owned()).font(k.ty.small).color(k.c.text_faint))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_in_order_match_and_word_starts_win() {
        assert!(score("shr", "Share this tab").is_some());
        assert!(score("xyz", "Share this tab").is_none());
        assert!(score("stt", "Share this tab").unwrap() > score("stt", "Screenshot this tab").unwrap_or(-99));
        assert!(score("open", "Open file…").unwrap() > score("open", "Shared pages: reopen").unwrap_or(-99));
        assert!(score("tone", "Match loudness off").is_none(), "scattered letters don't count");
        assert!(score("tone", "Pick 2 · Soft whoosh tone-a").unwrap() > 100);
    }
}
