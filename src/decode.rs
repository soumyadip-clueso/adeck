//! Any audio file, or the audio track of a video, to interleaved stereo f32.
//!
//! symphonia reads wav, mp3, flac, aiff, ogg and the AAC in mp4/mov. Anything it can't read is
//! handed to ffmpeg once and the wav it writes is cached.

use std::fs::File;
use std::path::{Path, PathBuf};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymErr;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Interleaved stereo at `rate`.
#[derive(Clone, Debug, Default)]
pub struct Pcm {
    pub rate: u32,
    /// Channels the file had (mono files are spread to both sides).
    pub source_channels: u16,
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }
    pub fn secs(&self) -> f64 {
        self.frames() as f64 / f64::from(self.rate.max(1))
    }
    pub fn mono(&self) -> Vec<f32> {
        self.samples.as_chunks::<2>().0.iter().map(|f| (f[0] + f[1]) * 0.5).collect()
    }
}

pub fn decode(path: &Path) -> Result<Pcm, String> {
    match decode_symphonia(path) {
        Ok(p) if p.frames() > 0 => Ok(p),
        first => {
            let why = first.err().unwrap_or_else(|| "no samples".into());
            let wav = ffmpeg_wav(path).map_err(|e| format!("{why}; ffmpeg fallback: {e}"))?;
            decode_symphonia(&wav)
        }
    }
}

fn decode_symphonia(path: &Path) -> Result<Pcm, String> {
    let file = File::open(path).map_err(|e| format!("couldn't open {}: {e}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
        .map_err(|e| format!("unreadable: {e}"))?;
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some()).ok_or("no audio track")?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| format!("no decoder: {e}"))?;
    let mut out = Vec::new();
    let mut channels = track.codec_params.channels.map_or(2, |c| c.count() as u16);
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymErr::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymErr::ResetRequired) => break,
            Err(e) => {
                return if out.is_empty() { Err(format!("read failed: {e}")) } else { Ok(stereo(out, channels, rate)) };
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymErr::DecodeError(_)) => continue,
            Err(e) => return Err(format!("decode failed: {e}")),
        };
        let spec = *decoded.spec();
        channels = spec.channels.count() as u16;
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if (b.capacity()) < decoded.capacity() * spec.channels.count() {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        out.extend_from_slice(b.samples());
    }
    Ok(stereo(out, channels, rate))
}

fn stereo(interleaved: Vec<f32>, channels: u16, rate: u32) -> Pcm {
    let samples = match channels {
        2 => interleaved,
        0 | 1 => interleaved.iter().flat_map(|&s| [s, s]).collect(),
        n => interleaved.chunks_exact(n as usize).flat_map(|f| [f[0], f[1]]).collect(),
    };
    Pcm { rate, source_channels: channels.max(1), samples }
}

/// ffmpeg's wav of a file symphonia couldn't read, cached by path.
fn ffmpeg_wav(path: &Path) -> Result<PathBuf, String> {
    let out = crate::derive::cache_for(path).join("ffmpeg.wav");
    if out.is_file() {
        return Ok(out);
    }
    std::fs::create_dir_all(out.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
    let st = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "2", "-c:a", "pcm_f32le"])
        .arg(&out)
        .status()
        .map_err(|e| format!("couldn't run ffmpeg: {e}"))?;
    if st.success() { Ok(out) } else { Err(format!("ffmpeg exited with {st}")) }
}

/// Stereo `pcm` at `to` Hz. Same rate: unchanged.
pub fn resample(pcm: &Pcm, to: u32) -> Pcm {
    use rubato::{FftFixedIn, Resampler};
    if pcm.rate == to || pcm.frames() == 0 {
        return pcm.clone();
    }
    let frames = pcm.frames();
    let chunk = 1024;
    let Ok(mut rs) = FftFixedIn::<f32>::new(pcm.rate as usize, to as usize, chunk, 2, 2) else {
        return pcm.clone();
    };
    let (l, r): (Vec<f32>, Vec<f32>) = pcm.samples.as_chunks::<2>().0.iter().map(|f| (f[0], f[1])).unzip();
    let mut out_l = Vec::with_capacity(frames * to as usize / pcm.rate as usize + chunk);
    let mut out_r = Vec::with_capacity(out_l.capacity());
    let mut i = 0;
    while i < frames {
        let end = (i + chunk).min(frames);
        let mut a = l[i..end].to_vec();
        let mut b = r[i..end].to_vec();
        a.resize(chunk, 0.0);
        b.resize(chunk, 0.0);
        if let Ok(o) = rs.process(&[a, b], None) {
            out_l.extend_from_slice(&o[0]);
            out_r.extend_from_slice(&o[1]);
        }
        i = end;
    }
    let want = (frames as u64 * u64::from(to) / u64::from(pcm.rate)) as usize;
    // The resampler's delay: drop its lead-in so the two clocks line up.
    let delay = rs.output_delay();
    let samples: Vec<f32> = (delay..(delay + want).min(out_l.len())).flat_map(|k| [out_l[k], out_r[k]]).collect();
    Pcm { rate: to, source_channels: pcm.source_channels, samples }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A 16-bit stereo wav of `secs` of a sine at `hz`, for tests and fixtures.
    pub fn write_sine(path: &Path, hz: f32, secs: f32, rate: u32, amp: f32) {
        let n = (secs * rate as f32) as usize;
        let mut data = Vec::with_capacity(n * 4);
        for i in 0..n {
            let s = ((i as f32 / rate as f32 * hz * std::f32::consts::TAU).sin() * amp * 32767.0) as i16;
            data.extend_from_slice(&s.to_le_bytes());
            data.extend_from_slice(&s.to_le_bytes());
        }
        let mut w = Vec::new();
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
        std::fs::write(path, w).unwrap();
    }

    #[test]
    fn decodes_and_resamples_a_wav() {
        let p = std::env::temp_dir().join(format!("adeck-dec-{}.wav", std::process::id()));
        write_sine(&p, 440.0, 1.0, 44_100, 0.5);
        let pcm = decode(&p).unwrap();
        assert_eq!(pcm.rate, 44_100);
        assert_eq!(pcm.frames(), 44_100);
        let up = resample(&pcm, 48_000);
        assert!((up.frames() as i64 - 48_000).abs() < 64, "{}", up.frames());
        let _ = std::fs::remove_file(&p);
    }
}
