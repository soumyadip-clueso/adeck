//! A picture on the mixer's clock: the frame showing is always the one at the mixer's position,
//! so takes flipped mid-play land on the same frame, and 2× runs the picture at 2×.
//!
//! ffmpeg decodes. A worker runs it from a frame boundary (`-ss` on `idx / fps` lands exactly on
//! frame `idx`, even mid-GOP) and pipes RGBA frames into a short queue; each UI frame takes the
//! one for the mixer's time and asks for a fresh seek only when the time jumped back or far
//! ahead. Exports often put keyframes seconds apart, which makes every seek decode hundreds of
//! frames, so a proxy (at most 1280×720, a keyframe every 8 frames, constant rate from 0) is
//! cached beside the analysis on first open and used once it's ready.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};

const MAX_W: u32 = 1280;
const MAX_H: u32 = 720;
/// Frames decoded ahead of the one showing.
const AHEAD: usize = 10;
/// A jump forward of more than this many frames re-seeks; less, and the decoder runs on to it.
const RUN_ON: u64 = 36;
/// Bump to rebuild every cached proxy.
const PROXY_VERSION: u32 = 1;

/// ffmpeg or ffprobe: Homebrew's when the app was opened from the Dock (no shell PATH), else
/// whatever the PATH finds.
pub fn tool(name: &str) -> PathBuf {
    ["/opt/homebrew/bin", "/usr/local/bin"].iter().map(|d| Path::new(d).join(name)).find(|p| p.is_file()).unwrap_or_else(|| name.into())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    /// Frame rate as a fraction, so 30000/1001 stays exact.
    pub rate: (u64, u64),
    pub frames: u64,
    pub secs: f64,
    pub audio: bool,
    /// The size frames are decoded at: the picture fitted inside 1280×720, even sides.
    pub out: (u32, u32),
}

impl Info {
    pub fn fps(&self) -> f64 {
        self.rate.0 as f64 / self.rate.1.max(1) as f64
    }
    /// The frame showing at `secs`.
    pub fn frame_at(&self, secs: f64) -> u64 {
        let f = (secs.max(0.0) * self.fps() + 1e-6).floor() as u64;
        f.min(self.frames.saturating_sub(1))
    }
    pub fn secs_of(&self, frame: u64) -> f64 {
        frame as f64 * self.rate.1 as f64 / self.rate.0.max(1) as f64
    }
}

fn fit(w: u32, h: u32) -> (u32, u32) {
    let k = (f64::from(MAX_W) / f64::from(w.max(1))).min(f64::from(MAX_H) / f64::from(h.max(1))).min(1.0);
    let even = |x: f64| ((x.round() as u32) / 2 * 2).max(2);
    (even(f64::from(w) * k), even(f64::from(h) * k))
}

fn ratio(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.split_once('/').unwrap_or((s, "1"));
    let (a, b) = (a.trim().parse::<u64>().ok()?, b.trim().parse::<u64>().ok()?);
    (a > 0 && b > 0).then_some((a, b))
}

pub fn probe(path: &Path) -> Result<Info, String> {
    let out = Command::new(tool("ffprobe"))
        .args(["-v", "error", "-show_entries", "stream=codec_type,width,height,r_frame_rate,avg_frame_rate,nb_frames,duration:format=duration", "-of", "json"])
        .arg(path)
        .output()
        .map_err(|e| format!("couldn't run ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("ffprobe failed").to_owned());
    }
    parse_probe(&String::from_utf8_lossy(&out.stdout))
}

fn parse_probe(json: &str) -> Result<Info, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let vs = streams.iter().find(|s| s["codec_type"] == "video").ok_or("no video stream")?;
    let audio = streams.iter().any(|s| s["codec_type"] == "audio");
    let (width, height) = (vs["width"].as_u64().unwrap_or(0) as u32, vs["height"].as_u64().unwrap_or(0) as u32);
    if width == 0 || height == 0 {
        return Err("the video has no size".into());
    }
    // The average rate is what the frames actually come at; r_frame_rate can be a timebase.
    let rate =
        [&vs["avg_frame_rate"], &vs["r_frame_rate"]].iter().filter_map(|r| r.as_str().and_then(ratio)).find(|(a, b)| (*a as f64 / *b as f64) < 241.0).ok_or("no frame rate")?;
    let num = |x: &serde_json::Value| x.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| x.as_f64());
    let secs = num(&vs["duration"]).or_else(|| num(&v["format"]["duration"])).unwrap_or(0.0);
    let frames = vs["nb_frames"].as_str().and_then(|s| s.parse::<u64>().ok()).filter(|n| *n > 0).unwrap_or_else(|| (secs * rate.0 as f64 / rate.1 as f64).round() as u64).max(1);
    Ok(Info { width, height, rate, frames, secs, audio, out: fit(width, height) })
}

