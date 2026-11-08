//! What adeck works out about a file, once, cached by its path, size and modification time in
//! `~/Library/Caches/adeck/<hash>/`.
//!
//! Native (fast, always): length, a min/max/RMS envelope for the waveform, a log-frequency
//! spectrogram, integrated loudness (ITU-R BS.1770, gated), sample peak, and for music the tempo,
//! beat grid and key (rbxport's analyser). earshot (slow, on request): `describe`, `measure`,
//! `critique` and `sheet`, each run once and its JSON or PNG kept.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::decode::{self, Pcm};

/// Columns in the stored envelope; the waveform is drawn from it at any width.
pub const ENVELOPE_COLS: usize = 4000;
pub const SPEC_ROWS: usize = 256;
/// Bump when the analysis changes, so old caches are redone.
const VERSION: u32 = 3;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Analysis {
    pub version: u32,
    pub secs: f64,
    pub rate: u32,
    pub channels: u16,
    /// Per column: min, max, RMS (mono, -1 to 1).
    pub envelope: Vec<[f32; 3]>,
    pub lufs: Option<f64>,
    pub peak_dbfs: f64,
    pub bpm: Option<f64>,
    pub bpm_confidence: Option<f64>,
    /// Beat times in seconds, and whether each is a downbeat.
    pub beats: Vec<(f64, bool)>,
    pub key: Option<String>,
    pub spec_w: u32,
    pub spec_h: u32,
}

impl Analysis {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "duration_s": round(self.secs, 3),
            "sample_rate": self.rate,
            "channels": self.channels,
            "lufs": self.lufs.map(|v| round(v, 2)),
            "peak_dbfs": round(self.peak_dbfs, 2),
            "bpm": self.bpm.map(|v| round(v, 2)),
            "bpm_confidence": self.bpm_confidence.map(|v| round(v, 2)),
            "beats": self.beats.len(),
            "key": self.key,
        })
    }
}

fn round(v: f64, places: i32) -> f64 {
    let m = 10f64.powi(places);
    (v * m).round() / m
}

/// The cache folder for one file version.
pub fn cache_for(path: &Path) -> PathBuf {
    let mut h = DefaultHasher::new();
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canon.hash(&mut h);
    if let Ok(m) = std::fs::metadata(&canon) {
        m.len().hash(&mut h);
        m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).hash(&mut h);
    }
    crate::config::cache_dir().join(format!("{:016x}", h.finish()))
}

/// The analysis and the spectrogram's RGBA, from the cache or worked out now.
pub fn analyse(path: &Path) -> Result<(Analysis, Vec<u8>), String> {
    let dir = cache_for(path);
    if let (Ok(a), Ok(spec)) = (std::fs::read(dir.join("analysis.json")), std::fs::read(dir.join("spec.rgba")))
        && let Ok(a) = serde_json::from_slice::<Analysis>(&a)
        && a.version == VERSION
        && spec.len() == (a.spec_w * a.spec_h * 4) as usize
    {
        return Ok((a, spec));
    }
    let pcm = decode::decode(path)?;
    let (a, spec) = analyse_pcm(&pcm);
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(j) = serde_json::to_vec(&a) {
        let _ = std::fs::write(dir.join("analysis.json"), j);
        let _ = std::fs::write(dir.join("spec.rgba"), &spec);
    }
    Ok((a, spec))
}

pub fn analyse_pcm(pcm: &Pcm) -> (Analysis, Vec<u8>) {
    let mono = pcm.mono();
    let secs = pcm.secs();
    let peak = pcm.samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    let mut a = Analysis {
        version: VERSION,
        secs,
        rate: pcm.rate,
        channels: pcm.source_channels,
        envelope: envelope(&mono, ENVELOPE_COLS),
        lufs: loudness(pcm),
        peak_dbfs: if peak > 0.0 { 20.0 * f64::from(peak).log10() } else { -120.0 },
        ..Analysis::default()
    };
    // A beat grid means nothing on a one-shot effect.
    if secs >= 6.0 {
        let options = rbl_analysis::AnalysisPreset::Rbxport.options();
        let head = &mono[..mono.len().min(pcm.rate as usize * 600)];
        let an = rbl_analysis::analyse_with(head, pcm.rate, options);
        if an.tempo.bpm > 0.0 {
            a.bpm = Some(an.tempo.bpm);
            a.bpm_confidence = Some(an.tempo.confidence);
            a.beats = an.tempo.beats.iter().map(|b| (f64::from(b.time_ms) / 1000.0, b.beat_number == 1)).collect();
        }
        a.key = an.key.map(|k| format!("{} ({})", k.name, k.camelot()));
    }
    let (w, h, spec) = spectrogram(&mono, pcm.rate);
    a.spec_w = w;
    a.spec_h = h;
    (a, spec)
}

