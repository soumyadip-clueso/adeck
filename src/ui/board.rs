//! A board: one card per group, sorted (by search count, say), each a row of tiles: the picks,
//! the "before", the reference files. Click a tile to play it; only one plays, and switching
//! inside a group keeps the position. The focused group opens up: its waveform and
//! spectrogram zoomed to the sound, how its bands, width and tail sit against the target,
//! what made it, the keep/drop/move decision, and the ranked rest.

use rune::{Color, El, Tx, col, el, row, spacer, text};
use rune_kit::Kit;
use serde_json::{Map, Value};

use crate::app::{App, Entry, Tab};
use crate::board::{Chip, Group, Member};
use crate::ui::{px, widgets};

const TILE_MIN: f32 = 184.0;
const GAP: f32 = 10.0;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let Some(b) = t.board() else {
        return text("This board is empty.").font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT);
    };
    let blind = px::toggle("Blind", t.spec.blind).key("blind").help("Hide which is which in each group until you pick · B").on_press(|s: &mut App| s.toggle_blind());
    let lm = px::toggle("Match", t.spec.loudness_match).key("match").help("Match loudness: trim every take to the same LUFS · M").on_press(|s: &mut App| s.toggle_loudness_match());
    let zoom = px::toggle("Zoom", t.zoom_sound).key("zoom").help("Zoom to the sound: strips show just the sound, not the silence · Z").on_press(|s: &mut App| s.toggle_zoom());
    let mut tally = row().gap(14.0).center_y();
    for (tone, n) in b.tally() {
        tally = tally.child(px::lamp(&format!("{n} {}", tone_word(&tone)), tone_ink(&tone)));
    }
    let mut tools = row().gap(8.0).center_y().child(widgets::search_field(t, "Search", 200.0, k)).child(blind).child(lm).child(zoom);
    if !b.is_pads() {
        tools = tools.child(split_all(s, b));
    }
    let head = px::header("board-head", t.spec.label.clone().unwrap_or_else(|| "Board".into()), tally, tools);

    let mut page = col().key("board").w_full().h_full().min_h(0.0).gap(14.0).child(head);
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    if let Some(err) = &t.error {
        page = page.child(text(err.clone()).font(rune::TextStyle::new(13.0, 400.0)).color(px::BAD).wrap());
    }
    let focused = t.focused().map(|g| g.key.clone());
    let inner = width - 14.0;
    let mut list = col().key("groups").w_full().gap(12.0).pr(10.0);
    let shown = s.shown_groups(t);
    for g in &shown {
        let on = focused.as_deref() == Some(g.key.as_str());
        list = list.child(group_card(s, t, g, on, inner, k));
    }
    if shown.is_empty() && !b.groups.is_empty() {
        list = list.child(text(format!("No group or sound matches “{}”.", t.filter.text.trim())).font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).py(20.0));
    }
    if b.groups.is_empty() {
        list = list.child(text("Nothing on this board yet. The agent adds groups as results land.").font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).py(20.0));
    }
    page.child(col().key("groups-scroll").w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
}

fn tone_word(tone: &str) -> &str {
    match tone {
        "good" => "good",
        "close" => "close",
        "bad" => "not there yet",
        _ => "other",
    }
}

fn tone_ink(tone: &str) -> Color {
    match tone {
        "good" => px::GOOD,
        "close" => px::CLOSE,
        "bad" => px::BAD,
        _ => px::SILK,
    }
}

pub fn chip(ch: &Chip, _k: &Kit) -> El {
    px::tag(&ch.text, tone_ink(&ch.tone))
}

pub fn role_color(role: &str, _k: &Kit) -> Color {
    match role {
        "pick" => px::LED,
        "before" => px::CLOSE,
        "ref" => px::GOOD,
        _ => px::SILK,
    }
}