/// Where `src`'s proxy lives (it may not exist yet).
pub fn proxy_path(src: &Path, info: &Info) -> PathBuf {
    crate::derive::cache_for(src).join(format!("proxy-v{PROXY_VERSION}-{}x{}.mp4", info.out.0, info.out.1))
}

struct St {
    info: Option<Info>,
    error: Option<String>,
    /// The file frames come from: the original, then the proxy once it's built.
    src: Option<PathBuf>,
    /// A seek the UI asked for, by frame.
    seek: Option<u64>,
    /// Decoded frames waiting, in order.
    frames: VecDeque<(u64, Vec<u8>)>,
    /// The frame the decoder emits next; `None` while idle (never started, or at the end).
    next: Option<u64>,
    /// The UI is waiting on a frame: wake it when one lands.
    hungry: bool,
    quit: bool,
    proxy: ProxyState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyState {
    /// Not built and not being built (headless runs only use one that's there).
    None,
    Building,
    Ready,
    Failed,
}

struct Inner {
    st: Mutex<St>,
    cv: Condvar,
    waker: Option<rune::Waker>,
}

impl Inner {
    fn wake(&self) {
        if let Some(w) = &self.waker {
            w();
        }
    }
}

/// One video, decoded on a worker into `pixels`. Dropping it stops the worker and its ffmpeg.
pub struct Picture {
    /// The file as given, and where it was found.
    pub id: String,
    pub path: PathBuf,
    pub pixels: rune::Pixels,
    /// The frame in `pixels`, and the one the last `show` asked for (they differ only while a
    /// seek or the decoder catches up).
    pub shown: Option<u64>,
    pub wanted: Option<u64>,
    inner: Arc<Inner>,
}

impl Drop for Picture {
    fn drop(&mut self) {
        if let Ok(mut st) = self.inner.st.lock() {
            st.quit = true;
        }
        self.inner.cv.notify_all();
    }
}

impl Picture {
    /// Probes and starts decoding `path` on a worker; `build_proxy` also caches a proxy.
    pub fn open(id: String, path: PathBuf, waker: Option<rune::Waker>, build_proxy: bool) -> Picture {
        let inner = Arc::new(Inner {
            st: Mutex::new(St { info: None, error: None, src: None, seek: None, frames: VecDeque::new(), next: None, hungry: false, quit: false, proxy: ProxyState::None }),
            cv: Condvar::new(),
            waker,
        });
        let (w, p) = (inner.clone(), path.clone());
        std::thread::Builder::new().name("adeck-video".into()).spawn(move || worker(&w, &p, build_proxy)).ok();
        Picture { id, path, pixels: rune::Pixels::new(), shown: None, wanted: None, inner }
    }

    pub fn info(&self) -> Option<Info> {
        self.inner.st.lock().ok()?.info.clone()
    }

    pub fn error(&self) -> Option<String> {
        self.inner.st.lock().ok()?.error.clone()
    }

    pub fn proxy(&self) -> ProxyState {
        self.inner.st.lock().map_or(ProxyState::None, |s| s.proxy)
    }

    /// Puts the frame for `secs` in `pixels` if it's decoded, asking for it if not. True when
    /// the picture changed.
    pub fn show(&mut self, secs: f64) -> bool {
        let Ok(mut st) = self.inner.st.lock() else { return false };
        let Some(info) = st.info.clone() else { return false };
        let want = info.frame_at(secs);
        self.wanted = Some(want);
        let mut best = None;
        while st.frames.front().is_some_and(|f| f.0 <= want) {
            best = st.frames.pop_front();
        }
        if !st.frames.is_empty() || best.is_some() {
            self.inner.cv.notify_all();
        }
        let have = best.as_ref().map(|b| b.0).or(self.shown);
        if have != Some(want) {
            let ahead = st.frames.front().map(|f| f.0);
            let reseek = match (ahead, st.next) {
                // Went back (or the queue is past it, which a constant rate never does).
                (Some(f), _) if f > want => best.is_none(),
                (Some(_), _) => false,
                (None, None) => true,
                (None, Some(n)) => want < n || want >= n + RUN_ON,
            };
            if reseek && st.seek != Some(want) {
                st.seek = Some(want);
                self.inner.cv.notify_all();
            }
            st.hungry = true;
        }
        drop(st);
        match best {
            Some((idx, rgba)) if Some(idx) != self.shown => {
                self.pixels.set(info.out.0, info.out.1, rgba);
                self.shown = Some(idx);
                true
            }
            _ => false,
        }
    }

