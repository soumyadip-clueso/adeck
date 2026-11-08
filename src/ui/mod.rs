//! The window: tabs along the titlebar, the front tab's content, and the footer.

pub mod board;
pub mod compare;
pub mod diff;
pub mod pads;
pub mod keys;
pub mod palette;
pub mod shared;
pub mod sheet;
pub mod icons;
pub mod render;
pub mod run;
pub mod single;
pub mod statusbar;
pub mod widgets;

use rune::{Cx, El, KeyEvent, Tx, col, el, row, text};
use rune_kit::shell::{TabItem, TabOrientation, TabStrip, WindowTitlebar};
use rune_kit::{Kit, Toast, ToastRegion};

use crate::app::{App, TabKind};

pub fn kit(dark: bool) -> &'static Kit {
    rune_theme_opencode::kit(dark)
}

pub const PAD: f32 = 16.0;

pub fn view(s: &App, cx: &mut Cx<App>) -> El {
    let k = kit(cx.system_dark());
    let c = &k.c;
    let mut layer = Vec::new();
    let titlebar = titlebar(s, cx, k, &mut layer);
    let width = cx.layout_size("body").map_or(1240.0, |z| z[0]) - 2.0 * PAD;
    let body = match s.current() {
        None => empty(s, k),
        Some(t) => match t.spec.kind {
            TabKind::Single => single::build(s, t, width, k),
            TabKind::Compare => compare::build(s, t, width, k),
            TabKind::Render => render::build(s, t, width, k),
            TabKind::Run => run::build(s, t, k),
            TabKind::Board => {
                let main_w = sheet::main_width(s, t, width);
                let main = if t.board().is_some_and(crate::board::Board::is_pads) { pads::build(s, t, main_w, k) } else { board::build(s, t, main_w, k) };
                sheet::wrap(s, t, main, width, k)
            }
            TabKind::Diff => diff::build(s, t, width, k),
            TabKind::Shared => shared::build(s, t, width, k),
        },
    };
    // Narrow windows scroll the whole page rather than squeezing the strips.
    let body = match s.current().map(|t| t.spec.kind) {
        Some(TabKind::Single | TabKind::Compare | TabKind::Render) if width < 620.0 => col().key("narrow-scroll").w_full().h_full().min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(body),
        _ => body,
    };
    let footer_w = cx.layout_size("statusbar").map_or(1280.0, |z| z[0]);
    if let Some([w, h]) = cx.layout_size("root") {
        s.win.set((w, h, cx.system_dark()));
    }
    let mut root = col().key("root").w_full().h_full().fill(c.page).child(titlebar).child(col().key("body").w_full().grow(1.0).min_h(0.0).px(PAD).pt(10.0).pb(10.0).child(body));
    if let Some(a) = statusbar::activity(s, k) {
        root = root.child(a);
    }
    root = root.child(statusbar::build(s, footer_w, k));
    if let Some((msg, err)) = &s.toast {
        let t = Toast::new("toast", msg.clone()).icon(k, if *err { &icons::WARNING } else { &icons::CHECK }).closable().on_close(|s: &mut App| s.toast = None).build(k);
        layer.push(el().absolute(0.0, 0.0).w_full().h_full().inert().child(ToastRegion::new().toast(t).build(k)));
    }
    keys::build(s, footer_w, k, &mut layer);
    palette::build(s, footer_w, k, &mut layer);
    root.children(layer).on_drop_files(|s: &mut App, files, _| s.open_files(files.to_vec())).accepts_files(|p: &std::path::Path| crate::item::media_of(p).is_some() || p.is_dir())
}

fn titlebar(s: &App, cx: &mut Cx<App>, k: &Kit, layer: &mut Vec<El>) -> El {
    let items: Vec<TabItem<'_, App>> = s
        .tabs
        .iter()
        .map(|t| {
            let icon = match t.spec.kind {
                TabKind::Single => &icons::WAVE,
                TabKind::Compare => &icons::COMPARE,
                TabKind::Render => &icons::FILM,
                TabKind::Run => &icons::TABLE,
                TabKind::Board => &icons::BOARD,
                TabKind::Diff => &icons::DIFF,
                TabKind::Shared => &icons::SHARE,
            };
            TabItem::new(t.id().to_string(), tab_title(s, t)).icon(icon)
        })
        .collect();
    let active = s.active.map(|a| a.to_string());
    let strip = TabStrip::new("tabs", TabOrientation::Horizontal)
        .tabs(items)
        .active(active.as_deref())
        .viewport_width(cx.layout_size("tabs").map(|z| z[0]))
        .drag_state(&s.tab_drag, |s: &mut App| &mut s.tab_drag)
        .close_icon(&icons::CLOSE)
        .on_select(|s: &mut App, id: &str, _tx: &mut Tx| {
            if let Ok(id) = id.parse() {
                s.focus(id);
            }
        })
        .on_close(|s: &mut App, id: &str, _tx: &mut Tx| {
            if let Ok(id) = id.parse() {
                s.close(id);
            }
        })
        .on_reorder(|s: &mut App, r, _tx: &mut Tx| s.reorder(&r.id, r.to))
        .trailing_action(new_menu(s, k, layer))
        .build_layered(k, layer);
    let mcp = row()
        .key("mcp-status")
        .gap(6.0)
        .center_y()
        .px(8.0)
        .child(el().size(7.0, 7.0).radius(4.0).fill(if s.calls.is_some() { k.c.success.icon } else { k.c.icon_faint }))
        .child(text(s.mcp_status.clone()).font(k.ty.small).color(k.c.text_muted))
        .help("Connect with: claude mcp add --transport http adeck http://127.0.0.1:<port>/mcp");
    let trailing = row().gap(10.0).center_y().child(mcp);
    WindowTitlebar::from_insets(cx.chrome_insets(), 1.0).tabs(strip).trailing(trailing).build(k)
}