fn group_card(s: &App, t: &Tab, g: &Group, on: bool, width: f32, k: &Kit) -> El {
    let hidden = t.group_hidden(&g.key);
    let key = g.key.clone();
    // A fixed-height head whose right end always holds the same-sized slot, so focusing a group
    // (by playing, arrows or scrolling) never moves anything.
    let mut head = row()
        .w_full()
        .h(30.0)
        .gap(12.0)
        .center_y()
        .child(text(g.title().to_uppercase()).font(px::pixel(20.0)).color(if on { px::LED } else { px::SOFT }).truncate().shrink().min_w(80.0));
    if let Some(sub) = g.subtitle.as_ref().filter(|_| width >= 520.0) {
        head = head.child(text(sub.to_uppercase()).font(px::pixel(11.0)).color(px::FAINT).truncate().shrink());
    }
    head = head.child(spacer());
    // On a narrow card the verdict chip gets its own line so the title keeps its room.
    let tight = width < 620.0;
    if let Some(ch) = g.chip.as_ref().filter(|_| !hidden && !tight) {
        head = head.child(chip(ch, k));
    }
    let slot = if on {
        row().h(28.0).center().child(px::lamp("In pane", px::AMBER)).help("This group's target and detail are in the pane")
    } else {
        let k2 = key.clone();
        px::key("OPEN", false).key(format!("open-{key}")).help("Focus this group: its target and detail open in the pane").on_press(move |s: &mut App| {
            s.focus_group(&k2);
            if s.pane == px::Dock::Hidden {
                s.toggle_pane();
            }
        })
    };
    head = head.child(row().w(84.0).h(28.0).child(spacer()).child(slot));
    let mut card = col()
        .key(format!("group-{key}"))
        .w_full()
        .px(14.0)
        .py(14.0)
        .gap(12.0)
        .radius(6.0)
        .fill(if on { px::RAISED } else { Color::TRANSPARENT })
        .border(1.0, if on { px::GRILLE_ON } else { px::LINE })
        .child(head);
    if let Some(ch) = g.chip.as_ref().filter(|_| !hidden && tight) {
        card = card.child(row().child(chip(ch, k)));
    }

    // The tiles, wrapped to the width; the stem lanes open under the row of the one you hear.
    let order = t.group_order(g);
    let inner = width - 24.0;
    let cols = (((inner + GAP) / (TILE_MIN + GAP)).floor() as usize).clamp(1, order.len().max(1));
    let tile_w = (inner - GAP * (cols as f32 - 1.0)) / cols as f32;
    for (r, chunk) in order.chunks(cols).enumerate() {
        let mut line = row().key(format!("tiles-{key}-{r}")).w_full().gap(GAP).items_stretch();
        for (i, &m) in chunk.iter().enumerate() {
            line = line.child(tile(s, t, g, m, on, r * cols + i, tile_w, k));
        }
        card = card.child(line);
        if on && t.stems_open && chunk.contains(&t.active) {
            card = card.child(lanes(s, t, g, inner, k));
        }
    }
    // The same lines whether it's focused or not.
    if let Some(n) = &g.note {
        card = card.child(rune_kit::Markdown::new(n.clone()).build(k));
    }
    if !g.more.is_empty() {
        card = card.child(text(format!("+ {} MORE TAKES · RANKED IN THE PANE", g.more.len())).font(px::pixel(10.0)).color(px::FAINT));
    }
    card
}