    /// `show`, waiting up to `timeout` for the exact frame: for headless shots.
    pub fn show_now(&mut self, secs: f64, timeout: std::time::Duration) {
        let end = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < end {
            self.show(secs);
            if self.error().is_some() || self.info().is_some_and(|i| self.shown == Some(i.frame_at(secs))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

fn worker(inner: &Arc<Inner>, path: &Path, build_proxy: bool) {
    let info = match probe(path) {
        Ok(i) => i,
        Err(e) => {
            if let Ok(mut st) = inner.st.lock() {
                st.error = Some(format!("Couldn't read the video: {e}"));
            }
            inner.wake();
            return;
        }
    };
    let proxy = proxy_path(path, &info);
    let ready = proxy.is_file();
    if let Ok(mut st) = inner.st.lock() {
        st.src = Some(if ready { proxy.clone() } else { path.to_owned() });
        st.proxy = if ready {
            ProxyState::Ready
        } else if build_proxy {
            ProxyState::Building
        } else {
            ProxyState::None
        };
        st.info = Some(info.clone());
    }
    inner.wake();
    if !ready && build_proxy {
        let (i2, p2, src, info2) = (inner.clone(), proxy, path.to_owned(), info.clone());
        std::thread::Builder::new().name("adeck-proxy".into()).spawn(move || make_proxy(&i2, &src, &p2, &info2)).ok();
    }
    let frame_bytes = info.out.0 as usize * info.out.1 as usize * 4;
    loop {
        // Wait for a seek.
        let (start, src) = {
            let Ok(mut st) = inner.st.lock() else { return };
            loop {
                if st.quit {
                    return;
                }
                if let Some(s) = st.seek.take() {
                    st.frames.clear();
                    st.next = Some(s);
                    break (s, st.src.clone().unwrap_or_else(|| path.to_owned()));
                }
                st = match inner.cv.wait(st) {
                    Ok(s) => s,
                    Err(_) => return,
                };
            }
        };
        let mut child = match spawn_decoder(&src, start, &info) {
            Ok(c) => c,
            Err(e) => {
                if let Ok(mut st) = inner.st.lock() {
                    st.error = Some(e);
                    st.next = None;
                }
                inner.wake();
                continue;
            }
        };
        let Some(mut out) = child.stdout.take() else { continue };
        let mut idx = start;
        loop {
            let mut buf = vec![0u8; frame_bytes];
            if out.read_exact(&mut buf).is_err() {
                // The end, or killed: idle until the next seek.
                if let Ok(mut st) = inner.st.lock()
                    && st.next == Some(idx)
                {
                    st.next = None;
                }
                break;
            }
            let Ok(mut st) = inner.st.lock() else { break };
            // A newer seek: the first frame of each still shows (a scrub sees every seek that
            // finishes), the rest are dropped.
            if st.quit || (st.seek.is_some() && idx != start) {
                break;
            }
            st.frames.push_back((idx, buf));
            idx += 1;
            st.next = Some(idx);
            if std::mem::take(&mut st.hungry) {
                inner.wake();
            }
            while st.frames.len() >= AHEAD && st.seek.is_none() && !st.quit {
                st = match inner.cv.wait(st) {
                    Ok(s) => s,
                    Err(_) => return,
                };
            }
            if st.quit || st.seek.is_some() {
                break;
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn spawn_decoder(src: &Path, start: u64, info: &Info) -> Result<Child, String> {
    let (w, h) = info.out;
    Command::new(tool("ffmpeg"))
        .args(["-v", "error", "-nostdin", "-ss", &format!("{:.6}", info.secs_of(start)), "-i"])
        .arg(src)
        .args(["-an", "-sn", "-dn", "-vf", &format!("scale={w}:{h}:flags=bilinear"), "-fps_mode", "cfr", "-r", &format!("{}/{}", info.rate.0, info.rate.1)])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't run ffmpeg: {e}"))
}

/// Encodes the proxy beside the analysis cache, for the decoder's next seek.
fn make_proxy(inner: &Arc<Inner>, src: &Path, to: &Path, info: &Info) {
    let tmp = to.with_extension("part.mp4");
    if let Some(dir) = to.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let (w, h) = info.out;
    let child = Command::new(tool("ffmpeg"))
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(src)
        .args(["-an", "-sn", "-dn", "-vf", &format!("scale={w}:{h}:flags=bicubic"), "-fps_mode", "cfr", "-r", &format!("{}/{}", info.rate.0, info.rate.1)])
        .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-g", "8", "-bf", "0", "-pix_fmt", "yuv420p", "-f", "mp4"])
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let ok = match child {
        Ok(mut c) => loop {
            if inner.st.lock().map_or(true, |s| s.quit) {
                let _ = c.kill();
                let _ = c.wait();
                break false;
            }
            match c.try_wait() {
                Ok(Some(st)) => break st.success(),
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(_) => break false,
            }
        },
        Err(_) => false,
    };
    let ok = ok && std::fs::rename(&tmp, to).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
    let Ok(mut st) = inner.st.lock() else { return };
    st.proxy = if ok { ProxyState::Ready } else { ProxyState::Failed };
    // The next seek (a scrub, a loop, a jump) reads the proxy; playing on doesn't stall for it.
    if ok {
        st.src = Some(to.to_owned());
    }
    drop(st);
    inner.cv.notify_all();
    inner.wake();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_output_gives_rate_frames_and_a_fitted_size() {
        let i = parse_probe(
            r#"{"streams":[{"codec_type":"video","width":1920,"height":1080,"r_frame_rate":"30/1","avg_frame_rate":"30/1","nb_frames":"3896","duration":"129.866667"},{"codec_type":"audio"}],"format":{"duration":"129.9"}}"#,
        )
        .unwrap();
        assert_eq!((i.rate, i.frames, i.out, i.audio), ((30, 1), 3896, (1280, 720), true));
        let small = parse_probe(r#"{"streams":[{"codec_type":"video","width":960,"height":540,"avg_frame_rate":"12/1","duration":"60.75"}],"format":{}}"#).unwrap();
        assert_eq!((small.frames, small.out, small.audio), (729, (960, 540), false));
        let tall = parse_probe(r#"{"streams":[{"codec_type":"video","width":1080,"height":1920,"avg_frame_rate":"30000/1001","duration":"2"}],"format":{}}"#).unwrap();
        assert_eq!(tall.out, (404, 720));
    }

    /// The decoder's seek lands on exactly frame `idx`, mid-GOP too: its first frame is the
    /// sequential decode's frame `idx`.
    #[test]
    fn a_seek_starts_on_exactly_the_frame_asked_for() {
        let Some(film) = crate::scenes::fixture_video() else { return };
        let info = probe(&film).unwrap();
        assert!(info.audio);
        let n = info.out.0 as usize * info.out.1 as usize * 4;
        let all = Command::new(tool("ffmpeg"))
            .args(["-v", "error", "-i"])
            .arg(&film)
            .args(["-an", "-vf", &format!("scale={}:{}:flags=bilinear", info.out.0, info.out.1), "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
            .output()
            .unwrap()
            .stdout;
        assert_eq!(all.len() / n, 120);
        for idx in [0u64, 1, 29, 30, 77, 119] {
            let mut c = spawn_decoder(&film, idx, &info).unwrap();
            let mut buf = vec![0u8; n];
            c.stdout.take().unwrap().read_exact(&mut buf).unwrap();
            let _ = c.kill();
            let _ = c.wait();
            let i = idx as usize;
            assert!(buf == all[i * n..(i + 1) * n], "frame {idx}");
        }
    }

    #[test]
    fn frames_land_on_the_mixer_s_time() {
        let i = Info { width: 2, height: 2, rate: (30000, 1001), frames: 300, secs: 10.0, audio: false, out: (2, 2) };
        assert_eq!(i.frame_at(0.0), 0);
        // Frame 100 starts at 100 * 1001 / 30000 s; a hair before is still 99.
        assert_eq!(i.frame_at(i.secs_of(100)), 100);
        assert_eq!(i.frame_at(i.secs_of(100) - 0.001), 99);
        assert_eq!(i.frame_at(99.0), 299);
        // 2× doesn't skip the clock: the frame follows the time, whatever the speed.
        let twelve = Info { rate: (12, 1), ..i };
        assert_eq!(twelve.frame_at(19.5), 234);
    }
}