pub fn tab_title(s: &App, t: &crate::app::Tab) -> String {
    if let Some(l) = &t.spec.label {
        return l.clone();
    }
    match t.spec.kind {
        TabKind::Run => t.spec.run.clone().unwrap_or_else(|| "Run".into()),
        TabKind::Board => "Board".into(),
        TabKind::Diff => t.spec.diff.as_ref().map_or_else(|| "Diff".into(), |d| format!("{} vs {}", d.a, d.b)),
        _ if t.is_blind_hidden() => format!("Blind ×{}", t.spec.items.len()),
        TabKind::Compare | TabKind::Render if t.spec.items.len() > 1 => {
            format!("{} ×{}", short(s, &t.spec.items[0]), t.spec.items.len())
        }
        _ => t.spec.items.first().map_or_else(|| "Empty".into(), |i| short(s, i)),
    }
}

fn short(s: &App, id: &str) -> String {
    s.entry(id).map_or_else(|| id.rsplit('/').next().unwrap_or(id).to_owned(), |e| e.item.name.clone())
}

/// `/`: into the tab's search box, on tabs that have one.
pub fn focus_search(s: &mut App) -> bool {
    let Some(t) = s.current_mut().filter(|t| matches!(t.spec.kind, TabKind::Board | TabKind::Run | TabKind::Shared)) else { return false };
    t.filter.focused = true;
    true
}

/// The "+" after the tabs: open a file or folder, a new compare, or a run.
fn new_menu(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let plus = rune_kit::IconButton::new(&icons::PLUS)
        .variant(rune_kit::IconButtonVariant::Ghost)
        .label("New tab")
        .shortcut("⌘T")
        .on_press(|s: &mut App| {
            if s.new_menu {
                s.new_menu = false;
            } else {
                s.open_new_menu();
            }
        })
        .build(k)
        .key("new-tab");
    let mut m = rune_kit::Menu::new("new-tab-menu")
        .trigger(plus)
        .open(s.new_menu)
        .min_width(240.0)
        .on_close(|s: &mut App| s.new_menu = false)
        .item(rune_kit::MenuItem::new("Open file…").icon(&icons::OPEN_FILE).shortcut("⌘O").on_select_tx(|s: &mut App, tx: &mut Tx| {
            s.new_menu = false;
            pick_files(tx, false);
        }))
        .item(rune_kit::MenuItem::new("Open folder…").icon(&icons::FOLDER).description("Every file in it, as a table").on_select_tx(|s: &mut App, tx: &mut Tx| {
            s.new_menu = false;
            pick_folder(tx);
        }))
        .item(rune_kit::MenuItem::new("New compare").icon(&icons::COMPARE).description("Then add or drop files into it").on_select(|s: &mut App| {
            s.new_menu = false;
            s.new_compare();
        }))
        .item(rune_kit::MenuItem::new("Shared pages").icon(&icons::SHARE).description("Every page you've shared; take them down").on_select(|s: &mut App| {
            s.new_menu = false;
            s.open_shared();
        }));
    if !s.menu_runs.is_empty() {
        m = m.separator().group_label("Runs");
        for r in &s.menu_runs {
            let run = r.clone();
            m = m.item(rune_kit::MenuItem::new(r.clone()).icon(&icons::TABLE).on_select(move |s: &mut App| {
                s.new_menu = false;
                s.open(TabKind::Run, Vec::new(), Some(run.clone()), false, true, Some(run.clone()), None);
            }));
        }
    }
    m.build(k, layer)
}