/// One member: click anywhere on it to play it.
#[allow(clippy::too_many_arguments)]
fn tile(s: &App, t: &Tab, g: &Group, m: usize, focused: bool, pos: usize, w: f32, k: &Kit) -> El {
    let mem = &g.members[m];
    let hidden = t.group_hidden(&g.key);
    let e = s.entry(&mem.item);
    let active = focused && t.active == m;
    let playing = active && s.mixer_is_for(t.id()) && s.mixer.is_playing();
    let label = t.board_label(g, m);
    let mut top = row().gap(6.0).center_y();
    if pos < 9 {
        top = top.child(text((pos + 1).to_string()).font(px::pixel(11.0)).color(if focused { px::SILK } else { px::OFF }));
    }
    top = top.child(text(label.to_uppercase()).font(px::pixel(12.0)).color(if hidden { px::LED } else { role_color(&mem.role, k) }).truncate().shrink());
    if !hidden && e.is_some_and(|e| e.item.is_ref) {
        top = top.child(widgets::ref_badge(k));
    }
    top = top.child(spacer());
    if let Some(d) = s.decisions.get(&mem.item) {
        top = top.child(text(d.to_uppercase()).font(px::pixel(10.0)).color(if d == "drop" { px::BAD } else { px::SILK }));
    }
    // Stems: open the lanes (splitting it first if need be). Not on references.
    if e.is_some_and(|e| !e.item.is_ref) {
        let open = focused && t.stems_open && t.active == m;
        let have = s.stems.contains_key(&mem.item);
        let busy = !have && e.is_some_and(|e| s.splitting.contains(&e.item.path));
        let key = g.key.clone();
        top = top.child(
            px::key_sm(if busy { "Split…" } else { "Stems" }, open)
                .key(format!("stems-{}-{m}", g.key))
                .label(format!("Stems of {label}"))
                .help(if have {
                    "Its stems in lanes under this row: solo, mute, and A/B against the mix · T"
                } else if busy {
                    "Splitting into stems (about 15 seconds a minute of audio)"
                } else {
                    "Split into stems (drums, bass, other, vocals), then open them in lanes · T"
                })
                .on_press(move |s: &mut App| s.toggle_stems(&key, m)),
        );
    }
    // Hidden while blind: the file name would give it away.
    if !hidden && e.is_some() {
        let id = mem.item.clone();
        top = top.child(
            row()
                .key(format!("dl-{}-{m}", g.key))
                .size(24.0, 24.0)
                .center()
                .radius(4.0)
                .border(1.0, px::GRILLE)
                .hover(px::GRILLE)
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label(format!("Download {label}"))
                .help("Save a copy of this file")
                .child(rune::icon(&crate::ui::icons::DROP, 14.0).color(px::SOFT))
                .on_press_tx(move |s: &mut App, tx: &mut Tx| s.download(&id, tx)),
        );
    }
    let mut thumb = el().w_full().h(60.0).radius(3.0).fill(widgets::screen()).clip_rounded(3.0).instant_changes().inert();
    if let Some(e) = e {
        let range = if t.zoom_sound { e.sound } else { None };
        thumb = thumb.child(widgets::tile_picture(e, range, w - 20.0, 60.0));
        if active && let Some((a, b)) = view_of(t, e) {
            let pos = widgets::playhead(s, t);
            if pos >= a && pos <= b && b > a {
                thumb = thumb.child(el().absolute(((pos - a) / (b - a)) as f32 * (w - 20.0) - 1.0, 0.0).w(2.0).h_full().fill(px::AMBER));
            }
        }
    }
    let mut body = col().gap(6.0).child(top).child(thumb);
    if !hidden {
        let nums = numbers_line(&mem.numbers, s, t, g);
        if !nums.is_empty() {
            body = body.child(text(nums.to_uppercase()).font(px::pixel(10.0)).color(px::SILK).wrap());
        }
        if let Some(ch) = &mem.chip {
            body = body.child(row().child(chip(ch, k)));
        }
    }
    let set = s.stem_set(t, &mem.item);
    // What the track is made of: each stem's share of the loudness, in its colour.
    if let Some(st) = set.filter(|st| st.share.iter().any(Option::is_some)) {
        let w = 120.0;
        let mut bar = row().w(w).h(4.0).radius(1.0).fill(px::GRILLE).clip_rounded(1.0);
        for (n, sh) in st.names.iter().zip(&st.share) {
            if let Some(sh) = sh.filter(|v| *v > 0.0) {
                bar = bar.child(el().w(w * sh.clamp(0.0, 1.0) as f32).h_full().fill(px::stem_color(n)));
            }
        }
        let parts: Vec<String> = st.names.iter().zip(&st.share).filter_map(|(n, sh)| sh.map(|v| format!("{n} {:.0}%", v * 100.0))).collect();
        body = body.child(
            row()
                .gap(6.0)
                .center_y()
                .help(format!("Each stem's share of the track's loudness ({}): {}", st.label(), parts.join(", ")))
                .child(text("STEMS").font(px::pixel(10.0)).color(px::FAINT))
                .child(bar),
        );
    }
    // How much of the loudness is the bass: the low end that buries the instruments.
    if let Some(sh) = set.and_then(crate::stems::Stems::bass_share) {
        body = body.child(
            row()
                .gap(6.0)
                .center_y()
                .help("The bass stem's share of the track's loudness (BS.1770 energy of all its stems)")
                .child(text("LOW END").font(px::pixel(10.0)).color(px::FAINT))
                .child(text(format!("{:.0}%", sh * 100.0)).font(px::pixel(10.0)).color(if sh >= LOUD_BASS { px::CLOSE } else { px::SILK }))
                .child(meter(sh as f32, 48.0, if sh >= LOUD_BASS { px::CLOSE } else { px::SILK })),
        );
    }
    // Vocals are only a check: generated beds shouldn't have any, so a voice gets flagged.
    if let Some(v) = set.and_then(|st| st.share_of("vocals")).filter(|v| *v >= VOICE) {
        body = body.child(
            text(format!("VOCALS STEM {:.0}%", v * 100.0))
                .font(px::pixel(10.0))
                .color(px::BAD)
                .help("Something in the vocals stem: a voice, or a lead the splitter took for one. Solo vocals to hear it."),
        );
    }
    let secs = e.and_then(|e| e.analysis.as_ref()).map(|a| a.secs);
    if let Some(sec) = secs {
        body = body.child(spacer()).child(text(if sec < 1.0 { format!("{:.0} MS", sec * 1000.0) } else { format!("{sec:.1} S") }).font(px::pixel(10.0)).color(px::FAINT));
    }
    let key = g.key.clone();
    col()
        .key(format!("tile-{}-{m}", g.key))
        .w(w)
        .px(10.0)
        .py(10.0)
        .radius(4.0)
        .fill(if active { px::SELECTED } else { px::SCREEN })
        .border(
            1.0,
            if playing {
                px::AMBER
            } else if active {
                px::SOFT
            } else {
                px::LINE
            },
        )
        .hover(if active { px::SELECTED } else { px::RAISED })
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(format!("Play {label}"))
        .help(if focused && pos < 9 {
            format!("Play {label} · {} · double-click sends it to the agent", pos + 1)
        } else {
            format!("Play {label} · double-click sends it to the agent")
        })
        .motion(rune::Spring::SNAPPY)
        .child(body)
        .on_press(move |s: &mut App| s.board_play(&key, m))
        .on_double_press_tx(move |s: &mut App, tx: &mut Tx| s.send_to_agent(Some(m), tx))
}

