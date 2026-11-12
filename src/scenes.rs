//! Named states for headless snapshots (`adeck --snap out.png --scene compare`), built from
//! fixture audio adeck writes itself: tones, noise and a click track, never real recordings, with
//! sidecar metadata shaped like a generation worker's.

use std::path::{Path, PathBuf};

use crate::app::{App, TabKind};

type Scene = (&'static str, fn(&mut App));

pub const ALL: &[Scene] = &[
    ("empty", empty),
    ("single", single),
    ("compare", compare),
    ("blind", blind),
    ("run", run),
    ("board", board),
    ("board-blind", board_blind),
    ("diff", diff),
    ("pads", pads),
    ("shared", shared),
    ("activity", activity),
    ("palette", palette),
    ("palette-search", palette_search),
    ("board-search", board_search),
    ("keys", keys),
    ("stems", stems),
];

/// A folder of fixture files, written once per process.
pub fn fixtures() -> PathBuf {
    let dir = std::env::temp_dir().join("adeck-fixtures/run-a");
    if dir.join("tick.wav").is_file() {
        fixture_stems(&dir);
        return dir;
    }
    let _ = std::fs::create_dir_all(&dir);
    let rate = 44_100u32;
    write_wav(&dir.join("tone-a.wav"), rate, 4.0, |t| (t * 220.0 * std::f32::consts::TAU).sin() * 0.4 * env(t, 4.0));
    write_wav(&dir.join("tone-b.wav"), rate, 4.0, |t| ((t * 330.0 * std::f32::consts::TAU).sin() * 0.25 + (t * 660.0 * std::f32::consts::TAU).sin() * 0.1) * env(t, 4.0));
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    write_wav(&dir.join("whoosh.wav"), rate, 2.0, move |t| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let n = (seed as f32 / u64::MAX as f32) * 2.0 - 1.0;
        n * 0.5 * (std::f32::consts::PI * t / 2.0).sin().powi(2)
    });
    write_wav(&dir.join("click-120.wav"), rate, 10.0, |t| {
        let beat = (t * 2.0).fract();
        let down = ((t * 2.0) as u32).is_multiple_of(4);
        let click = if beat < 0.02 { (t * if down { 2000.0 } else { 1200.0 } * std::f32::consts::TAU).sin() * (1.0 - beat / 0.02) * 0.8 } else { 0.0 };
        click + (t * 110.0 * std::f32::consts::TAU).sin() * 0.15
    });
    // A one-shot: 60 ms of click in a second of silence.
    write_wav(&dir.join("tick.wav"), rate, 1.0, |t| if (0.42..0.48).contains(&t) { (t * 3000.0 * std::f32::consts::TAU).sin() * (1.0 - (t - 0.42) / 0.06) * 0.7 } else { 0.0 });
    let meta = |name: &str, model: &str, mood: &str, seed: u32, extra: serde_json::Value| {
        let mut v =
            serde_json::json!({"id": name, "model": model, "mood": mood, "seed": seed, "prompt": format!("{mood} test sound, fixture"), "steps": 8, "cfg": 1.0, "flags": []});
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        let _ = std::fs::write(dir.join(format!("{name}.json")), serde_json::to_vec_pretty(&v).unwrap_or_default());
    };
    meta("tone-a", "fixture-sine", "calm", 1, serde_json::json!({"analysis": {"bpm_asked": null}}));
    meta("tone-b", "fixture-sine", "calm", 2, serde_json::json!({"steps": 50, "cfg": 7.0}));
    meta("whoosh", "fixture-noise", "whoosh-soft", 3, serde_json::json!({"flags": ["too_short"]}));
    meta("click-120", "fixture-click", "upbeat", 4, serde_json::json!({"bpm": 120}));
    fixture_stems(&dir);
    dir
}

/// click-120 split twice, as the splitters would: four stems (clicks on drums, a loud low sine
/// on bass, a quiet chord on other, nothing on vocals), and a six-stem set beside it that hears
/// the chord's top as piano and a pluck as guitar.
fn fixture_stems(dir: &Path) {
    let track = dir.join("click-120.wav");
    let rate = 44_100u32;
    let tau = std::f32::consts::TAU;
    let drums = |t: f32| {
        let beat = (t * 2.0).fract();
        if beat < 0.02 { (t * 1200.0 * tau).sin() * (1.0 - beat / 0.02) * 0.5 } else { 0.0 }
    };
    let bass = |t: f32| (t * 55.0 * tau).sin() * 0.55 * (0.6 + 0.4 * (t * tau / 2.0).cos().abs());
    let gate = |t: f32| ((t * 0.5).fract() < 0.75) as u8 as f32;
    let d = crate::stems::dir_for(&track);
    if !d.join("stems.json").is_file() {
        let _ = std::fs::create_dir_all(&d);
        write_wav(&d.join("drums.wav"), rate, 10.0, drums);
        write_wav(&d.join("bass.wav"), rate, 10.0, bass);
        write_wav(&d.join("other.wav"), rate, 10.0, |t| ((t * 440.0 * tau).sin() + (t * 554.0 * tau).sin() + (t * 659.0 * tau).sin()) * 0.05 * gate(t));
        write_wav(&d.join("vocals.wav"), rate, 10.0, |_| 0.0);
        let levels = serde_json::json!({"mix_lufs": -14.2, "stems": {
            "drums": {"lufs": -24.0, "share": 0.09}, "bass": {"lufs": -15.1, "share": 0.69},
            "other": {"lufs": -20.4, "share": 0.21}, "vocals": {"lufs": -70.0, "share": 0.0}}});
        let meta = serde_json::json!({"version": 1, "model": "htdemucs_ft", "source": "click-120.wav", "stems": crate::stems::FOUR, "levels": levels});
        let _ = std::fs::write(d.join("stems.json"), serde_json::to_vec_pretty(&meta).unwrap_or_default());
    }
    let d = d.with_file_name("click-120.stems-htdemucs_6s");
    if !d.join("stems.json").is_file() {
        let _ = std::fs::create_dir_all(&d);
        write_wav(&d.join("drums.wav"), rate, 10.0, drums);
        write_wav(&d.join("bass.wav"), rate, 10.0, bass);
        write_wav(&d.join("guitar.wav"), rate, 10.0, |t| (t * 330.0 * tau).sin() * 0.08 * (-(t * 2.0).fract() * 6.0).exp());
        write_wav(&d.join("piano.wav"), rate, 10.0, |t| ((t * 554.0 * tau).sin() + (t * 659.0 * tau).sin()) * 0.05 * gate(t));
        write_wav(&d.join("other.wav"), rate, 10.0, |t| (t * 440.0 * tau).sin() * 0.03 * gate(t));
        write_wav(&d.join("vocals.wav"), rate, 10.0, |_| 0.0);
        // In the order this model writes them, not adeck's: the lanes sort them.
        let names = ["bass", "drums", "other", "vocals", "guitar", "piano"];
        let levels = serde_json::json!({"v": 2, "mix_lufs": -14.2, "stems": {
            "drums": {"lufs": -24.0, "share": 0.09}, "bass": {"lufs": -15.1, "share": 0.66},
            "guitar": {"lufs": -26.5, "share": 0.05}, "piano": {"lufs": -21.0, "share": 0.15},
            "other": {"lufs": -27.9, "share": 0.05}, "vocals": {"lufs": -70.0, "share": 0.0}}});
        let meta = serde_json::json!({"version": 1, "model": "htdemucs_6s.yaml", "source": "click-120.wav", "stems": names, "levels": levels});
        let _ = std::fs::write(d.join("stems.json"), serde_json::to_vec_pretty(&meta).unwrap_or_default());
    }
}