/// Asks for a file. `add` puts it beside what's open (a compare); otherwise it opens in a tab.
pub fn pick_files(tx: &mut Tx, add: bool) {
    let mut exts: Vec<&str> = crate::item::AUDIO_EXT.to_vec();
    exts.extend_from_slice(crate::item::VIDEO_EXT);
    let opts = rune::FileDialogOptions { title: if add { "Add a file to compare".into() } else { "Open a file".into() }, ..Default::default() }.filter("Audio and video", &exts);
    tx.open_file(opts, move |s: &mut App, r, _tx: &mut Tx| match r {
        Ok(Some(p)) if add => s.add_members(vec![p.display().to_string()]),
        Ok(Some(p)) => s.open_files(vec![p]),
        Ok(None) => {}
        Err(e) => s.toast = Some((format!("Couldn't open the file dialog: {e:?}"), true)),
    });
}

pub fn pick_folder(tx: &mut Tx) {
    tx.open_folder(rune::FileDialogOptions { title: "Open a folder of audio".into(), ..Default::default() }, |s: &mut App, r, _tx: &mut Tx| {
        if let Ok(Some(p)) = r {
            s.open_folder(p);
        }
    });
}

/// A dashed target for dropped files, with the same choices as the "+" menu.
pub fn drop_zone(msg: &str, k: &Kit) -> El {
    let c = &k.c;
    col()
        .key("drop-zone")
        .w_full()
        .grow(1.0)
        .min_h(160.0)
        .center()
        .gap(10.0)
        .radius(k.radius.lg)
        .border(1.5, c.border_strong)
        .fill(c.layer_01)
        .transition(rune::Transition::opacity())
        .child(rune::icon(&icons::DROP, 26.0).color(c.icon_faint))
        .child(text(msg.to_owned()).font(k.ty.ui_medium).color(c.text_base))
        .child(
            row()
                .gap(8.0)
                .child(rune_kit::Button::new("Open file…").variant(rune_kit::ButtonVariant::Outline).on_press_tx(|s: &mut App, tx: &mut Tx| pick_files(tx, s.current().is_some_and(|t| t.spec.kind == TabKind::Compare))).build(k).key("dz-file"))
                .child(rune_kit::Button::new("Open folder…").variant(rune_kit::ButtonVariant::Outline).on_press_tx(|_s: &mut App, tx: &mut Tx| pick_folder(tx)).build(k).key("dz-folder")),
        )
}

fn empty(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let hint = format!("claude mcp add --transport http adeck http://127.0.0.1:{}/mcp", s.cfg.port);
    col()
        .key("empty")
        .w_full()
        .grow(1.0)
        .center()
        .gap(8.0)
        .child(rune::icon(&icons::DROP, 26.0).color(c.icon_faint))
        .child(text("Nothing open").font(k.ty.ui_medium).color(c.text_base))
        .child(text("Ask your agent to open files, a compare or a run. Or drop audio and video here.").font(k.ty.small).color(c.text_muted))
        .child(
            row()
                .gap(8.0)
                .py(4.0)
                .child(rune_kit::Button::new("Open file…").variant(rune_kit::ButtonVariant::Contrast).on_press_tx(|_s: &mut App, tx: &mut Tx| pick_files(tx, false)).build(k).key("empty-file"))
                .child(rune_kit::Button::new("Open folder…").variant(rune_kit::ButtonVariant::Outline).on_press_tx(|_s: &mut App, tx: &mut Tx| pick_folder(tx)).build(k).key("empty-folder"))
                .child(rune_kit::Button::new("New compare").variant(rune_kit::ButtonVariant::Outline).on_press(|s: &mut App| s.new_compare()).build(k).key("empty-compare")),
        )
        .child(text(hint).font(rune::TextStyle { size: 11.0, ..k.ty.mono }).color(c.text_faint).selectable(true).py(6.0))
}

