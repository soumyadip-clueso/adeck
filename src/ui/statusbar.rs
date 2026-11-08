//! The panel's footer: what's in view on the left, and one chip per job on the right. Chips
//! that don't fit fold into "+N more", and the Activity panel above the footer lists every job.
//!
//! Each chip and each Activity row is built from one `Job`, and everything in it is keyed by
//! that job's id. Its stop button captures that id and nothing else, so it can only ever stop
//! the job it sits in.

use rune::{El, Icon, Transition, col, el, row, spacer, text};
use rune_kit::{ControlSize, IconButton, IconButtonVariant, Kit, Spinner};

use crate::app::App;
use crate::jobs::{Job, Level, Outcome, Phase};
use crate::ui::{icons, px};
/// The footer's side padding.
pub const INSET: f32 = 16.0;

pub const HEIGHT: f32 = 32.0;
/// A compact chip's width, so the footer knows how many fit.
const CHIP_W: f32 = 236.0;
/// Room the summary on the left keeps before chips take over.
const SUMMARY_W: f32 = 230.0;
/// The "+N more" button's width.
const MORE_W: f32 = 76.0;

pub fn build(s: &App, width: f32, k: &Kit) -> El {
    let jobs = s.jobs.list();
    let free = width - 2.0 * INSET - SUMMARY_W;
    let fits = |n: usize| n as f32 * (CHIP_W + 6.0) <= free;
    let shown = if fits(jobs.len()) { jobs.len() } else { (0..jobs.len()).rev().find(|&n| n as f32 * (CHIP_W + 6.0) + MORE_W <= free).unwrap_or(0).max(1) };
    let hidden = jobs.len() - shown;
    let mut chips = row().key("status-jobs").gap(6.0).center_y();
    if hidden > 0 {
        chips = chips.child(px::key_sm(format!("+{hidden} more"), s.activity_open).key("status-more").on_press(|s: &mut App| s.activity_open ^= true));
    }
    // The newest jobs stay in the footer; older ones fold away first.
    for job in &jobs[hidden..] {
        chips = chips.child(chip(job, k));
    }
    let running = jobs.iter().filter(|j| j.is_active()).count();
    let lead = if running > 0 { px::lamp(&format!("{running} working"), px::AMBER) } else { px::lamp("Done", px::SILK) };
    row()
        .key("statusbar")
        .w_full()
        .h(HEIGHT)
        .px(INSET)
        .gap(12.0)
        .center_y()
        .fill(px::CASE)
        .border_sides([1.0, 0.0, 0.0, 0.0], px::LINE)
        .child(lead.key("status-summary").cursor(rune::Cursor::Pointer).help("Every job · click for Activity").on_press(|s: &mut App| s.activity_open ^= true))
        .child(if s.jobs.any_finished() {
            px::key_sm("Clear all", false).key("status-clear").help("Clear every finished job").on_press(|s: &mut App| s.jobs.clear_finished())
        } else {
            el().size(0.0, 22.0)
        })
        .child(spacer())
        .child(chips)
}

/// How a job looks right now: its leading mark, its line of text and that text's colour.
fn face(job: &Job, k: &Kit) -> (El, String, rune::Color) {
    match &job.phase {
        Phase::Running => (Spinner::new().size(12.0).color(px::AMBER).build(k), job.title.clone(), px::SOFT),
        Phase::Stopping => (Spinner::new().size(12.0).color(px::FAINT).build(k), format!("Stopping {}…", job.kind.noun()), px::SILK),
        Phase::Done { outcome: Outcome::Completed(m), .. } => (glyph(&icons::CHECK, px::GOOD), m.clone(), px::SOFT),
        Phase::Done { outcome: Outcome::Stopped(m), .. } => (glyph(&icons::STOP, px::FAINT), m.clone(), px::SILK),
        Phase::Done { outcome: Outcome::Failed(m), .. } => (glyph(&icons::WARNING, px::BAD), m.clone(), px::BAD),
    }
}

