mod app;
mod board;
mod config;
mod decode;
mod derive;
mod feedback;
mod item;
mod jobs;
mod mcp;
mod mixer;
mod scenes;
mod share;
mod shot;
mod source;
mod ui;

use rune::{AppSpec, Chrome};

use app::App;

fn main() {
    rune::prewarm_gpu();
    let spec = AppSpec {
        fonts: rune_theme_opencode::FONTS,
        families: rune_theme_opencode::FAMILIES,
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
