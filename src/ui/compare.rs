//! A/B/N: every take's waveform stacked on one time axis with one playhead, the one you hear
//! lit. Keys 1-9 switch at the same moment, Enter picks. The loudness match trims each take to
//! the same LUFS. Blind mode hides names and metadata behind X/Y/Z until you pick. What differs
//! between them is in the pane; play, the take switcher and Pick are in the player.

use rune::{El, col, row, spacer, text};
use rune_kit::Kit;

use crate::app::{App, Tab};
use crate::ui::{px, widgets};

/// The lane's letter column.
const LETTER_W: f32 = 40.0;
const GAP: f32 = 4.0;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let hidden = t.is_blind_hidden();
    let narrow = width < 620.0;
    let strip_w = width - LETTER_W - GAP;

    let title = t.spec.label.clone().unwrap_or_else(|| if hidden { format!("Blind ×{}", t.spec.items.len()) } else { "Compare".into() });
    let sub = if hidden {
        format!("{} takes · names show after you pick", t.spec.items.len())
    } else {
        let prompt = t.spec.items.first().and_then(|i| s.entry(i)).and_then(|e| e.item.pick(&["prompt", "caption", "text"]).map(str::to_owned));
        format!("{} takes{}", t.spec.items.len(), prompt.map(|p| format!(" · {p}")).unwrap_or_default())
    };
    let mut toggles = row().key("compare-head-tools").gap(8.0).flex_wrap();
    if t.spec.items.len() > 1 {
        let mut blind = px::toggle("Blind", t.spec.blind).key("blind").help("Hide names until you pick · B");
        if !t.spec.revealed {
            blind = blind.on_press(|s: &mut App| s.toggle_blind());
        }
        toggles = toggles
            .child(blind)
            .child(px::toggle("Match loudness", t.spec.loudness_match).key("match").help("Trim every take to the same LUFS · M").on_press(|s: &mut App| s.toggle_loudness_match()));
    }
    toggles = toggles.child(
        row()
            .key("add-file")
            .h(30.0)
            .px(10.0)
            .gap(8.0)
            .center_y()
            .radius(4.0)
            .border(1.0, px::GRILLE)
            .cursor(rune::Cursor::Pointer)
            .help("Add a file to compare (or drop files on the window)")
            .child(text("+ ADD").font(px::pixel(11.0)).color(px::SILK))
            .on_press_tx(|_s: &mut App, tx: &mut rune::Tx| crate::ui::pick_files(tx, true)),
    );
    let head = row()
        .key("compare-head")
        .w_full()
        .gap(16.0)
        .items_end()
        .flex_wrap()
        .child(col().gap(6.0).min_w(0.0).shrink().child(px::title(title)).child(text(sub).font(rune::TextStyle::new(13.0, 400.0)).color(px::SILK).truncate()))
        .child(spacer())
        .child(toggles);

    let mut page = widgets::page("compare", width).gap(16.0).child(head);
    if t.spec.items.is_empty() {
        return page.child(crate::ui::drop_zone("Drop audio files here to compare them", k));
    }
    if let Some(a) = widgets::annotation(t, k) {
        page = page.child(a);
    }
    if let Some(err) = &t.error {
        page = page.child(text(err.clone()).font(rune::TextStyle::new(13.0, 400.0)).color(px::BAD).wrap());
    }
    if let Some(p) = t.picked {
        let msg = if t.spec.blind { format!("You picked {}: {}", t.member_label(p), member_name(s, t, p)) } else { format!("Picked {}", t.member_label(p)) };
        page = page.child(px::lamp(&msg, px::GOOD).key("picked"));
    }

    // Lanes share what's left with the spectrogram of the one you hear.
    let shown = t.shown();
    let (_, win_h, _) = s.win.get();
    let room = (win_h - 40.0 - 68.0 - 40.0 - 120.0 - if t.spec.note.is_some() { 56.0 } else { 0.0 } - if t.picked.is_some() { 28.0 } else { 0.0 }).max(260.0);
    let spec_h = (room * 0.26).clamp(72.0, 200.0);
    let lane_h = ((room - spec_h - 24.0) / shown.len().max(1) as f32 - 6.0).clamp(48.0, 180.0);

    let ruler = widgets::ruler(s, t, "ruler", k);
    let mut lanes = col().key("members").w_full().gap(6.0).child(if narrow {
        ruler
    } else {
        row().w_full().gap(GAP).child(rune::el().w(LETTER_W)).child(col().grow(1.0).min_w(0.0).child(ruler))
    });
    for (pos, &m) in shown.iter().enumerate() {
        lanes = lanes.child(lane(s, t, m, pos, if narrow { width } else { strip_w }, lane_h, narrow, k));
    }
    // The spectrogram of the one you hear.
    let active_id = t.spec.items.get(t.active).cloned().unwrap_or_default();
    let spec = widgets::spec_strip(s, t, s.entry(&active_id), "spec", if narrow { width } else { strip_w }, spec_h, k);
    let letter = col()
        .w(LETTER_W)
        .h(spec_h)
        .py(6.0)
        .pl(2.0)
        .child(text("SPEC").font(px::pixel(10.0)).color(px::FAINT))
        .child(spacer())
        .child(text(t.member_label(t.active)).font(px::pixel(13.0)).color(px::LED));
    lanes = lanes.child(if narrow { spec } else { row().w_full().gap(GAP).pt(6.0).child(letter).child(col().grow(1.0).min_w(0.0).child(spec)) });
    page.child(lanes)
}

