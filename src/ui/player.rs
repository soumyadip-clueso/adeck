//! The player: the one transport at the bottom of the window, like the strip under a sampler's
//! screen. Play, the clock, an LED overview to scrub, zoom, volume, then what this page decides
//! with (score keys, the take you hear and Pick), your note and send-to-agent.

use rune::{Drag, El, Phase, Tx, el, row, text};
use rune_kit::Kit;

use crate::app::{App, Tab, TabKind};
use crate::ui::px;
use crate::ui::widgets::{clock, playhead, tab_secs};

const SEGS: usize = 96;

/// Whether a tab plays audio, and so gets the player.
pub fn has_player(t: &Tab) -> bool {
    matches!(t.spec.kind, TabKind::Single | TabKind::Compare | TabKind::Render | TabKind::Board)
}

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    // Under ~1100 points the bar drops its labels and the volume, so the scrubber keeps room.
    let compact = width < 1100.0;
    let playing = s.mixer_is_for(t.id()) && s.mixer.is_playing();
    let play = row()
        .key("play")
        .size(40.0, 40.0)
        .center()
        .radius(6.0)
        .fill(px::LED)
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(if playing { "Pause" } else { "Play" })
        .help(if playing { "Pause · Space" } else { "Play · Space" })
        .child(rune::icon(if playing { &crate::ui::icons::PAUSE } else { &crate::ui::icons::PLAY }, 16.0).color(px::CASE))
        .on_press(|s: &mut App| s.toggle_play());
    let total = tab_secs(s, t);
    let ph = playhead(s, t);
    // The clock, centred, with a line under it for whatever's transient (the loop, loading,
    // an output error). It floats, so it never moves the clock or squeezes the scrubber.
    let mut time = row()
        .key("clock")
        .w(if compact { 132.0 } else { 168.0 })
        .h(68.0)
        .gap(6.0)
        .center_y()
        .child(text(clock(ph)).font(px::pixel(24.0)).color(px::LED))
        .child(text(format!("/ {}", clock(total))).font(px::pixel(12.0)).color(px::OFF).pt(6.0));
    let status = if let Some(e) = s.mixer.error.as_ref().filter(|_| s.mixer_is_for(t.id())) {
        Some(text(e.to_uppercase()).font(px::pixel(10.0)).color(px::BAD).truncate().max_w(132.0).help(e.clone()))
    } else if s.mixer_is_for(t.id()) && s.mixer.is_loading() {
        Some(row().gap(6.0).center_y().child(rune_kit::Spinner::new().size(9.0).build(k)).child(px::label("Loading")))
    } else {
        t.loop_secs.map(|(a, b)| {
            row()
                .key("loop")
                .gap(6.0)
                .center_y()
                .cursor(rune::Cursor::Pointer)
                .help("Clear the loop · Esc")
                .child(el().size(6.0, 6.0).fill(px::AMBER))
                .child(text(format!("LOOP {}–{}  ×", clock(a), clock(b))).font(px::pixel(10.0)).color(px::AMBER))
                .on_press(|s: &mut App| s.set_loop(None))
        })
    };
    if let Some(st) = status {
        time = time.child(st.absolute(0.0, 49.0));
    }

    let mut r = row()
        .key("player")
        .w_full()
        .h(68.0)
        .px(if compact { 14.0 } else { 20.0 })
        .gap(if compact { 12.0 } else { 18.0 })
        .center_y()
        .fill(px::RAISED)
        .border_sides([1.0, 0.0, 0.0, 0.0], px::LINE)
        .child(play)
        .child(time)
        .child(overview(t, total, ph, width, compact));
    let fit = if t.view.is_some() { "FIT" } else { "ALL" };
    r = r
        .child(
            row()
                .key("zoom")
                .gap(2.0)
                .child(px::key("−", false).key("zoom-out").help("Zoom out · −").on_press(|s: &mut App| s.zoom(2.0)))
                .child(px::key(fit, false).key("zoom-fit").help("Whole file, or the loop · F").on_press(|s: &mut App| s.zoom_fit()))
                .child(px::key("+", false).key("zoom-in").help("Zoom in · =").on_press(|s: &mut App| s.zoom(0.5))),
        )
        .child(el().w(1.0).h(28.0).fill(px::GRILLE));
    if !compact {
        r = r.child(volume(s)).child(el().w(1.0).h(28.0).fill(px::GRILLE));
    }
    r = match t.spec.kind {
        TabKind::Single => r.child(scores(s, t, compact)).child(el().w(1.0).h(28.0).fill(px::GRILLE)),
        TabKind::Compare | TabKind::Render if t.spec.items.len() > 1 => r.child(takes(t, compact)).child(el().w(1.0).h(28.0).fill(px::GRILLE)),
        _ => r,
    };
    r = r.child(px::key("NOTE", false).key("note-toggle").help("Write a note for the agent · N").on_press(|s: &mut App| s.show_note()));
    let send = if matches!(t.spec.kind, TabKind::Compare | TabKind::Render) && t.spec.items.len() > 1 {
        px::key(format!("PICK {}", t.member_label(t.active)), true).w(84.0).key("pick").help("The one you're hearing is better · Enter").on_press(|s: &mut App| s.pick())
    } else {
        px::key(if compact { "SEND" } else { "SEND TO AGENT" }, false)
            .key("send")
            .help("Hand the agent what you're on: the file, the group, the loop or the zoomed stretch · S")
            .on_press_tx(|s: &mut App, tx: &mut Tx| s.send_to_agent(None, tx))
    };
    r.child(send)
}

