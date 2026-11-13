mod app;
mod board;
mod config;
mod decode;
mod derive;
mod dock;
mod feedback;
mod item;
mod jobs;
mod mcp;
mod mixer;
mod scenes;
mod share;
mod shot;
mod source;
mod stems;
mod ui;

use rune::{AppSpec, Chrome};

use app::App;

/// Geist for text, Geist Mono for values, Geist Pixel for labels, numbers and titles (all SIL
/// Open Font License; see assets/fonts).
const FONTS: &[&[u8]] = &[include_bytes!("../assets/fonts/Geist.ttf"), include_bytes!("../assets/fonts/GeistMono.ttf"), include_bytes!("../assets/fonts/GeistPixel.ttf")];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `adeck --refresh-shares [slug…]`: put published shares on this build's page, same links.
    if let Some(i) = args.iter().position(|a| a == "--refresh-shares") {
        let (cfg, _) = config::Config::load();
        let only = &args[i + 1..];
        let mut failed = false;
        for sh in share::load().iter().filter(|s| !s.gone && (only.is_empty() || only.contains(&s.slug))) {
            match share::refresh(sh, &cfg.share) {
                Ok(n) => println!("{}: {n} of {} files redrawn · {}", sh.slug, sh.files, sh.url),
                Err(e) => {
                    failed = true;
                    eprintln!("{}: {e}", sh.slug);
                }
            }
        }
        std::process::exit(i32::from(failed));
    }
    if let Some(r) = dock::dump(&args) {
        if let Err(e) = r {
            eprintln!("adeck: {e}");
            std::process::exit(1);
        }
        return;
    }
    rune::prewarm_gpu();
    let spec = AppSpec {
        fonts: FONTS,
        families: ("Geist", "Geist Mono"),
        setup: Some(|s: &mut App, w| s.setup(w)),
        poll: Some(|s: &mut App, clock, tx| s.poll(clock, tx)),
        on_key: Some(ui::on_key),
        min_size: Some((380.0, 520.0)),
        scenes: scenes::ALL,
        ..AppSpec::new("adeck", (1280.0, 800.0), App::new, ui::view)
    };
    let chrome = Chrome { full_size_content: true, traffic_lights: Some((18.0, 18.0)), drag_regions: true };
    rune::App::new(spec).chrome(chrome).run();
}