/// One take: its letter and key on the left, its waveform with the name, loudness and trim on
/// the screen itself. Click to hear it; double-click sends it to the agent.
#[allow(clippy::too_many_arguments)]
fn lane(s: &App, t: &Tab, m: usize, pos: usize, strip_w: f32, lane_h: f32, narrow: bool, k: &Kit) -> El {
    let hidden = t.is_blind_hidden();
    let id = &t.spec.items[m];
    let e = s.entry(id);
    let on = m == t.active;
    let label = t.member_label(m);

    let mut chip = row().absolute(10.0, 8.0).gap(10.0).center_y().px(6.0).py(2.0).fill(px::SCREEN);
    if !hidden {
        chip = chip.child(text(member_name(s, t, m)).font(rune::TextStyle::new(12.0, 400.0).mono()).color(if on { px::LED } else { px::SILK }).max_w(strip_w * 0.5).truncate());
        if e.is_some_and(|e| e.item.is_ref) {
            chip = chip.child(px::tag("REF", px::SILK));
        }
    }
    let mut nums = Vec::new();
    if let Some(l) = s.lufs(id) {
        nums.push(format!("{l:.1} LUFS"));
    }
    if let Some(d) = s.trim_db(t, id) {
        nums.push(format!("{d:+.1} DB"));
    }
    if let Some(score) = s.scores.get(id).filter(|_| !hidden) {
        nums.push(format!("SCORE {score}"));
    }
    chip = chip.child(text(nums.join(" · ")).font(px::pixel(11.0)).color(if on { px::SILK } else { px::FAINT }));
    chip = chip.child(
        text("×")
            .key(format!("remove-{m}"))
            .font(px::pixel(12.0))
            .color(px::OFF)
            .cursor(rune::Cursor::Pointer)
            .help(format!("Remove {label}"))
            .on_press(move |s: &mut App| s.remove_member(m)),
    );
    let mut wave = widgets::wave_strip(s, t, e, &format!("wave-{m}"), strip_w, lane_h, k).child(chip);
    if on {
        wave = wave.border(1.0, px::GRILLE_ON);
    } else {
        wave = wave.opacity(0.6);
    }
    let wave = wave.opacity_motion(rune::Spring::SNAPPY);
    let letter = col()
        .key(format!("member-{m}"))
        .w(LETTER_W)
        .h(lane_h)
        .py(8.0)
        .pl(2.0)
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(format!("Listen to {label}"))
        .help(format!("Listen to {label} · press {} · double-click sends it to the agent", pos + 1))
        .child(text(label.clone()).font(px::pixel(28.0)).color(if on { px::LED } else { px::OFF }))
        .child(spacer())
        .child(text(format!("{}", pos + 1)).font(px::pixel(11.0)).color(if on { px::FAINT } else { px::OFF }))
        .on_press(move |s: &mut App| s.set_active_member(m))
        .on_double_press_tx(move |s: &mut App, tx: &mut rune::Tx| s.send_to_agent(Some(m), tx));
    if narrow {
        col().w_full().gap(4.0).child(row().gap(8.0).center_y().child(text(label).font(px::pixel(20.0)).color(if on { px::LED } else { px::OFF }))).child(wave)
    } else {
        row().w_full().gap(GAP).child(letter).child(col().grow(1.0).min_w(0.0).child(wave))
    }
}

fn member_name(s: &App, t: &Tab, m: usize) -> String {
    let id = &t.spec.items[m];
    s.entry(id).map_or_else(|| id.rsplit('/').next().unwrap_or(id).to_owned(), |e| e.item.label())
}
