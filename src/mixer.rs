//! Playback for one tab: N members that all run on one clock, of which you hear one.
//!
//! Every member is decoded to the device's rate up front and the audio callback walks them with
//! a single cursor, so switching moves only gains (a 5 ms crossfade) and lands at exactly the
//! same moment with no gap. Each member has a trim (the loudness match). The callback owns its
//! state; the UI sends it commands over a channel and reads the position from an atomic.
//!
//! The device opens on a worker the first time something plays, never at launch, and never in a
//! headless run.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::decode::{self, Pcm};

/// Crossfade length on a switch, in seconds.
const XFADE: f32 = 0.005;

type Paths = Vec<(String, std::path::PathBuf)>;
type Decoded = Result<(Vec<Arc<Vec<f32>>>, u64), String>;
type Opened = Result<(Sender<Cmd>, u32), String>;

enum Cmd {
    Load { set: u64, tracks: Vec<Arc<Vec<f32>>> },
    Play,
    Pause,
    Seek(u64),
    Active(usize),
    Trims(Vec<f32>),
    Loop(Option<(u64, u64)>),
    Volume(f32),
}

struct Shared {
    pos: AtomicU64,
    playing: AtomicBool,
    /// The set the callback has loaded, so the UI knows its load landed.
    set: AtomicU64,
    ended: AtomicBool,
    /// The loudest sample of the last buffer out, as f32 bits, for the level meter.
    level: AtomicU32,
}

/// What the UI holds.
pub struct Mixer {
    tx: Option<Sender<Cmd>>,
    shared: Arc<Shared>,
    pub rate: u32,
    opening: Option<Receiver<Opened>>,
    /// Commands sent before the device opened, replayed once it has.
    queued: Vec<Cmd>,
    pub error: Option<String>,
    /// Which set of files is loaded, by the tab that loaded it and its member ids.
    pub loaded: Option<(u64, Vec<String>)>,
    pub frames: u64,
    next_set: u64,
    /// Decodes in flight: (set, result).
    decoding: Option<(u64, Receiver<Decoded>)>,
    pub active: usize,
    pub loop_frames: Option<(u64, u64)>,
    want_play: bool,
    waker: Option<rune::Waker>,
    /// A load waiting for the device to open.
    pending: Option<(u64, Paths)>,
    /// Output volume, 0 to 1.5 (linear).
    pub volume: f32,
    /// A seek asked for while the files were still decoding, in seconds, applied once they land.
    pending_seek: Option<f64>,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer {
            tx: None,
            shared: Arc::new(Shared { pos: AtomicU64::new(0), playing: AtomicBool::new(false), set: AtomicU64::new(0), ended: AtomicBool::new(false), level: AtomicU32::new(0) }),
            rate: 48_000,
            opening: None,
            queued: Vec::new(),
            error: None,
            loaded: None,
            frames: 0,
            next_set: 1,
            decoding: None,
            active: 0,
            loop_frames: None,
            want_play: false,
            waker: None,
            pending: None,
            volume: 1.0,
            pending_seek: None,
        }
    }
}

impl Mixer {
    pub fn set_waker(&mut self, w: rune::Waker) {
        self.waker = Some(w);
    }

    fn open(&mut self) {
        if self.tx.is_some() || self.opening.is_some() || rune::tooling::headless_run().is_some() {
            return;
        }
        let (otx, orx) = channel();
        let waker = self.waker.clone();
        let shared = self.shared.clone();
        std::thread::Builder::new()
            .name("adeck-audio".into())
            .spawn(move || {
                // The stream isn't Send, so it lives on this thread for the life of the app.
                match start_stream(shared) {
                    Ok((stream, tx, rate)) => {
                        let _ = otx.send(Ok((tx, rate)));
                        if let Some(w) = &waker {
                            w();
                        }
                        let _keep = stream;
                        loop {
                            std::thread::park();
                        }
                    }
                    Err(e) => {
                        let _ = otx.send(Err(e));
                        if let Some(w) = &waker {
                            w();
                        }
                    }
                }
            })
            .ok();
        self.opening = Some(orx);
    }

    fn send(&mut self, c: Cmd) {
        match &self.tx {
            Some(tx) => {
                let _ = tx.send(c);
            }
            None => {
                self.queued.push(c);
                self.open();
            }
        }
    }

    /// Loads these files for `owner` (a tab id) unless they're loaded already. Decoding runs on a
    /// worker; playing before it lands plays when it does.
    pub fn load(&mut self, owner: u64, paths: Vec<(String, std::path::PathBuf)>) {
        let ids: Vec<String> = paths.iter().map(|(id, _)| id.clone()).collect();
        if self.loaded.as_ref().is_some_and(|(o, l)| *o == owner && *l == ids) {
            return;
        }
        self.open();
        let set = self.next_set;
        self.next_set += 1;
        self.loaded = Some((owner, ids));
        self.want_play = false;
        self.pending_seek = None;
        self.frames = 0;
        self.active = 0;
        self.loop_frames = None;
        self.shared.pos.store(0, Ordering::Release);
        self.shared.playing.store(false, Ordering::Release);
        self.send(Cmd::Pause);
        if self.tx.is_some() {
            self.start_decode(set, paths);
        } else {
            self.pending = Some((set, paths));
        }
    }