/// The job's one button: stop while it runs, nothing while it stops, dismiss once it's done.
fn action(job: &Job, k: &Kit) -> El {
    let id = job.id;
    match job.phase {
        Phase::Running => IconButton::new(&icons::STOP)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label(format!("Stop {}", job.kind.noun()))
            .help(format!("Stop {}", job.kind.noun()))
            .on_press(move |s: &mut App| s.jobs.cancel(id))
            .build(k),
        // Nothing to press while it winds down. Keep the space so nothing shifts.
        Phase::Stopping => el().size(20.0, 20.0),
        Phase::Done { .. } => IconButton::new(&icons::CLOSE)
            .variant(IconButtonVariant::GhostMuted)
            .control_size(ControlSize::Small)
            .label("Dismiss")
            .on_press(move |s: &mut App| s.jobs.dismiss(id))
            .build(k),
    }
}

fn chip(job: &Job, k: &Kit) -> El {
    let id = job.id;
    let (lead, title, ink) = face(job, k);
    let mut body = row()
        .grow(1.0)
        .min_w(0.0)
        .gap(8.0)
        .center_y()
        .child(lead.key(id.key("lead")))
        .child(text(title).key(id.key("title")).font(rune::TextStyle::new(12.0, 500.0)).color(ink).truncate().shrink());
    if job.is_active() {
        body = body.child(bar(job, 40.0, k));
        if let Some(f) = job.fraction() {
            body = body.child(text(format!("{:.0}%", f * 100.0)).key(id.key("pct")).font(px::pixel(10.0)).color(px::SILK).w(30.0).align(rune::Align::End));
        }
    }
    let fill = match &job.phase {
        Phase::Done { outcome: Outcome::Failed(_), .. } => px::BAD.alpha(0.12),
        Phase::Running | Phase::Stopping | Phase::Done { outcome: Outcome::Completed(_) | Outcome::Stopped(_), .. } => px::SCREEN,
    };
    let help = if job.detail.is_empty() { chip_label(job) } else { format!("{} · {}", chip_label(job), job.detail) };
    let chip = row()
        .key(id.key("chip"))
        .role("status")
        .label(chip_label(job))
        .w(CHIP_W)
        .h(24.0)
        .pl(8.0)
        .pr(2.0)
        .gap(4.0)
        .center_y()
        .radius(4.0)
        .fill(fill)
        .border(1.0, px::LINE)
        .transition(Transition::offset(0.0, 6.0))
        .child(body.help(help).on_press(move |s: &mut App| s.open_job(id)).cursor(rune::Cursor::Pointer))
        .child(action(job, k).key(id.key("action")));
    k.interactive(chip)
}