/// Min, max and RMS of `cols` equal slices.
pub fn envelope(mono: &[f32], cols: usize) -> Vec<[f32; 3]> {
    if mono.is_empty() {
        return vec![[0.0; 3]; cols];
    }
    (0..cols)
        .map(|i| {
            let a = i * mono.len() / cols;
            let b = ((i + 1) * mono.len() / cols).max(a + 1).min(mono.len());
            let s = &mono[a..b];
            let (mut lo, mut hi, mut sq) = (0f32, 0f32, 0f32);
            for &x in s {
                lo = lo.min(x);
                hi = hi.max(x);
                sq += x * x;
            }
            [lo, hi, (sq / s.len() as f32).sqrt()]
        })
        .collect()
}

/// Integrated loudness, ITU-R BS.1770-4: K-weighting, 400 ms blocks with 75% overlap, the
/// absolute gate at -70 LUFS and the relative gate 10 LU under the gated mean.
pub fn loudness(pcm: &Pcm) -> Option<f64> {
    let fs = f64::from(pcm.rate);
    if pcm.frames() == 0 {
        return None;
    }
    let shelf = Biquad::high_shelf(fs, 1_681.974_450_955_532, 3.999_843_853_973_347, 0.707_175_236_955_419_3);
    let hp = Biquad::high_pass(fs, 38.135_470_876_139_82, 0.500_327_037_325_395_3);
    let mut z = Vec::with_capacity(pcm.frames());
    let (mut s0, mut s1, mut h0, mut h1) = (shelf, shelf, hp, hp);
    for f in pcm.samples.as_chunks::<2>().0 {
        let l = h0.run(s0.run(f64::from(f[0])));
        let r = h1.run(s1.run(f64::from(f[1])));
        z.push(l * l + r * r);
    }
    let block = (0.4 * fs) as usize;
    let step = (0.1 * fs).max(1.0) as usize;
    let mut powers = Vec::new();
    if z.len() < block {
        powers.push(z.iter().sum::<f64>() / z.len() as f64);
    } else {
        let mut start = 0;
        while start + block <= z.len() {
            powers.push(z[start..start + block].iter().sum::<f64>() / block as f64);
            start += step;
        }
    }
    let lufs = |p: f64| -0.691 + 10.0 * p.log10();
    let abs: Vec<f64> = powers.into_iter().filter(|&p| p > 0.0 && lufs(p) > -70.0).collect();
    if abs.is_empty() {
        return None;
    }
    let rel_gate = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let rel: Vec<f64> = abs.into_iter().filter(|&p| lufs(p) > rel_gate).collect();
    if rel.is_empty() {
        return None;
    }
    Some(lufs(rel.iter().sum::<f64>() / rel.len() as f64))
}

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn new(b: [f64; 3], a0: f64, a: [f64; 2]) -> Self {
        Biquad { b: [b[0] / a0, b[1] / a0, b[2] / a0], a: [a[0] / a0, a[1] / a0], z: [0.0; 2] }
    }
    fn high_shelf(fs: f64, fc: f64, gain_db: f64, q: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w = std::f64::consts::TAU * fc / fs;
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q);
        let sa = 2.0 * a.sqrt() * alpha;
        Biquad::new(
            [a * ((a + 1.0) + (a - 1.0) * cs + sa), -2.0 * a * ((a - 1.0) + (a + 1.0) * cs), a * ((a + 1.0) + (a - 1.0) * cs - sa)],
            (a + 1.0) - (a - 1.0) * cs + sa,
            [2.0 * ((a - 1.0) - (a + 1.0) * cs), (a + 1.0) - (a - 1.0) * cs - sa],
        )
    }
    fn high_pass(fs: f64, fc: f64, q: f64) -> Self {
        let w = std::f64::consts::TAU * fc / fs;
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q);
        Biquad::new([(1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0], 1.0 + alpha, [-2.0 * cs, 1.0 - alpha])
    }
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// A log-frequency spectrogram, 30 Hz to Nyquist, in magma-like colours, 90 dB under the file's
/// loudest bin. Returns width, height and RGBA (low frequencies at the bottom).
pub fn spectrogram(mono: &[f32], rate: u32) -> (u32, u32, Vec<u8>) {
    let secs = mono.len() as f64 / f64::from(rate.max(1));
    let cols = ((secs * 120.0) as usize).clamp(240, 3000);
    let n = if secs < 2.0 { 1024 } else { 2048 };
    let rows = SPEC_ROWS;
    let mut planner = realfft::RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let window: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
    let bins = n / 2 + 1;
    let nyq = rate as f32 / 2.0;
    let lo = 30f32.min(nyq / 4.0);
    // Each row spans a band of bins, so high rows average and low rows interpolate.
    let edges: Vec<f32> = (0..=rows).map(|r| lo * (nyq / lo).powf(r as f32 / rows as f32) / nyq * (bins - 1) as f32).collect();
    let mut db = vec![-200f32; cols * rows];
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();
    let mut mag = vec![0f32; bins];
    let hop = if mono.len() > n { (mono.len() - n) as f64 / (cols - 1).max(1) as f64 } else { 0.0 };
    for c in 0..cols {
        let start = (c as f64 * hop) as usize;
        for i in 0..n {
            input[i] = mono.get(start + i).copied().unwrap_or(0.0) * window[i];
        }
        if fft.process(&mut input, &mut output).is_err() {
            continue;
        }
        for (m, o) in mag.iter_mut().zip(&output) {
            *m = o.norm();
        }
        for r in 0..rows {
            let (a, b) = (edges[r], edges[r + 1]);
            let v = if b - a < 1.0 {
                let i = a.floor() as usize;
                let t = a - i as f32;
                mag[i] * (1.0 - t) + mag.get(i + 1).copied().unwrap_or(0.0) * t
            } else {
                let (i, j) = (a.floor() as usize, (b.ceil() as usize).min(bins));
                mag[i..j].iter().copied().fold(0.0, f32::max)
            };
            db[(rows - 1 - r) * cols + c] = 20.0 * (v.max(1e-9) / (n as f32 / 4.0)).log10();
        }
    }
    let top = db.iter().copied().fold(-200f32, f32::max);
    let floor = top - 90.0;
    let mut rgba = Vec::with_capacity(cols * rows * 4);
    for v in db {
        let t = ((v - floor) / 90.0).clamp(0.0, 1.0);
        let [r, g, b] = magma(t);
        rgba.extend_from_slice(&[r, g, b, 255]);
    }
    (cols as u32, rows as u32, rgba)
}

