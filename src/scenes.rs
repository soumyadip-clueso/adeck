//! Named states for headless snapshots (`adeck --snap out.png --scene compare`), built from
//! fixture audio adeck writes itself: tones, noise and a click track, never real recordings, with
//! sidecar metadata shaped like a generation worker's.

use std::path::{Path, PathBuf};

use crate::app::{App, TabKind};

type Scene = (&'static str, fn(&mut App));

pub const ALL: &[Scene] = &[("empty", empty), ("single", single), ("compare", compare), ("blind", blind), ("run", run), ("board", board), ("board-blind", board_blind), ("diff", diff), ("pads", pads)];

/// A folder of fixture files, written once per process.
pub fn fixtures() -> PathBuf {
    let dir = std::env::temp_dir().join("adeck-fixtures/run-a");
    if dir.join("tick.wav").is_file() {
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
    dir
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
    let target = serde_json::json!({"target_bands_db": {"sub": -30.0, "low": -12.0, "lowmid": -6.0, "mid": -14.0, "presence": -30.0}, "target_correlation": 0.4, "target_decay_ms": 300.0});
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
        for key in ["group-whoosh-soft", "group-ui-click", "tile-whoosh-soft-0", "bwave", "bspec", "analysis", "decide"] {
            assert!(rt.find(key).is_some(), "{key}");
        }
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
        let r = rt.state.call("board_upsert", &serde_json::json!({"tab_id": id, "groups": [
            {"key": "pop", "sort": 9999, "members": [{"item": tick, "role": "pick"}]},
            {"key": "ui-click", "sort": 1, "chip": {"text": "good now", "tone": "good"}, "members": [{"item": tick}]}
        ]}))
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
        // Narrow: the sheet takes the page, and closing it shows the tiles.
        for key in ["sheet", "analysis", "details", "bwave"] {
            let (r, _) = rt.presented(key).unwrap_or_else(|| panic!("{key}"));
            assert!(r[0] >= 0.0 && r[0] + r[2] <= 390.5, "{key} spills: {r:?}");
        }
        rt.state.toggle_sheet();
        rt.invalidate();
        for _ in 0..20 {
            rt.frame(1.0 / 60.0);
        }
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
        for key in ["ruler", "strip-grip", "scrubber", "volume", "zoom-in", "send"] {
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
        assert!(rt.find("note").is_none(), "the note field starts hidden");
        rt.state.toggle_note();
        rt.state.current_mut().unwrap().annotation_folded = true;
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("note").is_some() && rt.find("annotation-fold").is_some());
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
        assert!(rt.find("bwave").is_none(), "no strips until asked");
        assert!(crate::ui::pads::on_key(&mut rt.state, '2'));
        assert_eq!(rt.state.current().unwrap().active, 1);
        assert!(crate::ui::pads::on_key(&mut rt.state, 'c'));
        let id = rt.state.current().unwrap().spec.items[1].clone();
        assert_eq!(rt.state.scores.get(&id), Some(&2));
        // A flat list of files makes one row of pads.
        let files = ids(&["tick", "tone-a"]);
        let v = rt.state.call("open_pads", &serde_json::json!({"items": files, "label": "Clicks"})).unwrap();
        assert_eq!(v["board"]["groups"], 1);
        rt.state.current_mut().unwrap().sheet_open = true;
        rt.invalidate();
        rt.frame(1.0 / 60.0);
        assert!(rt.find("pad-pads-1").is_some() && rt.find("bwave").is_some());
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