/// Every job, one per row with room for the item in hand, above the footer. Each row can open
/// its log.
pub fn activity(s: &App, k: &Kit) -> Option<El> {
    if !s.activity_open || s.jobs.list().is_empty() {
        return None;
    }
    let rows = s.jobs.list().iter().rev().map(|job| {
        let id = job.id;
        let open = s.log_open == Some(id);
        let (lead, title, ink) = face(job, k);
        let detail = match (&job.phase, job.detail.is_empty()) {
            (Phase::Running | Phase::Stopping, false) => {
                format!("{} · {}", job.detail, elapsed(job))
            }
            (Phase::Running | Phase::Stopping, true) => elapsed(job),
            (Phase::Done { .. }, _) => job.title.clone(),
        };
        let mut r = row().key(id.key("activity")).w_full().h(40.0).px(INSET).gap(10.0).center_y().child(lead).child(
            col()
                .min_w(0.0)
                .shrink()
                .gap(1.0)
                .child(text(title).font(rune::TextStyle::new(13.0, 500.0)).color(ink).truncate())
                .child(text(detail.to_uppercase()).key(id.key("detail")).font(px::pixel(10.0)).color(px::FAINT).truncate()),
        );
        r = r.child(spacer());
        if job.is_active() {
            r = r.child(bar(job, 140.0, k));
            r = r.child(text(job.fraction().map_or_else(String::new, |f| format!("{:.0}%", f * 100.0))).font(px::pixel(10.0)).color(px::SILK).w(34.0).align(rune::Align::End));
        }
        let warnings = job.log.iter().filter(|l| l.level != Level::Info).count();
        let label = if warnings > 0 { format!("Log · {warnings}") } else { "Log".to_owned() };
        r = r.child(px::key_sm(label, open).key(id.key("log-toggle")).on_press(move |s: &mut App| s.log_open = if s.log_open == Some(id) { None } else { Some(id) }));
        let r = r.child(action(job, k).key(id.key("activity-action")));
        let mut block = col().key(id.key("activity-block")).w_full().child(r);
        if open {
            block = block.child(log(job, k));
        }
        block
    });
    let header = row()
        .w_full()
        .h(32.0)
        .px(INSET)
        .center_y()
        .gap(6.0)
        .child(text("ACTIVITY").font(px::pixel(12.0)).color(px::LED))
        .child(spacer())
        .child({
            let clear = px::key_sm("Clear finished", false).key("activity-clear");
            if s.jobs.any_finished() { clear.on_press(|s: &mut App| s.jobs.clear_finished()) } else { clear.opacity(0.4) }
        })
        .child(px::key_sm("Open log file", false).key("activity-logfile").help("Opens every job's log in Console").on_press(|_: &mut App| {
            if let Some(p) = crate::jobs::log_path() {
                let _ = std::process::Command::new("open").arg("-a").arg("Console").arg(p).spawn();
            }
        }))
        .child(
            IconButton::new(&icons::CLOSE)
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label("Close activity")
                .on_press(|s: &mut App| s.activity_open = false)
                .build(k)
                .key("activity-close"),
        );
    Some(
        col()
            .key("activity")
            .w_full()
            // A sheet pulled up over the list: a firm top edge, a raised surface, and a shadow
            // cast up onto the rows so it never reads as more of them.
            .border_sides([1.0, 0.0, 0.0, 0.0], px::GRILLE)
            .fill(px::RAISED)
            .shadow(-6.0, 18.0, rune::Color::BLACK.alpha(if k.dark { 0.45 } else { 0.12 }))
            .transition(Transition::offset(0.0, 12.0))
            .child(header.border_sides([0.0, 0.0, 1.0, 0.0], px::LINE))
            .child(col().key("activity-rows").w_full().max_h(360.0).scroll(rune::ScrollSpec { stick_bottom: false }).children(rows)),
    )
}

/// How long a running job has been going: "0:42".
fn elapsed(job: &Job) -> String {
    let secs = job.started.elapsed().as_secs() as u32;
    format!("running {}", {
        let s = secs;
        format!("{}:{:02}", s / 60, s % 60)
    })
}

/// One job's log: every line with its time and level, newest at the bottom, and a copy button.
fn log(job: &Job, k: &Kit) -> El {
    let id = job.id;
    let mono = rune::TextStyle { size: 11.0, ..k.ty.mono };
    let lines = job.log.iter().map(|l| {
        let (tag, ink) = match l.level {
            Level::Info => ("     ", px::SILK),
            Level::Warn => ("warn ", px::CLOSE),
            Level::Error => ("error", px::BAD),
        };
        row()
            .w_full()
            .gap(8.0)
            .child(text(l.clock.clone()).font(mono).color(px::FAINT))
            .child(text(tag).font(mono).color(ink))
            .child(text(l.text.clone()).font(mono).color(if l.level == Level::Info { px::SOFT } else { ink }).wrap().selectable(true).grow(1.0).min_w(0.0))
    });
    let body = col().key(id.key("log-lines")).w_full().max_h(220.0).py(8.0).px(10.0).gap(3.0).scroll(rune::ScrollSpec { stick_bottom: true }).children(lines);
    let copy = px::key_sm("Copy log", false).key(id.key("log-copy")).on_press_tx(move |s: &mut App, tx: &mut rune::Tx| {
        if let Some(j) = s.jobs.get(id) {
            tx.copy(j.log_text());
        }
    });
    col()
        .key(id.key("log"))
        .w_full()
        .px(INSET)
        .pb(10.0)
        .child(col().w_full().radius(4.0).fill(px::SCREEN).border(1.0, px::LINE).child(body).child(row().w_full().px(4.0).pb(4.0).child(spacer()).child(copy)))
}

/// A thin progress bar. Indeterminate while the job is still counting its work.
fn bar(job: &Job, w: f32, _k: &Kit) -> El {
    let ink = if job.phase == Phase::Stopping { px::FAINT } else { px::AMBER };
    let fill = el().h_full().radius(2.0).fill(ink);
    let fill = match job.fraction() {
        Some(f) => fill.w_frac(f),
        None => fill.w_frac(0.3).looping(rune::LoopSpec::slide(1.4, [-1.0, 0.0], [3.4, 0.0])),
    };
    row().w(w).h(4.0).radius(1.0).fill(px::GRILLE).clip_soft(0.0).child(fill)
}