/// An approximation of matplotlib's magma.
fn magma(t: f32) -> [u8; 3] {
    const STOPS: [[f32; 3]; 6] = [[0.0, 0.0, 4.0], [40.0, 11.0, 84.0], [101.0, 21.0, 110.0], [183.0, 55.0, 121.0], [251.0, 135.0, 97.0], [252.0, 253.0, 191.0]];
    let x = t * (STOPS.len() - 1) as f32;
    let i = (x.floor() as usize).min(STOPS.len() - 2);
    let f = x - i as f32;
    let mix = |k: usize| (STOPS[i][k] + (STOPS[i + 1][k] - STOPS[i][k]) * f) as u8;
    [mix(0), mix(1), mix(2)]
}

// earshot: each command once per file version, its JSON kept beside the analysis.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Earshot {
    Describe,
    Measure,
    Critique,
    Timeline,
}

impl Earshot {
    pub fn parse(s: &str) -> Option<Earshot> {
        match s {
            "describe" => Some(Earshot::Describe),
            "measure" => Some(Earshot::Measure),
            "critique" => Some(Earshot::Critique),
            "timeline" => Some(Earshot::Timeline),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Earshot::Describe => "describe",
            Earshot::Measure => "measure",
            Earshot::Critique => "critique",
            Earshot::Timeline => "timeline",
        }
    }
    fn args(self) -> &'static [&'static str] {
        match self {
            Earshot::Describe => &["describe", "--json"],
            Earshot::Measure => &["measure", "--json", "--light"],
            Earshot::Critique => &["critique", "--json", "--fast"],
            Earshot::Timeline => &["timeline", "--json", "--detail", "low"],
        }
    }
}