    fn start_decode(&mut self, set: u64, paths: Vec<(String, std::path::PathBuf)>) {
        let (tx, rx) = channel();
        let waker = self.waker.clone();
        let rate = self.rate;
        std::thread::spawn(move || {
            let mut tracks = Vec::new();
            let mut longest = 0u64;
            for (_, p) in &paths {
                match decode::decode(p) {
                    Ok(pcm) => {
                        let pcm: Pcm = decode::resample(&pcm, rate);
                        longest = longest.max(pcm.frames() as u64);
                        tracks.push(Arc::new(pcm.samples));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(format!("{}: {e}", p.display())));
                        if let Some(w) = &waker {
                            w();
                        }
                        return;
                    }
                }
            }
            let _ = tx.send(Ok((tracks, longest)));
            if let Some(w) = &waker {
                w();
            }
        });
        self.decoding = Some((set, rx));
    }

    pub fn is_loading(&self) -> bool {
        self.decoding.is_some() || self.pending.is_some()
    }

    pub fn owner(&self) -> Option<u64> {
        self.loaded.as_ref().map(|(o, _)| *o)
    }

    pub fn play(&mut self) {
        self.want_play = true;
        if !self.is_loading() && self.frames > 0 {
            if self.position() >= self.frames.saturating_sub(1) && self.loop_frames.is_none() {
                self.seek_frames(0);
            }
            self.shared.playing.store(true, Ordering::Release);
            self.send(Cmd::Play);
        }
    }

    pub fn pause(&mut self) {
        self.want_play = false;
        self.shared.playing.store(false, Ordering::Release);
        self.send(Cmd::Pause);
    }

    pub fn toggle(&mut self) {
        if self.is_playing() || self.want_play { self.pause() } else { self.play() }
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Acquire)
    }

    pub fn position(&self) -> u64 {
        self.shared.pos.load(Ordering::Acquire)
    }

    pub fn secs(&self) -> f64 {
        if let Some(p) = self.pending_seek {
            return p;
        }
        self.position() as f64 / f64::from(self.rate)
    }

    pub fn total_secs(&self) -> f64 {
        self.frames as f64 / f64::from(self.rate)
    }

    pub fn seek_secs(&mut self, s: f64) {
        // Before the files have decoded there's no length to clamp to: hold the seek.
        if self.frames == 0 {
            self.pending_seek = Some(s.max(0.0));
            return;
        }
        self.seek_frames((s.max(0.0) * f64::from(self.rate)) as u64);
    }

    pub fn seek_frames(&mut self, f: u64) {
        let f = f.min(self.frames.saturating_sub(1));
        self.shared.pos.store(f, Ordering::Release);
        self.send(Cmd::Seek(f));
    }

    pub fn set_active(&mut self, i: usize) {
        self.active = i;
        self.send(Cmd::Active(i));
    }

    /// Linear gains per member.
    pub fn set_trims(&mut self, trims: Vec<f32>) {
        self.send(Cmd::Trims(trims));
    }

    pub fn set_loop_secs(&mut self, l: Option<(f64, f64)>) {
        let r = f64::from(self.rate);
        self.loop_frames = l.map(|(a, b)| ((a.min(b) * r) as u64, (a.max(b) * r) as u64)).filter(|(a, b)| b > a);
        self.send(Cmd::Loop(self.loop_frames));
    }

    /// Drains the worker results. True when something visible changed.
    pub fn tick(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.opening
            && let Ok(r) = rx.try_recv()
        {
            self.opening = None;
            changed = true;
            match r {
                Ok((tx, rate)) => {
                    self.rate = rate;
                    self.tx = Some(tx.clone());
                    for c in self.queued.drain(..) {
                        let _ = tx.send(c);
                    }
                    // Loads asked for before the device's rate was known decode now.
                    if let Some((set, paths)) = self.pending.take() {
                        self.start_decode(set, paths);
                    }
                }
                Err(e) => self.error = Some(format!("Couldn't open the audio output: {e}")),
            }
        }
        if let Some((set, rx)) = &self.decoding
            && let Ok(r) = rx.try_recv()
        {
            let set = *set;
            self.decoding = None;
            changed = true;
            match r {
                Ok((tracks, longest)) => {
                    self.frames = longest;
                    self.error = None;
                    self.send(Cmd::Load { set, tracks });
                    self.send(Cmd::Active(self.active));
                    if let Some(p) = self.pending_seek.take() {
                        self.seek_secs(p);
                    }
                    if self.want_play {
                        self.play();
                    }
                }
                Err(e) => {
                    self.error = Some(format!("Couldn't load: {e}"));
                    self.loaded = None;
                }
            }
        }
        if self.shared.ended.swap(false, Ordering::AcqRel) {
            self.want_play = false;
            changed = true;
        }
        let _ = self.shared.set.load(Ordering::Relaxed);
        changed
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.5);
        let v = self.volume;
        self.send(Cmd::Volume(v));
    }

    /// Forgets what's loaded, so the next `load` reloads.
    pub fn unload(&mut self) {
        self.pause();
        self.loaded = None;
        self.frames = 0;
        self.decoding = None;
        self.pending = None;
    }
}