fn env(t: f32, secs: f32) -> f32 {
    (t / 0.05).min(1.0) * ((secs - t) / 0.3).clamp(0.0, 1.0)
}

fn write_wav(path: &Path, rate: u32, secs: f32, mut f: impl FnMut(f32) -> f32) {
    let n = (secs * rate as f32) as usize;
    let mut data = Vec::with_capacity(n * 4);
    for i in 0..n {
        let s = (f(i as f32 / rate as f32).clamp(-1.0, 1.0) * 32767.0) as i16;
        data.extend_from_slice(&s.to_le_bytes());
        data.extend_from_slice(&s.to_le_bytes());
    }
    let mut w = Vec::with_capacity(44 + data.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    let _ = std::fs::write(path, w);
}

fn ids(names: &[&str]) -> Vec<String> {
    let dir = fixtures();
    names.iter().map(|n| dir.join(format!("{n}.wav")).display().to_string()).collect()
}

/// Scenes start from no tabs, whatever the session held, and point the run roots at the fixtures.
fn reset(s: &mut App) {
    let ids: Vec<u64> = s.tabs.iter().map(|t| t.id()).collect();
    for id in ids {
        s.close(id);
    }
    s.cfg.run_roots = vec![fixtures().parent().map(Path::to_path_buf).unwrap_or_default()];
    s.headless = true;
}

fn empty(s: &mut App) {
    reset(s);
}

fn single(s: &mut App) {
    reset(s);
    let id = s.open(TabKind::Single, ids(&["click-120"]), None, false, true, None, None);
    s.annotate(id, "Listen for the **downbeat** click: it should land on every bar line.".into());
    s.settle_now();
}

fn activity(s: &mut App) {
    single(s);
    use crate::jobs::{JobKind, Phase};
    s.jobs.stage(JobKind::Earshot, "earshot describe · click-120", "click-120.wav", 2, Some(5), Phase::Running);
    s.jobs.stage(JobKind::Fetch, "Downloading run-b", "take-4.wav", 14, Some(40), Phase::Running);
    s.activity_open = true;
    s.pane_tab = "EARSHOT".into();
}

fn compare(s: &mut App) {
    reset(s);
    let id = s.open(TabKind::Compare, ids(&["tone-a", "tone-b", "whoosh"]), Some("Tones vs whoosh".into()), false, true, None, None);
    s.annotate(id, "B has more upper harmonics; the match should make A and B equally loud.".into());
    s.settle_now();
}

fn blind(s: &mut App) {
    reset(s);
    s.open(TabKind::Compare, ids(&["tone-a", "tone-b"]), None, true, true, None, None);
    s.settle_now();
}

fn run(s: &mut App) {
    reset(s);
    s.open(TabKind::Run, Vec::new(), None, false, true, Some("run-a".into()), None);
    s.settle_now();
}

/// A board shaped like the best-picks page: two categories, picks, a before, a ranked rest.
pub fn board_args() -> serde_json::Value {
    let [a, b, w, c, tick] = <[String; 5]>::try_from(ids(&["tone-a", "tone-b", "whoosh", "click-120", "tick"])).unwrap_or_default();
    let target =
        serde_json::json!({"target_bands_db": {"sub": -30.0, "low": -12.0, "lowmid": -6.0, "mid": -14.0, "presence": -30.0}, "target_correlation": 0.4, "target_decay_ms": 300.0});
    let measure = |sub: f64, corr: f64, decay: f64| serde_json::json!({"bands_db": {"sub": sub, "low": -10.0, "lowmid": -8.0, "mid": -15.0, "presence": -33.0}, "correlation": corr, "decay_ms": decay});
    serde_json::json!({
        "label": "Best picks",
        "choices": ["keep", "drop"],
        "move_to": ["whoosh-soft", "ui-click"],
        "metrics": [{"key": "vibe", "better": "higher"}, {"key": "shape", "better": "lower"}],
        "groups": [
            {"key": "ui-click", "title": "UI click", "subtitle": "528 searches", "sort": 528, "chip": {"text": "Close, still a bit synthetic", "tone": "close"}, "target": target,
             "members": [
                {"item": tick, "role": "pick", "numbers": {"vibe": 0.97, "shape": 0.62, "z": 2.4, "p_real": 0.55}, "meta": {"prompt": "crisp ui click", "model": "sa3-small", "seed": 11, "chain": "v2"}, "measure": measure(-28.0, 0.95, 120.0)},
                {"item": c, "role": "before", "numbers": {"vibe": 0.81}, "meta": {"chain": "raw"}},
             ]},
            {"key": "whoosh-soft", "title": "Soft whoosh", "subtitle": "846 searches", "sort": 846, "chip": {"text": "Sounds like the pack", "tone": "good"}, "target": target,
             "members": [
                {"item": w, "role": "pick", "numbers": {"vibe": 1.05, "shape": 0.51, "z": 3.2, "p_real": 0.93}, "chip": {"text": "good", "tone": "good"}, "meta": {"prompt": "soft airy whoosh", "model": "sa3-small", "seed": 7, "chain": "v2"}, "measure": measure(-26.0, 0.68, 725.0)},
                {"item": a, "role": "pick", "numbers": {"vibe": 1.01, "shape": 0.63}},
                {"item": b, "role": "before", "numbers": {"vibe": 0.89, "shape": 1.31}, "meta": {"chain": "raw"}},
             ],
             "more": [{"item": c, "numbers": {"vibe": 0.86, "shape": 1.1}}]}
        ]
    })
}

fn board(s: &mut App) {
    reset(s);
    let _ = s.call("open_board", &board_args());
    s.settle_now();
}

/// A board with the stem lanes open under a track, its six-stem set picked, the piano soloed.
fn stems(s: &mut App) {
    reset(s);
    let _ = s.call("open_board", &board_args());
    s.settle_now();
    s.toggle_stems("ui-click", 1);
    s.settle_now();
    s.set_stem_set("htdemucs_6s");
    s.stem_press("piano", false);
    s.settle_now();
}

fn board_blind(s: &mut App) {
    reset(s);
    let mut a = board_args();
    a["blind"] = serde_json::Value::Bool(true);
    let _ = s.call("open_board", &a);
    s.settle_now();
}

fn pads(s: &mut App) {
    reset(s);
    let mut a = board_args();
    a["label"] = serde_json::json!("UI sounds");
    let _ = s.call("open_pads", &a);
    s.settle_now();
}

/// ⌘K over a board, empty and then searching.
fn palette(s: &mut App) {
    board(s);
    crate::ui::palette::open(s);
}

fn palette_search(s: &mut App) {
    board(s);
    crate::ui::palette::open(s);
    if let Some(p) = s.palette.as_mut() {
        p.query = rune::TextEdit::new("tone");
        p.query.focused = true;
    }
}

fn board_search(s: &mut App) {
    board(s);
    if let Some(t) = s.current_mut() {
        t.filter = rune::TextEdit::new("click");
    }
}

fn keys(s: &mut App) {
    board(s);
    s.keys_help = true;
}

fn shared(s: &mut App) {
    reset(s);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let share = |slug: &str, title: &str, ago: u64, files: usize, audio: &str, gone: bool| crate::share::Share {
        slug: slug.into(),
        title: title.into(),
        url: format!("https://pages.example.com/{slug}"),
        created: now - ago,
        files,
        audio: audio.into(),
        gone,
    };
    s.shares = vec![
        share("best-picks", "Best picks", 600, 7, "bundle", false),
        share("tones-vs-whoosh", "Tones vs whoosh", 7200, 3, "s3://bucket/shares/tones", false),
        share("ui-sounds", "UI sounds", 3 * 86400, 12, "bundle", true),
    ];
    s.open_shared();
    s.shares_checked = Some(Ok(()));
}

fn diff(s: &mut App) {
    reset(s);
    let [a, b] = <[String; 2]>::try_from(ids(&["tone-a", "tone-b"])).unwrap_or_default();
    let _ = s.call(
        "open_diff",
        &serde_json::json!({
            "a": "chain v1", "b": "chain v2", "label": "v1 vs v2",
            "metrics": [{"key": "vibe", "better": "higher"}, {"key": "shape", "better": "lower"}, {"key": "z", "better": "higher"}, {"key": "p_real", "label": "pro-made", "better": "higher"}],
            "rows": [
                {"key": "whoosh-soft", "title": "Soft whoosh", "sort": 846, "a": {"vibe": 1.05, "shape": 0.593, "z": 3.11, "p_real": 0.921}, "b": {"vibe": 1.054, "shape": 0.506, "z": 3.16, "p_real": 0.927}, "items": {"a": a, "b": b}},
                {"key": "impact-cinematic", "title": "Cinematic impact", "sort": 598, "a": {"vibe": 0.91, "shape": 0.8, "z": 2.1, "p_real": 0.4}, "b": {"vibe": 0.88, "shape": 0.7, "z": 2.4, "p_real": 0.38}},
                {"key": "ui-click", "title": "UI click", "sort": 528, "a": {"vibe": 0.97, "shape": 0.62}, "b": {"vibe": 0.97, "shape": 0.64}}
            ]
        }),
    );
    s.settle_now();
}

/// An app on an empty config home, for tests.
#[cfg(test)]
pub fn test_app() -> App {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let home = std::env::temp_dir().join(format!("adeck-test-home-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&home);
        // SAFETY: set once, before any test thread reads these.
        unsafe {
            std::env::set_var("ADECK_HOME", &home);
            std::env::set_var("ADECK_CACHE", home.join("cache"));
            std::env::set_var("ADECK_HEADLESS", "1");
        }
    });
    App::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    fn rt(scene: fn(&mut App)) -> Runtime<App> {
        let mut app = test_app();
        scene(&mut app);
        let mut rt = Runtime::new(app, crate::ui::view, (1280.0, 800.0), Env { system_dark: true, scale: 1.0 });
        for _ in 0..20 {
            rt.frame(1.0 / 60.0);
        }
        rt
    }

    #[test]
    fn a_single_tab_shows_its_waveform_metadata_and_grid() {
        let rt = rt(single);
        let t = rt.state.current().unwrap();
        let e = rt.state.entry(&t.spec.items[0]).unwrap();
        let a = e.analysis.as_ref().expect("analysed");
        assert!((a.secs - 10.0).abs() < 0.05);
        assert!(a.bpm.is_some_and(|b| (b - 120.0).abs() < 2.0), "{:?}", a.bpm);
        assert_eq!(e.item.get("model"), Some("fixture-click"));
        assert!(rt.find("wave").is_some() && rt.find("spec").is_some() && rt.find("annotation").is_some());
    }

    #[test]
    fn a_compare_lists_only_what_differs_and_trims_to_one_loudness() {
        let rt = rt(compare);
        let s = &rt.state;
        let t = s.current().unwrap();
        let items: Vec<&crate::item::Item> = t.spec.items.iter().map(|i| &s.entry(i).unwrap().item).collect();
        let diff = crate::item::differing(&items);
        assert!(diff.contains(&"seed".to_owned()) && diff.contains(&"model".to_owned()));
        assert!(!diff.contains(&"cfg".to_owned()) || items[0].get("cfg") != items[1].get("cfg"));
        // Each trimmed member lands on the match target.
        for i in &t.spec.items {
            let (l, d) = (s.lufs(i).unwrap(), s.trim_db(t, i).unwrap());
            assert!((l + d - crate::app::MATCH_LUFS).abs() < 0.01 || d >= 12.0);
        }
        for m in 0..3 {
            assert!(rt.find(&format!("member-{m}")).is_some());
        }
    }

    #[test]
    fn a_blind_pick_reveals_and_records_the_map() {
        let mut rt = rt(blind);
        assert!(rt.state.current().unwrap().is_blind_hidden());
        let before = rt.state.feedback.read_all().len();
        rt.state.set_active_member(1);
        rt.state.pick();
        let t = rt.state.current().unwrap();
        assert!(!t.is_blind_hidden());
        let rows = rt.state.feedback.read_all();
        assert_eq!(rows.len(), before + 1);
        let row = rows.last().unwrap();
        assert_eq!(row.kind, "pick");
        assert_eq!(row.value, serde_json::Value::String(t.spec.items[1].clone()));
        let map = row.reveal_map.as_ref().unwrap().as_object().unwrap();
        assert_eq!(map.len(), 2);
        assert!(map.values().any(|v| v == &serde_json::Value::String(t.spec.items[1].clone())));
    }

    #[test]
    fn a_board_sorts_groups_focuses_the_first_and_plays_its_members() {
        let mut rt = rt(board);
        let t = rt.state.current().unwrap();
        assert_eq!(t.spec.kind, TabKind::Board);
        // Sorted by searches: whoosh-soft (846) first, so it's in focus and its members (and rest) play.
        assert_eq!(t.spec.group.as_deref(), Some("whoosh-soft"));
        assert_eq!(t.spec.items.len(), 4);
        for key in ["group-whoosh-soft", "group-ui-click", "tile-whoosh-soft-0", "bwave", "bspec", "decide"] {
            assert!(rt.find(key).is_some(), "{key}");
        }
        assert!(rt.find("analysis").is_none(), "the target meters are gone");
        rt.state.board_play("ui-click", 0);
        let t = rt.state.current().unwrap();
        assert_eq!(t.spec.group.as_deref(), Some("ui-click"));
        assert_eq!(t.active, 0);
        // The one-shot is zoomed to its 60 ms of sound.
        let e = rt.state.entry(&t.spec.items[0]).unwrap();
        let (a, b) = e.sound.unwrap();
        assert!(a > 0.38 && b < 0.52, "{a}..{b}");
        rt.state.board_step_group(1);
        assert_eq!(rt.state.current().unwrap().spec.group.as_deref(), Some("whoosh-soft"));
    }

    #[test]
    fn board_groups_and_diff_rows_upsert_live() {
        let mut rt = rt(board);
        let id = rt.state.active.unwrap();
        let tick = ids(&["tick"]).remove(0);
        let r = rt
            .state
            .call(
                "board_upsert",
                &serde_json::json!({"tab_id": id, "groups": [
                    {"key": "pop", "sort": 9999, "members": [{"item": tick, "role": "pick"}]},
                    {"key": "ui-click", "sort": 1, "chip": {"text": "good now", "tone": "good"}, "members": [{"item": tick}]}
                ]}),
            )
            .unwrap();
        assert_eq!((r["added"].as_u64(), r["replaced"].as_u64(), r["groups"].as_u64()), (Some(1), Some(1), Some(3)));
        let b = rt.state.current().unwrap().board().unwrap();
        assert_eq!(b.sorted()[0].key, "pop");
        assert_eq!(b.tally(), vec![("good".to_owned(), 2)]);
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("group-pop").is_some());

        let mut rt = super::tests::rt(diff);
        let id = rt.state.active.unwrap();
        rt.state.call("board_upsert", &serde_json::json!({"tab_id": id, "rows": [{"key": "pop", "a": {"vibe": 0.5}, "b": {"vibe": 0.7}}]})).unwrap();
        let d = rt.state.current().unwrap().spec.diff.clone().unwrap();
        assert_eq!(d.rows.len(), 4);
        assert_eq!(d.tally()[0].0, 2);
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("drow-pop").is_some() && rt.find("diff-tally").is_some());
    }

    #[test]
    fn decisions_persist_and_come_back_over_the_tool() {
        let mut rt = rt(board);
        let id = rt.state.active.unwrap();
        rt.state.decide_active("drop");
        let first = rt.state.current().unwrap().spec.items[0].clone();
        rt.state.decide(&first, "move:ui-click");
        let v = rt.state.call("get_decisions", &serde_json::json!({"tab_id": id})).unwrap();
        assert_eq!(v["decisions"][&first]["decision"], "move:ui-click");
        // A fresh app reads them back from disk.
        assert_eq!(test_app().decisions.get(&first).map(String::as_str), Some("move:ui-click"));
    }

    #[test]
    fn a_blind_board_hides_roles_until_the_group_is_picked() {
        let mut rt = rt(board_blind);
        let t = rt.state.current().unwrap();
        let g = t.focused().unwrap().clone();
        let labels: Vec<String> = (0..g.members.len()).map(|i| t.board_label(&g, i)).collect();
        let mut sorted = labels.clone();
        sorted.sort();
        assert_eq!(sorted, ["X", "Y", "Z"]);
        assert!(rt.find("analysis").is_none());
        rt.state.board_pick();
        let t = rt.state.current().unwrap();
        assert!(!t.group_hidden(&g.key));
        assert_eq!(t.board_label(&g, 0), "Pick 1");
        let row = rt.state.feedback.read_all().pop().unwrap();
        assert_eq!(row.reveal_map.unwrap().as_object().unwrap().len(), 3);
    }

    #[test]
    fn a_board_fits_a_phone_width_without_spilling() {
        let mut app = test_app();
        board(&mut app);
        let mut rt = Runtime::new(app, crate::ui::view, (390.0, 760.0), Env { system_dark: false, scale: 1.0 });
        for _ in 0..20 {
            rt.frame(1.0 / 60.0);
        }
        // Narrow: no pane, just the tiles and the player.
        assert!(rt.find("pane").is_none() && rt.find("pane-edge").is_none());
        for key in ["tile-whoosh-soft-0", "tile-whoosh-soft-2", "board-head"] {
            let (r, _) = rt.presented(key).unwrap_or_else(|| panic!("{key}"));
            assert!(r[0] >= 0.0 && r[0] + r[2] <= 390.5, "{key} spills: {r:?}");
        }
    }

    #[test]
    fn files_dropped_on_a_single_make_it_a_compare_and_members_come_off() {
        let mut rt = rt(single);
        let extra = ids(&["tone-a", "tone-b"]);
        rt.state.open_files(extra.iter().map(std::path::PathBuf::from).collect());
        let t = rt.state.current().unwrap();
        assert_eq!(t.spec.kind, TabKind::Compare);
        assert_eq!(t.spec.items.len(), 3);
        assert_eq!(rt.state.tabs.len(), 1, "they joined the open tab");
        rt.state.remove_member(0);
        assert_eq!(rt.state.current().unwrap().spec.items, extra);
        rt.state.new_compare();
        assert!(rt.state.current().unwrap().spec.items.is_empty());
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("drop-zone").is_some());
    }

    #[test]
    fn zoom_narrows_the_window_around_the_playhead_and_fit_undoes_it() {
        let mut rt = rt(single);
        let t = rt.state.current().unwrap();
        assert_eq!(rt.state.window(t), (0.0, 10.0));
        rt.state.zoom(0.5);
        rt.state.zoom(0.5);
        let (a, b) = rt.state.window(rt.state.current().unwrap());
        assert!((b - a - 2.5).abs() < 1e-6, "{a}..{b}");
        rt.state.pan(0.5);
        let (a2, _) = rt.state.window(rt.state.current().unwrap());
        assert!(a2 > a);
        rt.state.zoom_fit();
        assert_eq!(rt.state.window(rt.state.current().unwrap()), (0.0, 10.0));
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        for key in ["ruler", "zoom-fit", "scrubber", "volume", "zoom-in", "send"] {
            assert!(rt.find(key).is_some(), "{key}");
        }
    }

    #[test]
    fn sending_to_the_agent_lands_as_a_reference_it_can_read() {
        let mut rt = rt(board);
        rt.state.set_loop(Some((0.2, 0.6)));
        let text = rt.state.reference(Some(1)).unwrap();
        assert!(text.contains("Soft whoosh") && text.contains("Pick 2"), "{text}");
        let v = rt.state.call("get_selection", &serde_json::json!({})).unwrap();
        let r = &v["references"][0]["ref"];
        assert_eq!(r["group"], "whoosh-soft");
        assert_eq!(r["member"], 1);
        assert_eq!(r["range_is"], "loop");
        assert!(r["item"].as_str().unwrap().ends_with("tone-a.wav"));
    }

    #[test]
    fn the_note_and_the_agents_note_fold_away() {
        let mut rt = rt(single);
        assert!(rt.find("pane-note").is_none(), "the pane opens on META");
        rt.state.set_pane(crate::ui::px::Dock::Hidden);
        rt.state.show_note();
        rt.state.current_mut().unwrap().annotation_folded = true;
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("pane-note").is_some(), "N brings the pane back on the note");
        assert!(rt.find("annotation-fold").is_some());
        let id = rt.state.active.unwrap();
        rt.state.annotate(id, String::new());
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("annotation").is_none());
    }

    #[test]
    fn pads_play_from_the_top_on_their_key_and_score_on_the_pad() {
        let mut rt = rt(pads);
        let t = rt.state.current().unwrap();
        assert!(t.board().unwrap().is_pads());
        for key in ["pad-whoosh-soft-0", "pad-whoosh-soft-3", "pad-ui-click-0", "pad-score-whoosh-soft-0-2"] {
            assert!(rt.find(key).is_some(), "{key}");
        }
        rt.state.set_pane(crate::ui::px::Dock::Hidden);
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("bwave").is_none() && rt.find("pane-edge").is_some(), "hidden pane, no strips");
        assert!(crate::ui::pads::on_key(&mut rt.state, '2'));
        assert_eq!(rt.state.current().unwrap().active, 1);
        assert!(crate::ui::pads::on_key(&mut rt.state, 'c'));
        let id = rt.state.current().unwrap().spec.items[1].clone();
        assert_eq!(rt.state.scores.get(&id), Some(&2));
        // A flat list of files makes one row of pads.
        let files = ids(&["tick", "tone-a"]);
        let v = rt.state.call("open_pads", &serde_json::json!({"items": files, "label": "Clicks"})).unwrap();
        assert_eq!(v["board"]["groups"], 1);
        rt.state.toggle_pane();
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("pad-pads-1").is_some() && rt.find("bwave").is_some());
    }

    #[test]
    fn the_pane_resizes_from_its_edge_and_moves_by_its_grip() {
        let mut rt = rt(compare);
        let drag = |rt: &mut Runtime<App>, key: &str, dx: f32| {
            let ([x, y, w, h], _) = rt.presented(key).unwrap_or_else(|| panic!("{key}"));
            let (px, py) = (x + w / 2.0, y + h / 2.0);
            rt.pointer_down(px, py);
            rt.frame(1.0 / 60.0);
            for i in 1..=4 {
                rt.pointer_move(px + dx * i as f32 / 4.0, py);
                rt.frame(1.0 / 60.0);
            }
            rt.pointer_up(px + dx, py);
            for _ in 0..40 {
                rt.frame(1.0 / 60.0);
            }
        };
        let before = rt.presented("pane").unwrap().0[2];
        drag(&mut rt, "pane-resize", -120.0);
        let after = rt.presented("pane").unwrap().0[2];
        assert!((after - before - 120.0).abs() < 2.0, "wider by the drag: {before} -> {after}");
        drag(&mut rt, "pane-grip", -400.0);
        assert_eq!(rt.state.pane, crate::ui::px::Dock::Left, "dragged across, it docks left");
    }

    /// Every keyed element's laid-out rect, outside the pane (which is allowed to change when
    /// its subject does) and the transient overlays.
    fn layout(rt: &mut Runtime<App>) -> std::collections::BTreeMap<String, [f32; 4]> {
        for _ in 0..40 {
            rt.frame(1.0 / 60.0);
        }
        rt.elements()
            .into_iter()
            .filter(|e| !e.exiting && !in_pane(&e.path) && !e.path.contains("toast") && !e.path.contains("tip"))
            .filter_map(|e| e.key.map(|k| (format!("{k} @ {}", e.path.rsplit('/').nth(1).unwrap_or("")), e.target)))
            .collect()
    }

    type Scene = fn(&mut App);

    /// Inside the side pane (its own subject changes it) rather than the page beside it.
    fn in_pane(path: &str) -> bool {
        path.split('/').any(|seg| seg == "pane" || seg.starts_with("pane-") || seg == "pane-drop")
    }

    fn moved(a: &std::collections::BTreeMap<String, [f32; 4]>, b: &std::collections::BTreeMap<String, [f32; 4]>) -> Vec<String> {
        a.iter().filter_map(|(k, r)| b.get(k).filter(|q| r.iter().zip(q.iter()).any(|(x, y)| (x - y).abs() > 0.5)).map(|q| format!("{k}: {r:?} -> {q:?}"))).collect()
    }

    /// The everyday actions (switching what you hear, focusing a group, the pane's section,
    /// scores, hovering any control) never move anything on the page.
    /// No control on any page runs off the window, from a small window to a normal one.
    #[test]
    fn no_control_runs_off_the_window() {
        let scenes: Vec<(&str, Scene)> = vec![
            ("single", single),
            ("compare", compare),
            ("blind", blind),
            ("run", run),
            ("board", board),
            ("pads", pads),
            ("diff", diff),
            ("shared", shared),
            ("activity", activity),
        ];
        let mut bad = Vec::new();
        for (w, h) in [(760.0, 720.0), (1000.0, 700.0), (1280.0, 800.0)] {
            for (name, scene) in &scenes {
                let mut app = test_app();
                scene(&mut app);
                let mut rt = Runtime::new(app, crate::ui::view, (w, h), Env { system_dark: true, scale: 1.0 });
                for _ in 0..40 {
                    rt.frame(1.0 / 60.0);
                }
                for e in rt.elements() {
                    let clickable = e.focusable || e.role == "button" || e.role == "tab";
                    if clickable && !e.exiting && !e.inert && e.opacity > 0.01 && e.target[0] + e.target[2] > w + 0.5 {
                        bad.push(format!("{name} @ {w}: {} ends at {:.0}", e.key.unwrap_or(e.path), e.target[0] + e.target[2]));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "controls run off the window:\n{}", bad.join("\n"));
    }

    #[test]
    fn everyday_actions_never_shift_the_page() {
        type Act = (&'static str, Scene);
        let cases: Vec<(&str, Scene, Vec<Act>)> = vec![
            (
                "compare",
                compare,
                vec![
                    ("hear B", |s| s.set_active_member(1)),
                    ("hear C", |s| s.set_active_member(2)),
                    ("pane META", |s| s.pane_tab = "META".into()),
                    ("pane NOTE", |s| s.pane_tab = "NOTE".into()),
                    ("loop", |s| s.set_loop(Some((0.5, 1.5)))),
                ],
            ),
            (
                "board",
                board,
                vec![
                    ("focus ui-click", |s| s.focus_group("ui-click")),
                    ("focus whoosh", |s| s.focus_group("whoosh-soft")),
                    ("play tile 2", |s| s.board_play("whoosh-soft", 1)),
                    ("pane NOTE", |s| s.pane_tab = "NOTE".into()),
                    ("keep", |s| s.decide_active("keep")),
                ],
            ),
            ("pads", pads, vec![("play pad 2", |s| s.pad_play("whoosh-soft", 1)), ("score", |s| s.score_active(2))]),
            ("single", single, vec![("score 2", |s| s.score(2)), ("pane EARSHOT", |s| s.pane_tab = "EARSHOT".into())]),
            ("blind", blind, vec![("hear B", |s| s.set_active_member(1))]),
            ("run", run, vec![]),
            ("diff", diff, vec![]),
            ("shared", shared, vec![]),
            ("activity", activity, vec![]),
            ("empty", empty, vec![]),
        ];
        let mut bad = Vec::new();
        let mut hovered = Vec::new();
        for (name, scene, acts) in cases {
            for (what, act) in acts {
                let mut rt = rt(scene);
                let before = layout(&mut rt);
                act(&mut rt.state);
                rt.invalidate();
                let after = layout(&mut rt);
                for m in moved(&before, &after) {
                    bad.push(format!("{name} / {what}: {m}"));
                }
            }
            // Hovering any control.
            let mut rt = rt(scene);
            let before = layout(&mut rt);
            let keys: Vec<String> =
                rt.elements().into_iter().filter(|e| (e.focusable || e.role == "button" || e.role == "tab") && !e.inert && !in_pane(&e.path)).filter_map(|e| e.key).collect();
            hovered.push(format!("{name}: {}", keys.len()));
            for key in keys {
                if let Some((x, y)) = rt.find(&key) {
                    rt.pointer_move(x, y);
                    let after = layout(&mut rt);
                    for m in moved(&before, &after) {
                        bad.push(format!("{name} / hover {key}: {m}"));
                    }
                }
            }
        }
        assert!(hovered.len() >= 10, "the hover sweep found too few controls: {hovered:?}");
        assert!(bad.is_empty(), "the page shifted:\n{}", bad.join("\n"));
    }

    /// ⌘F puts the cursor in the page's search, so what you type next filters; on a page with
    /// no search it opens the command palette.
    #[test]
    fn command_f_searches() {
        let cmd_f = || rune::KeyEvent { key: rune::Key::Command('f'), mods: rune::Mods { cmd: true, logo: true, ..Default::default() } };
        for scene in [board as Scene, run, shared, pads] {
            let mut rt = rt(scene);
            rt.on_key = Some(crate::ui::on_key);
            // Even after clicking something else on the page first.
            if let Some((x, y)) = rt.find("play") {
                rt.pointer_down(x, y);
                rt.pointer_up(x, y);
            }
            rt.key(cmd_f());
            for _ in 0..5 {
                rt.frame(1.0 / 60.0);
            }
            rt.key(rune::KeyEvent { key: rune::Key::Text("w".into()), mods: rune::Mods::default() });
            rt.frame(1.0 / 60.0);
            let t = rt.state.current().expect("a tab");
            assert_eq!(t.filter.text, "w", "{:?}: typing after ⌘F didn't reach the search", t.spec.kind);
        }
        let mut rt = rt(single);
        rt.on_key = Some(crate::ui::on_key);
        rt.key(cmd_f());
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.state.palette.is_some(), "⌘F on a page with no search should open the palette");
    }

    /// Shift-drag across a waveform selects a stretch and plays nothing; L loops it, L again
    /// stops looping, shift-click clears it; a plain drag still only scrubs.
    #[test]
    fn a_tiles_download_button_saves_without_playing_the_tile() {
        let mut rt = rt(board);
        let click = |rt: &mut Runtime<App>, key: &str| {
            let r = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap_or_else(|| panic!("{key}")).target;
            rt.pointer_down(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
            rt.pointer_up(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
            rt.frame(1.0 / 60.0);
        };
        assert_eq!(rt.state.current().unwrap().spec.group.as_deref(), Some("whoosh-soft"));
        click(&mut rt, "dl-ui-click-0");
        assert_eq!(rt.state.current().unwrap().spec.group.as_deref(), Some("whoosh-soft"), "the press reached the tile");
        // The speed key flips 1× and 2×.
        click(&mut rt, "speed");
        assert_eq!(rt.state.mixer.speed, 2.0);
        click(&mut rt, "speed");
        assert_eq!(rt.state.mixer.speed, 1.0);
    }

    #[test]
    fn stem_lanes_open_under_a_tile_solo_and_close_with_t() {
        let mut rt = rt(board);
        let click = |rt: &mut Runtime<App>, key: &str| {
            let r = rt.elements().into_iter().find(|e| e.key.as_deref() == Some(key)).unwrap_or_else(|| panic!("{key}")).target;
            rt.pointer_down(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
            rt.pointer_up(r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
            rt.frame(1.0 / 60.0);
        };
        let has = |rt: &mut Runtime<App>, key: &str| rt.elements().iter().any(|e| e.key.as_deref() == Some(key) && !e.exiting);
        // The click track has stems on disk: its tile's key opens them under its row.
        click(&mut rt, "stems-ui-click-1");
        rt.state.settle_now();
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        let t = rt.state.current().unwrap();
        assert_eq!((t.spec.group.as_deref(), t.active, t.stems_open, t.stem_mode), (Some("ui-click"), 1, true, true));
        assert!(rt.state.open_stems(t).is_some_and(|st| st.bass_share() == Some(0.69)));
        for k in ["lanes-ui-click", "lane-row-drums", "lane-row-vocals", "ab-mix", "solo-bass", "mute-other"] {
            assert!(has(&mut rt, k), "{k}");
        }
        // Each stem resolved and analysed, so its lane has a waveform.
        let files = rt.state.open_stems(rt.state.current().unwrap()).unwrap().files.clone();
        assert!(files.iter().all(|p| rt.state.entry(&crate::app::stem_id(p)).is_some_and(|e| e.analysis.is_some())));
        // (The lanes sit below this window's fold, so these go through the app.)
        rt.state.stem_press("bass", false);
        rt.state.set_stem_mode(false);
        let t = rt.state.current().unwrap();
        assert_eq!((t.stem_solo.iter().map(String::as_str).collect::<Vec<_>>(), t.stem_mode), (vec!["bass"], false));
        rt.state.stem_press("drums", false);
        assert!(rt.state.current().unwrap().stem_mode, "a solo puts you back on the stems");
        // The track has a second, six-stem set: the header switches to it, solos carry by name.
        for _ in 0..4 {
            rt.frame(1.0 / 60.0);
        }
        assert!(has(&mut rt, "stem-set-main") && has(&mut rt, "stem-set-htdemucs_6s"));
        rt.state.set_stem_set("htdemucs_6s");
        rt.state.settle_now();
        rt.invalidate();
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        let t = rt.state.current().unwrap();
        let st = rt.state.open_stems(t).unwrap().clone();
        assert_eq!((st.label(), st.names.len()), ("htdemucs_6s".to_owned(), 6));
        assert_eq!(st.names, ["drums", "bass", "guitar", "piano", "vocals", "other"]);
        assert_eq!(crate::stems::gains(&st.names, &t.stem_solo, &t.stem_mute), [1.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(t.spec.stems.as_ref().map(|v| v.set.as_str()), Some("htdemucs_6s"), "the pick is kept with the tab");
        assert!(st.files.iter().all(|p| rt.state.entry(&crate::app::stem_id(p)).is_some_and(|e| e.analysis.is_some())));
        for k in ["lane-row-guitar", "lane-row-piano", "solo-piano"] {
            assert!(has(&mut rt, k), "{k}");
        }
        rt.on_key = Some(crate::ui::on_key);
        rt.key(rune::KeyEvent { key: rune::Key::Text("t".into()), mods: rune::Mods::default() });
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        assert!(!rt.state.current().unwrap().stems_open);
        assert!(!has(&mut rt, "lane-row-bass"));
        // A tile with no stems (and no splitter, headless) opens to the offer to split.
        click(&mut rt, "stems-ui-click-0");
        for _ in 0..10 {
            rt.frame(1.0 / 60.0);
        }
        assert!(has(&mut rt, "split-one"));
    }

    #[test]
    fn shift_drag_on_a_waveform_selects_a_stretch() {
        let shift = |rt: &mut Runtime<App>, on: bool| {
            rt.physical_input(rune::PhysicalInput::Modifiers(rune::Mods { shift: on, ..Default::default() }));
            rt.frame(0.0);
        };
        let mut rt = rt(single);
        let r = rt.elements().into_iter().find(|e| e.key.as_deref() == Some("wave")).expect("wave").target;
        let (y, x0, x1) = (r[1] + r[3] / 2.0, r[0] + r[2] * 0.25, r[0] + r[2] * 0.5);
        let total = rt.state.window(rt.state.current().unwrap());

        // A plain drag scrubs and sets no loop.
        rt.pointer_down(x0, y);
        rt.pointer_move(x1, y);
        rt.pointer_up(x1, y);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.current().unwrap().loop_secs.is_none(), "a plain drag made a loop");

        shift(&mut rt, true);
        rt.pointer_down(x0, y);
        rt.pointer_move((x0 + x1) / 2.0, y);
        rt.pointer_move(x1, y);
        rt.pointer_up(x1, y);
        rt.frame(1.0 / 60.0);
        let t = rt.state.current().unwrap();
        assert!(t.loop_secs.is_none(), "shift-drag shouldn't loop");
        let (a, b) = t.selection.expect("shift-drag made no selection");
        let span = total.1 - total.0;
        assert!((a - (total.0 + span * 0.25)).abs() < span * 0.02 && (b - (total.0 + span * 0.5)).abs() < span * 0.02, "selection {a}..{b} of {total:?}");
        assert!(!rt.state.mixer.wants_play() && !rt.state.mixer.is_playing(), "shift-drag shouldn't start playing");
        assert!((rt.state.mixer.secs() - a).abs() < span * 0.02, "the playhead isn't parked at the selection");

        // L loops the selection; L again stops looping and keeps it selected.
        rt.state.toggle_loop();
        assert_eq!(rt.state.current().unwrap().loop_secs, Some((a, b)));
        rt.state.toggle_loop();
        let t = rt.state.current().unwrap();
        assert!(t.loop_secs.is_none() && t.selection == Some((a, b)));

        // Shift-click with no drag clears it.
        rt.pointer_down(x1, y);
        rt.pointer_up(x1, y);
        rt.frame(1.0 / 60.0);
        shift(&mut rt, false);
        assert!(rt.state.current().unwrap().selection.is_none(), "shift-click didn't clear the selection");
    }

    /// A finished job leaves the footer after a few seconds (failures stay); the footer goes
    /// once there's nothing left in it.
    #[test]
    fn finished_jobs_fold_out_of_the_footer() {
        use crate::jobs::{JobKind, Outcome, Phase};
        let mut rt = rt(single);
        rt.poll = Some(|s: &mut App, clock: f64, tx: &mut rune::Tx| s.poll(clock, tx));
        rt.frame(1.0 / 60.0);
        let now = rt.state.clock;
        rt.state.jobs.stage(JobKind::Earshot, "earshot describe · a", "", 1, Some(1), Phase::Done { outcome: Outcome::Completed("earshot describe · a".into()), at: now });
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("statusbar").is_some(), "a just-finished job shows");
        for _ in 0..8 {
            rt.frame(1.0);
        }
        assert!(rt.find("statusbar").is_none(), "the footer should fold away once its jobs are old");
        rt.state.jobs.stage(JobKind::Fetch, "Downloading b", "", 0, None, Phase::Done { outcome: Outcome::Failed("no network".into()), at: rt.state.clock });
        rt.invalidate();
        for _ in 0..8 {
            rt.frame(1.0);
        }
        assert!(rt.find("statusbar").is_some(), "a failure stays until dismissed");
    }

    #[test]
    fn space_plays_whatever_was_clicked_last() {
        let space = || rune::KeyEvent { key: rune::Key::Text(" ".into()), mods: rune::Mods::default() };
        for (scene, keys) in [
            (compare as fn(&mut App), &["play", "hear-1", "member-1", "blind", "match", "scrubber", "wave-0", "pane-tab-meta", "zoom-in", "volume", "add-file", "note-toggle"][..]),
            (board as fn(&mut App), &["play", "tile-whoosh-soft-1", "blind", "zoom", "replay", "loop-toggle", "decide-keep"][..]),
            (single as fn(&mut App), &["play", "score-2", "wave", "pane-tab-earshot"][..]),
        ] {
            for key in keys {
                let mut rt = rt(scene);
                rt.on_key = Some(crate::ui::on_key);
                rt.set_full_keyboard_access(true);
                for _ in 0..20 {
                    rt.frame(1.0 / 60.0);
                }
                let Some((x, y)) = rt.find(key) else { panic!("no {key}") };
                rt.pointer_down(x, y);
                rt.pointer_up(x, y);
                for _ in 0..5 {
                    rt.frame(1.0 / 60.0);
                }
                let before = rt.state.mixer.wants_play() || rt.state.mixer.is_playing();
                rt.key(space());
                rt.frame(1.0 / 60.0);
                let after = rt.state.mixer.wants_play() || rt.state.mixer.is_playing();
                assert_ne!(before, after, "Space after clicking {key} didn't toggle play");
            }
        }
    }

    #[test]
    fn a_toast_closes_on_a_click() {
        let mut rt = rt(single);
        rt.state.toast = Some(("Saved score".into(), false));
        rt.invalidate();
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let ([x, y, w, h], _) = rt.presented("toast").expect("toast up");
        rt.pointer_down(x + w / 2.0, y + h / 2.0);
        rt.pointer_up(x + w / 2.0, y + h / 2.0);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.toast.is_none());
    }

    #[test]
    fn the_pane_docks_either_side_and_hides_to_an_edge() {
        let mut rt = rt(compare);
        let x = |rt: &mut Runtime<App>, key: &str| rt.presented(key).unwrap_or_else(|| panic!("{key}")).0[0];
        assert!(x(&mut rt, "pane") > x(&mut rt, "page"), "docked right by default");
        rt.state.set_pane(crate::ui::px::Dock::Left);
        rt.invalidate();
        for _ in 0..40 {
            rt.frame(1.0 / 60.0);
        }
        assert!(x(&mut rt, "pane") < x(&mut rt, "page"), "docked left");
        rt.state.toggle_pane();
        rt.invalidate();
        for _ in 0..40 {
            rt.frame(1.0 / 60.0);
        }
        assert!(rt.find("pane").is_none() && rt.find("pane-edge").is_some());
        assert!(x(&mut rt, "pane-edge") < x(&mut rt, "page"), "the edge tab stays on the side it left");
        rt.state.toggle_pane();
        assert_eq!(rt.state.pane, crate::ui::px::Dock::Left, "and comes back there");
    }

    #[test]
    fn sharing_publishes_a_page_without_local_paths_or_reference_audio() {
        let mut rt = rt(compare);
        let dir = std::env::temp_dir().join(format!("adeck-pub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("pub.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nd=$(dirname \"$0\")\ncase \"$1\" in\n  publish) cp -R \"$2\" \"$d/$3\"; echo \"{\\\"url\\\": \\\"https://pages.test/$3\\\"}\";;\n  remove) rm -rf \"$d/$2\"; echo '{}';;\n  list) echo '[]';;\nesac\n",
        )
        .unwrap();
        std::process::Command::new("chmod").arg("755").arg(&script).status().unwrap();
        let ids = rt.state.current().unwrap().spec.items.clone();
        let home = rt.state.entry(&ids[0]).unwrap().item.path.parent().unwrap().display().to_string();
        rt.state.items.get_mut(&ids[2]).unwrap().item.is_ref = true;
        rt.state.cfg.share.command = script.display().to_string();
        rt.state.share().unwrap();
        rt.state.settle_now();
        let sh = rt.state.shares.first().cloned().unwrap_or_else(|| panic!("not recorded: {:?}", rt.state.toast));
        assert!(sh.url.starts_with("https://pages.test/adeck-tones-vs-whoosh-"), "{}", sh.url);
        let out = dir.join(&sh.slug);
        let html = std::fs::read_to_string(out.join("index.html")).unwrap();
        assert!(!html.contains(&home) && !html.contains("/Users/"), "no local paths on the page");
        assert!(html.contains("\"peaks\":[") && html.contains("audio/00-"), "waveforms and audio links are in");
        assert_eq!(std::fs::read_dir(out.join("audio")).unwrap().count(), 2, "the reference stays home");
        rt.state.unshare(&sh.slug, false);
        assert_eq!(rt.state.confirm_unshare.as_deref(), Some(sh.slug.as_str()), "the first press only asks");
        rt.state.unshare(&sh.slug, false);
        for _ in 0..200 {
            rt.state.settle_now();
            if rt.state.shares.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(rt.state.shares.is_empty() && !out.exists(), "taken down");
    }

    #[test]
    fn command_k_finds_commands_tabs_and_sounds_and_runs_them() {
        let mut rt = rt(palette_search);
        assert!(rt.find("palette").is_some() && rt.find("pal-0").is_some(), "the palette is up");
        let hits = crate::ui::palette::results(&rt.state, "tone");
        assert!(hits.iter().any(|h| h.section == "Sounds"), "sounds are searchable: {:?}", hits.iter().map(|h| &h.title).collect::<Vec<_>>());
        let hits = crate::ui::palette::results(&rt.state, "blind");
        assert_eq!(hits[0].act, crate::ui::palette::Act::Cmd("blind"));
        // Clicking a row runs it and closes the palette.
        rt.state.palette.as_mut().unwrap().query = rune::TextEdit::new("blind");
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        let (x, y) = rt.find("pal-0").unwrap();
        let was = rt.state.current().unwrap().spec.blind;
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.palette.is_none(), "closed after running");
        crate::ui::palette::open(&mut rt.state);
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        rt.pointer_down(20.0, 760.0);
        rt.pointer_up(20.0, 760.0);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.palette.is_none(), "a click outside closes it");
        assert_ne!(rt.state.current().unwrap().spec.blind, was, "blind toggled");
    }

    #[test]
    fn the_keys_sheet_opens_from_the_palette_and_puts_this_tabs_keys_first() {
        let mut rt = rt(board);
        let hits = crate::ui::palette::results(&rt.state, "keyboard");
        assert_eq!(hits[0].act, crate::ui::palette::Act::Cmd("keys"));
        crate::ui::keys::toggle(&mut rt.state);
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("keys-help").is_some(), "the sheet is up");
        for _ in 0..30 {
            rt.frame(1.0 / 60.0);
        }
        let (x, y) = (20.0, 760.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert!(!rt.state.keys_help, "a click outside closes it");
    }

    #[test]
    fn searching_a_board_narrows_it_and_slash_focuses_the_box() {
        let mut rt = rt(board);
        assert_eq!(rt.state.shown_groups(rt.state.current().unwrap()).len(), 2);
        assert!(crate::ui::focus_search(&mut rt.state));
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.state.current().unwrap().filter.focused, "/ focuses the search");
        rt.state.current_mut().unwrap().filter = rune::TextEdit::new("ui click");
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        let shown: Vec<String> = rt.state.shown_groups(rt.state.current().unwrap()).iter().map(|g| g.key.clone()).collect();
        assert_eq!(shown, vec!["ui-click".to_owned()]);
        assert!(rt.find("group-whoosh-soft").is_none() && rt.find("tab-search").is_some());
    }

    #[test]
    fn dragging_across_the_waveform_scrubs() {
        let mut rt = rt(single);
        let (x, y) = rt.find("wave").expect("wave");
        let ([sx, _, sw, _], _) = rt.presented("wave").unwrap();
        rt.pointer_down(sx + sw * 0.2, y);
        rt.frame(1.0 / 60.0);
        let at_down = rt.state.mixer.secs();
        rt.pointer_move(sx + sw * 0.5, y);
        rt.frame(1.0 / 60.0);
        rt.pointer_move(sx + sw * 0.7, y);
        rt.frame(1.0 / 60.0);
        let mid = rt.state.mixer.secs();
        assert!(!rt.state.mixer.is_playing(), "scrubbing is silent");
        rt.pointer_up(sx + sw * 0.7, y);
        rt.frame(1.0 / 60.0);
        eprintln!("down {at_down} mid {mid} end {} x {x}", rt.state.mixer.secs());
        assert!((at_down - 2.0).abs() < 0.3, "press lands at 20%: {at_down}");
        assert!((mid - 7.0).abs() < 0.3, "drag follows: {mid}");
        assert!(!rt.state.mixer.is_playing(), "it was stopped, so it stays stopped");
    }

    #[test]
    fn a_run_tab_lists_every_fixture() {
        let rt = rt(run);
        let t = rt.state.current().unwrap();
        assert_eq!(t.rows.len(), 5, "{:?}", t.rows);
    }
}