fn glyph(i: &'static Icon, ink: rune::Color) -> El {
    rune::icon(i, 12.0).color(ink)
}

fn chip_label(job: &Job) -> String {
    match (&job.phase, job.fraction()) {
        (Phase::Running, Some(f)) => format!("{}, {:.0} percent", job.title, f * 100.0),
        (Phase::Running, None) => job.title.clone(),
        (Phase::Stopping, _) => format!("Stopping {}", job.kind.noun()),
        (Phase::Done { outcome: Outcome::Completed(m) | Outcome::Stopped(m) | Outcome::Failed(m), .. }, _) => m.clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::jobs::{JobKind, Phase};
    use rune::{Env, Runtime};

    fn press(rt: &mut Runtime<App>, key: &str) {
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
    }

    fn rt_with_two_jobs() -> (Runtime<App>, crate::jobs::JobId, crate::jobs::JobId) {
        let mut app = crate::scenes::test_app();
        let export = app.jobs.stage(JobKind::Fetch, "Downloading take-1", "take-1.mp3", 10, Some(100), Phase::Running);
        let analysis = app.jobs.stage(JobKind::Earshot, "earshot describe · take-2", "take-2.wav", 3, Some(48), Phase::Running);
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(app, crate::ui::view, (1280.0, 800.0), env);
        rt.frame(0.0);
        (rt, export, analysis)
    }

    #[test]
    fn the_stop_button_in_the_analysis_chip_stops_only_the_analysis() {
        let (mut rt, export, analysis) = rt_with_two_jobs();
        press(&mut rt, &analysis.key("action"));
        assert_eq!(rt.state.jobs.get(analysis).unwrap().phase, Phase::Stopping);
        assert_eq!(rt.state.jobs.get(export).unwrap().phase, Phase::Running);
    }

    #[test]
    fn the_stop_button_in_the_export_chip_stops_only_the_export() {
        let (mut rt, export, analysis) = rt_with_two_jobs();
        press(&mut rt, &export.key("action"));
        assert_eq!(rt.state.jobs.get(export).unwrap().phase, Phase::Stopping);
        assert_eq!(rt.state.jobs.get(analysis).unwrap().phase, Phase::Running);
    }

    #[test]
    fn each_stop_button_sits_inside_its_own_chip() {
        let (mut rt, export, analysis) = rt_with_two_jobs();
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let footer = rt.presented("statusbar").unwrap().0;
        for id in [export, analysis] {
            let chip = rt.presented(&id.key("chip")).unwrap().0;
            assert!(chip[0] >= footer[0] && chip[0] + chip[2] <= footer[0] + footer[2], "job {id:?}'s chip spills out of the footer");
            let (bx, by) = rt.find(&id.key("action")).unwrap();
            assert!(bx >= chip[0] && bx <= chip[0] + chip[2] && by >= chip[1] && by <= chip[1] + chip[3], "job {id:?}'s stop button is outside its chip");
        }
    }

    /// With more jobs than fit, the extras fold into "+N more", and the Activity panel gives
    /// every job its own stop button there too.
    #[test]
    fn folded_jobs_stop_one_at_a_time_from_activity() {
        let mut app = crate::scenes::test_app();
        let ids: Vec<_> = (0..6).map(|i| app.jobs.stage(JobKind::Earshot, &format!("earshot batch {i}"), "", 1, Some(10), Phase::Running)).collect();
        let env = Env { system_dark: true, scale: 2.0 };
        let mut rt = Runtime::new(app, crate::ui::view, (1280.0, 800.0), env);
        rt.frame(0.0);
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find(&ids[0].key("chip")).is_none(), "the oldest job folds away");
        press(&mut rt, "status-more");
        assert!(rt.state.activity_open);
        press(&mut rt, &ids[0].key("activity-action"));
        assert_eq!(rt.state.jobs.get(ids[0]).unwrap().phase, Phase::Stopping);
        for id in &ids[1..] {
            assert_eq!(rt.state.jobs.get(*id).unwrap().phase, Phase::Running);
        }
    }
}