struct State {
    tracks: Vec<Arc<Vec<f32>>>,
    gains: Vec<f32>,
    trims: Vec<f32>,
    active: usize,
    pos: u64,
    len: u64,
    playing: bool,
    looping: Option<(u64, u64)>,
    step: f32,
    /// Fade in after a seek or start, out before a pause, so nothing clicks.
    master: f32,
    volume: f32,
}

fn start_stream(shared: Arc<Shared>) -> Result<(cpal::Stream, Sender<Cmd>, u32), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let rate = config.sample_rate();
    let channels = config.channels() as usize;
    let (tx, rx) = channel::<Cmd>();
    let st = Mutex::new(State {
        tracks: Vec::new(),
        gains: Vec::new(),
        trims: Vec::new(),
        active: 0,
        pos: 0,
        len: 0,
        playing: false,
        looping: None,
        step: 1.0 / (XFADE * rate as f32),
        master: 0.0,
        volume: 1.0,
    });
    let sh = shared.clone();
    let fill = move |out: &mut [f32]| {
        let Ok(mut s) = st.lock() else {
            out.fill(0.0);
            return;
        };
        while let Ok(c) = rx.try_recv() {
            match c {
                Cmd::Load { set, tracks } => {
                    s.len = tracks.iter().map(|t| (t.len() / 2) as u64).max().unwrap_or(0);
                    s.gains = (0..tracks.len()).map(|i| if i == s.active { 1.0 } else { 0.0 }).collect();
                    s.trims.resize(tracks.len(), 1.0);
                    s.tracks = tracks;
                    s.pos = 0;
                    s.playing = false;
                    sh.set.store(set, Ordering::Release);
                }
                Cmd::Play => s.playing = true,
                Cmd::Pause => s.playing = false,
                Cmd::Seek(f) => {
                    s.pos = f;
                    s.master = 0.0;
                }
                Cmd::Active(i) => s.active = i,
                Cmd::Trims(t) => s.trims = t,
                Cmd::Loop(l) => s.looping = l,
                Cmd::Volume(v) => s.volume = v,
            }
        }
        let n = s.tracks.len();
        let mut loudest = 0f32;
        for frame in out.chunks_mut(channels) {
            let target_master = if s.playing { 1.0 } else { 0.0 };
            let step = s.step;
            s.master += (target_master - s.master).clamp(-step, step);
            if s.master <= 0.0 && !s.playing {
                frame.fill(0.0);
                continue;
            }
            let (mut l, mut r) = (0f32, 0f32);
            let p = s.pos as usize;
            for i in 0..n {
                let target = if i == s.active { 1.0 } else { 0.0 };
                let g = s.gains[i] + (target - s.gains[i]).clamp(-step, step);
                s.gains[i] = g;
                if g <= 0.0 {
                    continue;
                }
                let t = &s.tracks[i];
                let trim = s.trims.get(i).copied().unwrap_or(1.0);
                if let (Some(a), Some(b)) = (t.get(p * 2), t.get(p * 2 + 1)) {
                    l += a * g * trim;
                    r += b * g * trim;
                }
            }
            let m = s.master * s.volume;
            loudest = loudest.max((l * m).abs()).max((r * m).abs());
            frame[0] = l * m;
            if channels > 1 {
                frame[1] = r * m;
                for x in &mut frame[2..] {
                    *x = 0.0;
                }
            }
            if s.playing {
                s.pos += 1;
                if let Some((a, b)) = s.looping
                    && s.pos >= b
                {
                    s.pos = a;
                }
                if s.pos >= s.len {
                    s.pos = s.len.saturating_sub(1);
                    s.playing = false;
                    sh.playing.store(false, Ordering::Release);
                    sh.ended.store(true, Ordering::Release);
                }
            }
        }
        sh.pos.store(s.pos, Ordering::Release);
        sh.level.store(loudest.to_bits(), Ordering::Release);
    };
    let err = |e: cpal::Error| eprintln!("adeck: audio stream error: {e}");
    let cfg: cpal::StreamConfig = config.into();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(cfg, move |out: &mut [f32], _| fill(out), err, None),
        other => {
            return Err(format!("the output wants {other:?} samples; only f32 is supported"));
        }
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, tx, rate))
}

/// The linear trim that brings `lufs` to `target`, capped at +12 dB.
pub fn trim_for(lufs: Option<f64>, target: f64) -> f32 {
    match lufs {
        Some(l) => 10f64.powf(((target - l).min(12.0)) / 20.0) as f32,
        None => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_match_loudness_and_cap_boosts() {
        assert!((trim_for(Some(-20.0), -14.0) - 2.0).abs() < 0.01);
        assert!((trim_for(Some(-8.0), -14.0) - 0.501).abs() < 0.01);
        assert!((trim_for(Some(-60.0), -14.0) - 3.98).abs() < 0.01);
        assert_eq!(trim_for(None, -14.0), 1.0);
    }
}
