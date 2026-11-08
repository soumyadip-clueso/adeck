//! Screenshots: a tab rendered by a headless copy of adeck, and the framed version for sharing,
//! with the window on a soft gradient, rounded and lifted by a shadow, saved to ~/Pictures/adeck
//! and put on the clipboard.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use image::{GrayImage, ImageBuffer, Luma, Rgba, RgbaImage, imageops};

/// Renders tab `tab` as the window shows it (the caller saves the session first).
pub fn render(tab: u64, size: &str, theme: &str, scale: u32) -> Result<Vec<u8>, String> {
    let out = std::env::temp_dir().join(format!("adeck-shot-{}-{tab}-{scale}.png", std::process::id()));
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let run = Command::new(exe)
        .args(["--snap", &out.display().to_string(), "--size", size, "--scale", &scale.to_string(), "--theme", theme])
        .env("ADECK_FOCUS", tab.to_string())
        .env("ADECK_HEADLESS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Couldn't start the headless render: {e}"))?;
    if !run.status.success() {
        let err = String::from_utf8_lossy(&run.stderr);
        return Err(format!("The headless render failed: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no output")));
    }
    let png = std::fs::read(&out).map_err(|e| format!("No screenshot was written: {e}"))?;
    let _ = std::fs::remove_file(&out);
    Ok(png)
}