/// The cached result, if earshot has run on this file before.
pub fn earshot_cached(path: &Path, cmd: Earshot) -> Option<Value> {
    let f = cache_for(path).join(format!("earshot-{}.json", cmd.name()));
    serde_json::from_slice(&std::fs::read(f).ok()?).ok()
}

/// Runs earshot (or returns the cached result). Slow: call off the UI thread.
pub fn earshot(bin: &str, path: &Path, cmd: Earshot) -> Result<Value, String> {
    if let Some(v) = earshot_cached(path, cmd) {
        return Ok(v);
    }
    let dir = cache_for(path);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out_file = dir.join(format!("earshot-{}.json", cmd.name()));
    let tmp = dir.join(format!("earshot-{}.tmp.json", cmd.name()));
    let out = Command::new(bin).args(cmd.args()).arg("--out").arg(&tmp).arg(path).output().map_err(|e| format!("couldn't run {bin}: {e}"))?;
    let parsed = std::fs::read(&tmp).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).or_else(|| {
        let s = String::from_utf8_lossy(&out.stdout);
        s.find('{').and_then(|i| serde_json::from_str(&s[i..]).ok())
    });
    let _ = std::fs::remove_file(&tmp);
    match parsed {
        Some(v) if out.status.success() => {
            let _ = std::fs::write(&out_file, serde_json::to_vec(&v).unwrap_or_default());
            Ok(v)
        }
        _ => {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(format!("earshot {} failed: {}", cmd.name(), err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim()))
        }
    }
}

/// `earshot sheet`'s PNG for a file, made once.
pub fn sheet(bin: &str, path: &Path) -> Result<PathBuf, String> {
    let png = cache_for(path).join("sheet.png");
    if png.is_file() {
        return Ok(png);
    }
    std::fs::create_dir_all(png.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let out = Command::new(bin).args(["sheet", "--width", "2000", "-o"]).arg(&png).arg(path).output().map_err(|e| format!("couldn't run {bin}: {e}"))?;
    if out.status.success() && png.is_file() {
        Ok(png)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("earshot sheet failed: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim()))
    }
}

/// A PNG file as RGBA, for `Pixels`.
pub fn read_png(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::open(path).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    Some((w, h, img.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, secs: f32, amp: f32) -> Pcm {
        let n = (rate as f32 * secs) as usize;
        let samples = (0..n).flat_map(|i| {
            let s = (i as f32 / rate as f32 * 1000.0 * std::f32::consts::TAU).sin() * amp;
            [s, s]
        });
        Pcm { rate, source_channels: 2, samples: samples.collect() }
    }

    #[test]
    fn a_full_scale_1k_sine_reads_about_minus_3_lufs() {
        // BS.1770: a 0 dBFS 997 Hz sine in both channels reads +0.0 LUFS per channel summed,
        // i.e. about -3.01 + 3.01; with both channels at full scale it's ~0 LUFS. At -6 dBFS: ~-6.
        let l = loudness(&sine(48_000, 3.0, 0.5)).unwrap();
        assert!((l - -6.0).abs() < 0.7, "{l}");
    }

    #[test]
    fn spectrogram_puts_a_tone_in_one_band() {
        let p = sine(48_000, 1.0, 0.5);
        let (w, h, rgba) = spectrogram(&p.mono(), p.rate);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // The brightest row of the middle column sits near 1 kHz on the log axis.
        let c = w as usize / 2;
        let bright = (0..h as usize).max_by_key(|r| rgba[(r * w as usize + c) * 4] as u32 + rgba[(r * w as usize + c) * 4 + 1] as u32).unwrap();
        let row_from_bottom = h as usize - 1 - bright;
        let hz = 30f32 * (24_000f32 / 30.0).powf(row_from_bottom as f32 / h as f32);
        assert!((700.0..1500.0).contains(&hz), "{hz}");
    }
}