/// A bass share at or above this is called out: the low end is most of what you hear.
const LOUD_BASS: f64 = 0.4;
/// A vocals share at or above this is flagged.
const VOICE: f64 = 0.05;
const LANE_H: f32 = 48.0;

/// A thin bar `f` full.
fn meter(f: f32, w: f32, ink: Color) -> El {
    el().w(w).h(4.0).radius(1.0).fill(px::GRILLE).child(el().w(w * f.clamp(0.0, 1.0)).h_full().radius(1.0).fill(ink))
}

/// The board header's stem button: split every track on the board that isn't split yet.
fn split_all(s: &App, b: &crate::board::Board) -> El {
    let ids: Vec<String> = b.items().into_iter().filter(|i| s.entry(i).is_some_and(|e| !e.item.is_ref)).collect();
    let have = ids.iter().filter(|i| s.stems.contains_key(*i)).count();
    let busy = ids.iter().filter(|i| s.entry(i).is_some_and(|e| s.splitting.contains(&e.item.path))).count();
    let label = if busy > 0 { format!("Splitting {busy}") } else { format!("Stems {have}/{}", ids.len()) };
    px::toggle(&label, have == ids.len() && !ids.is_empty())
        .key("split-all")
        .help(if have == ids.len() {
            "Every track here has stems: open them from a tile, or press T"
        } else {
            "Split every track on this board into stems, once each, in the background"
        })
        .on_press(move |s: &mut App| {
            if let Err(e) = s.split_stems(ids.clone()) {
                s.toast = Some((e, true));
            }
        })
}