/// The window on a backdrop: gradient, rounded corners, a hairline edge and a soft shadow.
pub fn frame(png: &[u8], dark: bool, scale: u32) -> Result<Vec<u8>, String> {
    let win = image::load_from_memory(png).map_err(|e| format!("The render didn't decode: {e}"))?.to_rgba8();
    let (w, h) = win.dimensions();
    let pad = (w as f32 * 0.075).round().max(48.0) as u32;
    let (cw, ch) = (w + 2 * pad, h + 2 * pad);
    let radius = 12.0 * scale as f32;

    // Backdrop: a diagonal blend of the icon's blue and orange, deep in dark mode, pale in light.
    let stops: [[f32; 3]; 3] =
        if dark { [[22.0, 32.0, 64.0], [52.0, 36.0, 92.0], [110.0, 52.0, 40.0]] } else { [[214.0, 228.0, 255.0], [236.0, 226.0, 252.0], [255.0, 222.0, 200.0]] };
    let mut out = RgbaImage::from_fn(cw, ch, |x, y| {
        let t = ((x as f32 / cw as f32) * 0.6 + (y as f32 / ch as f32) * 0.4).clamp(0.0, 1.0);
        let (a, b, f) = if t < 0.5 { (stops[0], stops[1], t * 2.0) } else { (stops[1], stops[2], (t - 0.5) * 2.0) };
        let mix = |i: usize| (a[i] + (b[i] - a[i]) * f).round() as u8;
        Rgba([mix(0), mix(1), mix(2), 255])
    });

    // Shadow: the window's shape, dropped, blurred at quarter size and laid down soft.
    let q = 4u32;
    let (sw, sh) = (cw / q, ch / q);
    let drop = pad as f32 * 0.22;
    let mut mask = GrayImage::from_fn(sw, sh, |x, y| {
        let cov = coverage((x * q) as f32 + q as f32 / 2.0, (y * q) as f32 + q as f32 / 2.0 - drop, pad as f32, pad as f32, w as f32, h as f32, radius);
        Luma([(cov * 255.0) as u8])
    });
    mask = imageops::blur(&mask, pad as f32 / q as f32 / 2.2);
    let mask: ImageBuffer<Luma<u8>, Vec<u8>> = imageops::resize(&mask, cw, ch, imageops::FilterType::Triangle);
    let strength = if dark { 0.62 } else { 0.32 };
    for (x, y, p) in out.enumerate_pixels_mut() {
        let a = mask.get_pixel(x, y)[0] as f32 / 255.0 * strength;
        for c in 0..3 {
            p[c] = (p[c] as f32 * (1.0 - a)).round() as u8;
        }
    }

    // The window, its corners antialiased, then a hairline edge.
    let edge: [f32; 4] = if dark { [255.0, 255.0, 255.0, 0.14] } else { [0.0, 0.0, 0.0, 0.10] };
    for y in 0..h {
        for x in 0..w {
            let (ox, oy) = (x + pad, y + pad);
            let (fx, fy) = (ox as f32 + 0.5, oy as f32 + 0.5);
            let cov = coverage(fx, fy, pad as f32, pad as f32, w as f32, h as f32, radius);
            if cov <= 0.0 {
                continue;
            }
            let src = win.get_pixel(x, y);
            let inner = coverage(fx, fy, pad as f32 + scale as f32, pad as f32 + scale as f32, w as f32 - 2.0 * scale as f32, h as f32 - 2.0 * scale as f32, radius - scale as f32);
            let ring = (cov - inner).clamp(0.0, 1.0) * edge[3];
            let dst = out.get_pixel_mut(ox, oy);
            for c in 0..3 {
                let s = src[c] as f32 * (1.0 - ring) + edge[c] * ring;
                dst[c] = (dst[c] as f32 * (1.0 - cov) + s * cov).round() as u8;
            }
        }
    }
    // The window buttons the headless render leaves room for.
    let sc = scale as f32;
    for (i, rgb) in [[255.0, 95.0, 87.0], [254.0, 188.0, 46.0], [40.0, 200.0, 64.0]].iter().enumerate() {
        let (cx, cy, r) = (pad as f32 + (20.0 + 20.0 * i as f32) * sc, pad as f32 + 20.0 * sc, 6.0 * sc);
        for y in (cy - r - 1.0) as u32..=(cy + r + 1.0) as u32 {
            for x in (cx - r - 1.0) as u32..=(cx + r + 1.0) as u32 {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                let a = (r + 0.5 - d).clamp(0.0, 1.0);
                if a > 0.0 && x < cw && y < ch {
                    let p = out.get_pixel_mut(x, y);
                    for c in 0..3 {
                        p[c] = (p[c] as f32 * (1.0 - a) + rgb[c] * a).round() as u8;
                    }
                }
            }
        }
    }
    let mut bytes = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// How much of the pixel at (px, py) a rounded rectangle covers, 0 to 1.
fn coverage(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let (hx, hy) = (w / 2.0, h / 2.0);
    let (dx, dy) = ((px - x - hx).abs() - (hx - r), (py - y - hy).abs() - (hy - r));
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    let d = outside + dx.max(dy).min(0.0) - r;
    (0.5 - d).clamp(0.0, 1.0)
}

/// Writes a shot to ~/Pictures/adeck and copies it to the clipboard. Returns where it went.
pub fn keep(png: &[u8], label: &str) -> Result<PathBuf, String> {
    let dir = dirs_pictures().join("adeck");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    let stamp = Command::new("date").arg("+%Y-%m-%d at %H.%M.%S").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    let slug: String =
        label.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { ' ' }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    let path = dir.join(format!("adeck {slug} {stamp}.png"));
    std::fs::write(&path, png).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    let script = format!("set the clipboard to (read (POSIX file \"{}\") as «class PNGf»)", path.display().to_string().replace('"', "\\\""));
    let _ = Command::new("osascript").args(["-e", &script]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    Ok(path)
}

fn dirs_pictures() -> PathBuf {
    std::env::var_os("HOME").map_or_else(std::env::temp_dir, |h| PathBuf::from(h).join("Pictures"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_pads_rounds_and_keeps_the_window() {
        let win = RgbaImage::from_pixel(400, 250, Rgba([200, 10, 10, 255]));
        let mut png = Vec::new();
        win.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let out = image::load_from_memory(&frame(&png, true, 2).unwrap()).unwrap().to_rgba8();
        let pad = 48;
        assert_eq!(out.dimensions(), (400 + 2 * pad, 250 + 2 * pad));
        assert_eq!(out.get_pixel(pad + 200, pad + 125)[0], 200, "the middle is the window");
        assert_ne!(out.get_pixel(pad, pad)[0], 200, "the corner is rounded off");
        assert_ne!(out.get_pixel(5, 5)[0], 200, "the margin is backdrop");
    }
}