pub fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let plain = !ev.mods.cmd && !ev.mods.alt && !ev.mods.ctrl;
    if ev.is_command('k') || (ev.is_command('p') && ev.mods.shift) {
        if s.palette.is_some() {
            s.palette = None;
        } else {
            palette::open(s);
        }
        return true;
    }
    if s.palette.is_some() {
        return false;
    }
    if s.keys_help {
        if ev.key == rune::Key::Escape || ev.key == rune::Key::Text("?".into()) {
            s.keys_help = false;
        }
        return true;
    }
    if !ev.mods.cmd && !ev.mods.ctrl && ev.key == rune::Key::Text("?".into()) {
        keys::toggle(s);
        return true;
    }
    if ev.is_command('t') {
        s.open_new_menu();
        return true;
    }
    if ev.is_command('e') && ev.mods.shift {
        if let Err(e) = s.share() {
            s.toast = Some((e, true));
        }
        return true;
    }
    if ev.is_command('s') && ev.mods.shift {
        s.screenshot();
        return true;
    }
    if ev.is_command('o') {
        pick_files(tx, false);
        return true;
    }
    if ev.is_command('w') {
        if let Some(a) = s.active {
            s.close(a);
        }
        return true;
    }
    let Some(kind) = s.current().map(|t| t.spec.kind) else { return false };
    if plain && ev.key == rune::Key::Text("/".into()) && focus_search(s) {
        return true;
    }
    if matches!(kind, TabKind::Run | TabKind::Diff | TabKind::Shared) {
        return false;
    }
    if kind == TabKind::Board && s.current().and_then(|t| t.board()).is_some_and(crate::board::Board::is_pads) && !ev.mods.cmd && !ev.mods.alt && !ev.mods.ctrl {
        match &ev.key {
            rune::Key::Text(ch) if ch == " " => {
                s.pad_replay();
                return true;
            }
            rune::Key::Text(ch) if ch.chars().count() == 1 && pads::on_key(s, ch.chars().next().unwrap_or(' ')) => return true,
            _ => {}
        }
    }
    if kind == TabKind::Board && board_key(s, ev) {
        return true;
    }
    let Some(t) = s.current() else { return false };
    if ev.key == rune::Key::Text(" ".into()) && plain {
        s.toggle_play();
        return true;
    }
    if !plain {
        return false;
    }
    let step = if ev.mods.shift { 5.0 } else { 1.0 };
    match &ev.key {
        rune::Key::Left => {
            let at = s.mixer.secs() - step;
            s.seek_secs(at);
            return true;
        }
        rune::Key::Right => {
            let at = s.mixer.secs() + step;
            s.seek_secs(at);
            return true;
        }
        rune::Key::Escape => {
            s.set_loop(None);
            return true;
        }
        rune::Key::Text(ch) if matches!(ch.as_str(), "=" | "+" | "-" | "f" | "n" | "s" | "[" | "]") => {
            match ch.as_str() {
                "=" | "+" => s.zoom(0.5),
                "-" => s.zoom(2.0),
                "f" => s.zoom_fit(),
                "n" => s.toggle_note(),
                "s" => s.send_to_agent(None, tx),
                "[" => s.pan(-0.5),
                _ => s.pan(0.5),
            }
            return true;
        }
        rune::Key::Enter if kind == TabKind::Compare => {
            s.pick();
            return true;
        }
        rune::Key::Text(ch) if ch.len() == 1 => {
            let d = ch.chars().next().unwrap_or(' ');
            match (kind, d) {
                (TabKind::Single, '0'..='3') => {
                    s.score(i64::from(d as u8 - b'0'));
                    return true;
                }
                (TabKind::Compare | TabKind::Render, '1'..='9') => {
                    let pos = (d as u8 - b'1') as usize;
                    let shown = t.shown();
                    if let Some(&member) = shown.get(pos) {
                        s.set_active_member(member);
                    }
                    return true;
                }
                _ => {}
            }
        }
        _ => {}
    }
    false
}

/// A board's keys: ↑/↓ group, ←/→ member (⇧ seeks), 1-9 member, R replay, L loop, Z zoom,
/// K keep, D drop, B blind, M match, Enter pick.
fn board_key(s: &mut App, ev: &KeyEvent) -> bool {
    if ev.mods.cmd || ev.mods.alt || ev.mods.ctrl {
        return false;
    }
    match &ev.key {
        rune::Key::Up => s.board_step_group(-1),
        rune::Key::Down => s.board_step_group(1),
        rune::Key::Left | rune::Key::Right if ev.mods.shift => {
            let by = if ev.key == rune::Key::Left { -5.0 } else { 5.0 };
            let at = s.mixer.secs() + by;
            s.seek_secs(at);
        }
        rune::Key::Left => s.board_step_member(-1),
        rune::Key::Right => s.board_step_member(1),
        rune::Key::Enter => s.board_pick(),
        rune::Key::Escape => {
            if s.current().is_some_and(|t| t.loop_secs.is_some()) {
                s.set_loop(None);
            } else {
                s.toggle_sheet();
            }
        }
        rune::Key::Text(ch) if ch.chars().count() == 1 => {
            let d = ch.chars().next().unwrap_or(' ').to_ascii_lowercase();
            match d {
                '1'..='9' => {
                    let pos = (d as u8 - b'1') as usize;
                    let Some(t) = s.current() else { return false };
                    let Some(g) = t.focused() else { return false };
                    let Some(&m) = t.group_order(g).get(pos) else { return true };
                    let key = g.key.clone();
                    s.board_play(&key, m);
                }
                'r' => s.board_replay(),
                'l' => s.board_toggle_loop(),
                'z' => s.toggle_zoom(),
                'k' => s.decide_active("keep"),
                'd' => s.decide_active("drop"),
                'b' => s.toggle_blind(),
                'm' => s.toggle_loudness_match(),
                _ => return false,
            }
        }
        _ => return false,
    }
    true
}