/// The stem lanes of the track you're hearing: a header with the A/B and the low-end share,
/// then each stem (drums, bass, guitar, piano, vocals, other, as the split has them) with solo,
/// mute, its loudness and its waveform, all
/// on the board's one transport.
fn lanes(s: &App, t: &Tab, g: &Group, width: f32, k: &Kit) -> El {
    let Some(mem) = g.playable().get(t.active).copied() else { return col() };
    let wrap = col().key(format!("lanes-{}", g.key)).w_full().gap(6.0).px(10.0).py(10.0).radius(4.0).fill(px::SCREEN).border(1.0, px::GRILLE_ON);
    let Some(st) = s.stem_set(t, &mem.item) else {
        let busy = s.entry(&mem.item).is_some_and(|e| s.splitting.contains(&e.item.path));
        let msg = if busy { "Splitting into stems… about 15 seconds a minute of audio." } else { "No stems for this track yet." };
        let mut r =
            row().w_full().h(30.0).gap(10.0).center_y().child(px::label("Stems")).child(text(msg).font(rune::TextStyle::new(12.0, 400.0)).color(px::FAINT).truncate().shrink());
        if !busy {
            let id = mem.item.clone();
            r = r.child(spacer()).child(px::key_sm("Split", false).key("split-one").on_press(move |s: &mut App| {
                if let Err(e) = s.split_stems(vec![id.clone()]) {
                    s.toast = Some((e, true));
                }
            }));
        }
        return wrap.child(r);
    };
    let heard = t.stem_mode && (s.stems_loaded(t) || s.headless);
    let ab = row().gap(4.0).child(px::key_k("MIX", "A", !t.stem_mode).key("ab-mix").help("Hear the full mix · A").on_press(|s: &mut App| s.set_stem_mode(false))).child(
        px::key_k("STEMS", "A", t.stem_mode)
            .key("ab-stems")
            .help("Hear the stems summed, as soloed and muted below · A. Against the mix, what's left is the separation's artefacts.")
            .on_press(|s: &mut App| s.set_stem_mode(true)),
    );
    let mut head = row().w_full().h(30.0).gap(10.0).center_y().child(px::label("Stems")).child(ab);
    // The track's stem sets, by model, when it has more than one; else just which model.
    let sets = s.stems.get(&mem.item).map(Vec::as_slice).unwrap_or_default();
    if sets.len() > 1 {
        let mut pick = row().gap(4.0);
        for other in sets {
            let key = other.key.clone();
            pick = pick.child(
                px::key_sm(other.label(), other.key == st.key)
                    .key(format!("stem-set-{}", if key.is_empty() { "main" } else { key.as_str() }))
                    .help(format!("{} stems by {}: hear and see this split instead", other.names.len(), other.label()))
                    .on_press(move |s: &mut App| s.set_stem_set(&key)),
            );
        }
        head = head.child(pick);
    } else if st.model.is_some() {
        head = head.child(text(st.label().to_uppercase()).font(px::pixel(10.0)).color(px::OFF).truncate().shrink());
    }
    head = head.child(spacer());
    if let Some(sh) = st.bass_share() {
        head = head
            .child(text(format!("BASS IS {:.0}% OF THE LOUDNESS", sh * 100.0)).font(px::pixel(11.0)).color(if sh >= LOUD_BASS { px::CLOSE } else { px::SILK }).truncate().shrink());
    }
    if !s.stems_loaded(t) && s.mixer_is_for(t.id()) {
        head = head.child(text("LOADING").font(px::pixel(10.0)).color(px::FAINT));
    }
    head = head.child(px::key_sm("Close", false).key("stems-close").help("Close the lanes · T").on_press(|s: &mut App| s.toggle_stems_active()));
    let mut out = wrap.child(head);
    let entries: Vec<Option<&crate::app::Entry>> = st.files.iter().map(|p| s.entry(&crate::app::stem_id(p))).collect();
    // One scale for all of them, so the loud ones look loud next to the quiet ones.
    let peak = entries.iter().flatten().filter_map(|e| e.analysis.as_ref()).flat_map(|a| a.envelope.iter()).fold(0f32, |m, c| m.max(c[0].abs()).max(c[1].abs())).max(1e-4);
    let gains = crate::stems::gains(&st.names, &t.stem_solo, &t.stem_mute);
    let label_w = 168.0;
    for (i, name) in st.names.iter().enumerate() {
        let audible = gains[i] > 0.0;
        let title = row()
            .gap(5.0)
            .center_y()
            .child(el().w(6.0).h(6.0).radius(1.0).fill(px::stem_color(name)))
            .child(text(name.to_uppercase()).font(px::pixel(12.0)).color(if heard && audible { px::LED } else { px::SOFT }).truncate().shrink());
        let mut info = col().w(label_w - 64.0).gap(3.0).child(title);
        let mut sub = Vec::new();
        if let Some(l) = st.lufs[i] {
            sub.push(if l <= -69.0 { "SILENT".to_owned() } else { format!("{l:.1} LUFS") });
        }
        if let Some(sh) = st.share[i] {
            sub.push(format!("{:.0}%", sh * 100.0));
        }
        if !sub.is_empty() {
            info = info.child(text(sub.join(" · ")).font(px::pixel(9.0)).color(px::FAINT).truncate());
        }
        let (n1, n2) = (name.clone(), name.clone());
        let solo = px::key_sm("S", t.stem_solo.contains(name))
            .key(format!("solo-{name}"))
            .help(format!("Solo {name}: hear only the soloed stems"))
            .on_press(move |s: &mut App| s.stem_press(&n1, false));
        let mute = px::key_sm("M", t.stem_mute.contains(name)).key(format!("mute-{name}")).help(format!("Mute {name}")).on_press(move |s: &mut App| s.stem_press(&n2, true));
        let strip_w = (width - 20.0 - label_w - 10.0).max(40.0);
        let lane = widgets::lane_strip(s, t, entries[i], &format!("lane-{name}"), strip_w, LANE_H, peak, !(heard && audible), k);
        out = out.child(
            row()
                .key(format!("lane-row-{name}"))
                .w_full()
                .h(LANE_H)
                .gap(10.0)
                .center_y()
                .child(row().w(label_w).gap(6.0).center_y().child(info).child(spacer()).child(solo).child(mute))
                .child(col().grow(1.0).min_w(0.0).child(lane)),
        );
    }
    out
}

