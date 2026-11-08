//! Named states for headless snapshots (`adeck --snap out.png --scene compare`), built from
//! fixture audio adeck writes itself: tones, noise and a click track, never real recordings, with
//! sidecar metadata shaped like a generation worker's.

use std::path::{Path, PathBuf};

use crate::app::{App, TabKind};

type Scene = (&'static str, fn(&mut App));

pub const ALL: &[Scene] = &[("empty", empty), ("single", single), ("compare", compare), ("blind", blind), ("run", run)];

/// A folder of fixture files, written once per process.
pub fn fixtures() -> PathBuf {
    let dir = std::env::temp_dir().join("adeck-fixtures/run-a");
    if dir.join("click-120.json").is_file() {
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
    fn a_run_tab_lists_every_fixture() {
        let rt = rt(run);
        let t = rt.state.current().unwrap();
        assert_eq!(t.rows.len(), 4, "{:?}", t.rows);
    }
}