/// The whole file as a row of LED segments: lit behind the playhead, amber at it, the zoomed
/// window outlined. Drag across it to scrub.
fn overview(t: &Tab, total: f64, ph: f64, width: f32, compact: bool) -> El {
    // As many segments as fit at about six points each, so they never shrink to nothing.
    let room = width - if compact { 600.0 } else { 860.0 };
    let segs_n = ((room / 6.0) as usize).clamp(16, SEGS);
    let f = if total > 0.0 { (ph / total).clamp(0.0, 1.0) } else { 0.0 };
    let at = ((f * segs_n as f64) as usize).min(segs_n - 1);
    let (va, vb) =
        t.view.filter(|_| total > 0.0).map_or((0, segs_n), |(a, b)| (((a / total) * segs_n as f64) as usize, (((b / total) * segs_n as f64).ceil() as usize).min(segs_n)));
    let mut segs = row().grow(1.0).min_w(0.0).h(24.0).gap(2.0).center_y();
    for i in 0..segs_n {
        let ink = if i == at && total > 0.0 {
            px::AMBER
        } else if i < at {
            px::SOFT
        } else {
            px::GRILLE_ON
        };
        let h = if (va..vb).contains(&i) && t.view.is_some() { 16.0 } else { 10.0 };
        segs = segs.child(el().grow(1.0).w(0.0).h(h).fill(ink));
    }
    row().key("scrubber").instant_changes().grow(1.0).min_w(60.0).h(32.0).center_y().cursor(rune::Cursor::Pointer).help("Drag to scrub").child(segs).on_drag(
        move |s: &mut App, d: &mut Drag, _tx: &mut Tx| {
            if d.w > 0.0 {
                s.scrub(f64::from((d.x / d.w).clamp(0.0, 1.0)) * total, d.phase);
            }
        },
    )
}

/// Volume as seven rising bars, lit up to the level. Drag across to set it.
fn volume(s: &App) -> El {
    let vol = s.mixer.volume;
    let lit = ((vol / 1.5) * 7.0).round() as usize;
    let mut bars = row().gap(2.0).h(16.0).items_end();
    for i in 0..7 {
        bars = bars.child(el().w(3.0).h(4.0 + 2.0 * i as f32).fill(if i < lit { px::SOFT } else { px::GRILLE_ON }));
    }
    row()
        .key("volume")
        .w(40.0)
        .h(28.0)
        .center_y()
        .cursor(rune::Cursor::Pointer)
        .help(format!("Volume {}", if vol <= 0.001 { "muted".into() } else { format!("{:+.1} dB", 20.0 * vol.log10()) }))
        .child(bars)
        .on_drag(|s: &mut App, d: &mut Drag, _tx: &mut Tx| {
            if d.w > 0.0 && !matches!(d.phase, Phase::Up) {
                s.set_volume((d.x / d.w).clamp(0.0, 1.0) * 1.5);
            }
        })
}

fn scores(s: &App, t: &Tab, compact: bool) -> El {
    let current = t.spec.items.get(t.active).and_then(|i| s.scores.get(i)).copied();
    let help = ["0 · unusable", "1 · usable under a voiceover", "2 · as good as what ships today", "3 · library level"];
    let mut r = row().key("scores").gap(4.0).center_y();
    if !compact {
        r = r.child(px::label("SCORE").pr(4.0));
    }
    for v in 0..4i64 {
        r = r.child(px::key(v.to_string(), current == Some(v)).key(format!("score-{v}")).help(format!("{} · press {v}", help[v as usize])).on_press(move |s: &mut App| s.score(v)));
    }
    r
}

fn takes(t: &Tab, compact: bool) -> El {
    let mut r = row().key("takes").gap(4.0).center_y();
    if !compact {
        r = r.child(px::label("HEAR").pr(4.0));
    }
    for (pos, &m) in t.shown().iter().enumerate().take(9) {
        r = r.child(
            px::key(t.member_label(m), m == t.active)
                .key(format!("hear-{m}"))
                .help(format!("Listen to {} · {}", t.member_label(m), pos + 1))
                .on_press(move |s: &mut App| s.set_active_member(m)),
        );
    }
    r
}