/// "vibe 1.05 · shape 0.51", labelled by the board's metrics where it has them.
pub fn numbers_line(n: &Map<String, Value>, _s: &App, t: &Tab, _g: &Group) -> String {
    let metrics = t.board().map(|b| b.metrics.as_slice()).unwrap_or_default();
    n.iter()
        .map(|(key, v)| {
            let label = metrics.iter().find(|m| m.key == *key).map_or(key.as_str(), |m| m.label());
            let val = match v {
                Value::Number(x) => x.as_f64().map_or_else(|| x.to_string(), fmt_num),
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            format!("{label} {val}")
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn fmt_num(x: f64) -> String {
    if (x.fract() == 0.0 && x.abs() < 1e6) || x.abs() >= 100.0 {
        format!("{x:.0}")
    } else if x.abs() >= 10.0 {
        format!("{x:.1}")
    } else {
        format!("{x:.2}")
    }
}

/// The stretch a strip shows for an entry: just the sound while zoomed.
fn view_of(t: &Tab, e: &Entry) -> Option<(f64, f64)> {
    let secs = e.analysis.as_ref()?.secs;
    Some(if t.zoom_sound { e.sound.unwrap_or((0.0, secs)) } else { (0.0, secs) })
}

/// The focused group opened up.
pub fn detail(s: &App, t: &Tab, g: &Group, width: f32, k: &Kit) -> El {
    let all = g.playable();
    let Some(mem) = all.get(t.active).copied() else { return col() };
    let hidden = t.active < g.members.len() && t.group_hidden(&g.key);
    let e = s.entry(&mem.item);
    let label = t.board_label(g, t.active);

    // What you're hearing, its length and trim, and the keys that act on it.
    let mut when = col().gap(4.0).items_end();
    if let Some(e) = e
        && e.analysis.is_some()
    {
        let (a, b) = s.window(t);
        let whole = e.analysis.as_ref().map_or(0.0, |x| x.secs);
        let what = if b - a < whole - 1e-3 { format!("{}–{}", widgets::clock(a), widgets::clock(b)) } else { widgets::clock(whole) };
        when = when.child(text(what).font(px::pixel(13.0)).color(px::SOFT));
    }
    if let Some(tr) = s.trim_db(t, &mem.item) {
        when = when.child(text(format!("MATCHED {tr:+.1} DB")).font(px::pixel(10.0)).color(px::FAINT));
    }
    let looping = t.loop_secs.is_some();
    let mut keys = vec![
        px::key_k("REPLAY", "R", false).key("replay").help("From the start of the sound · R").on_press(|s: &mut App| s.board_replay()),
        px::key_k("LOOP", "L", looping).key("loop-toggle").help("Loop what the strip shows · L. Drag across the strip to loop part of it.").on_press(|s: &mut App| s.toggle_loop()),
    ];
    if t.spec.blind && !t.spec.revealed_groups.contains(&g.key) {
        keys.push(
            px::key_k(format!("PICK {}", label.to_uppercase()), "↵", true)
                .key("board-pick")
                .help("The one you're hearing is the best here · Enter")
                .on_press(|s: &mut App| s.board_pick()),
        );
    }
    let hearing = col()
        .key("hearing")
        .w_full()
        .gap(12.0)
        .pb(14.0)
        .child(
            row()
                .w_full()
                .gap(12.0)
                .items_end()
                .child(col().gap(4.0).min_w(0.0).shrink().child(px::label("Hearing")).child(text(label.to_uppercase()).font(px::pixel(22.0)).color(px::LED).truncate()))
                .child(spacer())
                .child(when),
        )
        .child(px::segments(keys));
    let mut d = col().key("detail").w_full().child(hearing);
    if let Some(e) = e {
        d = d.child(
            col()
                .key("strips")
                .w_full()
                .gap(6.0)
                .pb(16.0)
                .child(widgets::ruler(s, t, "bruler", k))
                .child(widgets::wave_strip(s, t, Some(e), "bwave", width, 72.0, k))
                .child(widgets::spec_strip(s, t, Some(e), "bspec", width, 96.0, k)),
        );
    }
    if hidden {
        return d.child(px::section(
            "blind",
            "Blind",
            None,
            text("The numbers and what made it show after you pick.").font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).wrap(),
        ));
    }
    d = d.child(details(s, t, mem, e, width, k));
    if let Some(dec) = decisions(s, t, mem, k) {
        d = d.child(dec);
    }
    if !g.more.is_empty() {
        d = d.child(more(s, t, g, width, k));
    }
    d
}

/// What made the file: the prompt, model, seed and chain (every field when opened), its scores
/// as small readouts, and the file with a copy button.
fn details(s: &App, t: &Tab, mem: &Member, e: Option<&Entry>, width: f32, _k: &Kit) -> El {
    let open = t.details_open;
    let mut rows: Vec<(String, String)> = crate::item::flatten(&Value::Object(mem.meta.clone()));
    if rows.is_empty()
        && let Some(e) = e
    {
        rows = e.item.meta.clone();
    }
    let short = |k: &str| k.rsplit('.').next().unwrap_or(k).to_owned();
    let brief: Vec<(String, String)> =
        ["prompt", "model", "seed", "chain"].iter().filter_map(|f| rows.iter().find(|(k, _)| short(k) == *f).map(|(k, v)| (short(k), v.clone()))).collect();
    let more = rows.len().saturating_sub(brief.len());
    let shown: Vec<(String, String)> = if open { rows.iter().take(60).map(|(k, v)| (short(k), v.clone())).collect() } else { brief };
    let mut body = col().w_full();
    let n = shown.len();
    for (i, (k, v)) in shown.iter().enumerate() {
        body = body.child(px::kv(k, v, i + 1 == n));
    }
    // The agent's numbers and the loudness, as readouts three to a row.
    let mut nums: Vec<(String, String)> = mem
        .numbers
        .iter()
        .map(|(k, v)| (k.clone(), v.as_f64().map_or_else(|| v.to_string(), |x| format!("{x:.3}").trim_end_matches('0').trim_end_matches('.').to_owned())))
        .collect();
    if let Some(a) = e.and_then(|e| e.analysis.as_ref()) {
        if let Some(l) = a.lufs {
            nums.push(("lufs".into(), format!("{l:.1}")));
        }
        nums.push(("peak".into(), format!("{:.1}", a.peak_dbfs)));
    }
    if !nums.is_empty() {
        let cell_w = ((width - 16.0) / 3.0).floor();
        let mut grid = row().w_full().pt(12.0).gap(8.0).flex_wrap();
        for (k, v) in nums {
            grid = grid.child(
                col()
                    .w(cell_w)
                    .py(8.0)
                    .px(10.0)
                    .gap(4.0)
                    .radius(4.0)
                    .fill(px::SCREEN)
                    .border(1.0, px::LINE)
                    .child(text(k.to_uppercase()).font(px::pixel(9.0)).color(px::FAINT).truncate())
                    .child(text(v).font(px::pixel(15.0)).color(px::LED).truncate()),
            );
        }
        body = body.child(grid);
    }
    if let Some(e) = e {
        let path = e.item.path.display().to_string();
        let name = e.item.path.file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        let copy = path.clone();
        body = body.child(
            row()
                .key("file-row")
                .w_full()
                .mt(12.0)
                .h(34.0)
                .pl(10.0)
                .pr(4.0)
                .gap(8.0)
                .center_y()
                .radius(4.0)
                .border(1.0, px::LINE)
                .help(path)
                .child(text("FILE").font(px::pixel(9.0)).color(px::FAINT))
                .child(text(name).font(rune::TextStyle::new(12.0, 400.0).mono()).color(px::SOFT).truncate().grow(1.0).min_w(0.0))
                .child(px::key_sm("Copy path", false).key("copy-path").on_press_tx(move |s: &mut App, tx: &mut Tx| {
                    tx.copy(copy.clone());
                    s.toast = Some(("Path copied".into(), false));
                })),
        );
    }
    let toggle = (more > 0 || open).then(|| {
        px::link(if open { "Fewer ▴".to_owned() } else { format!("All {} fields ▸", rows.len()) }).key("details-toggle").on_press(|s: &mut App| {
            if let Some(t) = s.current_mut() {
                t.details_open ^= true;
            }
        })
    });
    let _ = s;
    px::section("details", "What made it", toggle, body)
}

/// Keep, drop, move: the board's choices for the file you're hearing, as one control.
fn decisions(s: &App, t: &Tab, mem: &Member, _k: &Kit) -> Option<El> {
    let b = t.board()?;
    if b.choices.is_empty() && b.move_to.is_empty() {
        return None;
    }
    let now = s.decisions.get(&mem.item);
    let mut items = Vec::new();
    for ch in &b.choices {
        let on = now == Some(ch);
        let (id, v) = (mem.item.clone(), ch.clone());
        let hint = match ch.as_str() {
            "keep" => "K",
            "drop" => "D",
            _ => "",
        };
        items.push(
            px::key_k(ch.to_uppercase(), hint, on)
                .key(format!("decide-{ch}"))
                .help(if hint.is_empty() { ch.clone() } else { format!("{ch} · {hint}") })
                .on_press(move |s: &mut App| s.decide(&id, &v)),
        );
    }
    if !b.move_to.is_empty() {
        let moved = now.and_then(|n| n.strip_prefix("move:"));
        items.push(
            px::key_k(moved.map_or_else(|| "MOVE TO".to_owned(), |m| format!("IN {}", m.to_uppercase())), if t.move_open { "▴" } else { "▾" }, moved.is_some())
                .key("move")
                .help("Move it to another group")
                .on_press(|s: &mut App| {
                    if let Some(t) = s.current_mut() {
                        t.move_open ^= true;
                    }
                }),
        );
    }
    let mut body = col().key("decide").w_full().gap(8.0).child(px::segments(items));
    if t.move_open {
        let mut targets = row().key("move-targets").w_full().gap(4.0).flex_wrap();
        for g in &b.move_to {
            let (id, v) = (mem.item.clone(), format!("move:{g}"));
            targets = targets.child(px::key_sm(g.clone(), now.is_some_and(|n| *n == v)).key(format!("move-{g}")).on_press(move |s: &mut App| s.decide(&id, &v)));
        }
        body = body.child(targets);
    }
    Some(px::section("decision", "Decision", None, body))
}

/// The ranked rest, folded. Click one to hear it.
fn more(s: &App, t: &Tab, g: &Group, _width: f32, _k: &Kit) -> El {
    let open = t.more_open.contains(&g.key);
    let key = g.key.clone();
    let toggle = px::link(if open { "Hide ▴" } else { "Show ▸" }).key("more-toggle").on_press(move |s: &mut App| s.toggle_more(&key));
    let mut list = col().key("more").w_full().gap(2.0);
    if open {
        for (i, m) in g.more.iter().enumerate() {
            let idx = g.members.len() + i;
            let on = t.active == idx;
            let key = g.key.clone();
            let nums = numbers_line(&m.numbers, s, t, g);
            let name = m.label.clone().or_else(|| s.entry(&m.item).map(|e| e.item.name.clone())).unwrap_or_default();
            let mut r = row()
                .key(format!("more-{i}"))
                .w_full()
                .h(40.0)
                .px(10.0)
                .gap(10.0)
                .center_y()
                .radius(4.0)
                .fill(if on { px::SELECTED } else { Color::TRANSPARENT })
                .border(1.0, if on { px::GRILLE_ON } else { Color::TRANSPARENT })
                .hover(px::SELECTED)
                .cursor(rune::Cursor::Pointer)
                .role("button")
                .label(format!("Play take {}", i + 1))
                .child(text(format!("{:02}", i + 1)).font(px::pixel(11.0)).color(if on { px::LED } else { px::OFF }).w(20.0))
                .child(
                    col()
                        .gap(3.0)
                        .min_w(0.0)
                        .grow(1.0)
                        .child(text(name).font(rune::TextStyle::new(12.0, 400.0).mono()).color(if on { px::LED } else { px::SOFT }).truncate())
                        .child(text(nums.to_uppercase()).font(px::pixel(9.0)).color(px::FAINT).truncate()),
                );
            if let Some(dec) = s.decisions.get(&m.item) {
                r = r.child(px::tag(dec, if dec == "drop" { px::BAD } else { px::SILK }));
            }
            list = list.child(r.on_press(move |s: &mut App| s.board_play(&key, idx)));
        }
    }
    px::section("others", &format!("Other takes · {}", g.more.len()), Some(toggle), list)
}
