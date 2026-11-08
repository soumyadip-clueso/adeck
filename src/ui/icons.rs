//! Icons: OpenCode's catalog where it has one, our own 16 point line icons where it doesn't.

use rune::Icon;

pub use rune_theme_opencode::icons::{CHECK, CLOSE, MAGNIFYING_GLASS as SEARCH, STOP, WARNING};

pub static WAVE: Icon = Icon {
    name: "adeck-wave",
    view: 16.0,
    body: r#"<path d="M2.5 8H3.5M5 5.5V10.5M7 3.5V12.5M9 5V11M11 6.5V9.5M12.5 8H13.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#,
};

pub static PLAY: Icon = Icon {
    name: "adeck-play",
    view: 16.0,
    body: r#"<path d="M5 3.6V12.4C5 12.8 5.45 13.05 5.8 12.83L12.7 8.43C13.02 8.23 13.02 7.77 12.7 7.57L5.8 3.17C5.45 2.95 5 3.2 5 3.6Z" fill="currentColor"/>"#,
};

pub static PAUSE: Icon = Icon {
    name: "adeck-pause",
    view: 16.0,
    body: r#"<rect x="4.25" y="3.25" width="2.6" height="9.5" rx="0.8" fill="currentColor"/><rect x="9.15" y="3.25" width="2.6" height="9.5" rx="0.8" fill="currentColor"/>"#,
};

pub static DROP: Icon = Icon {
    name: "adeck-drop",
    view: 16.0,
    body: r#"<path d="M8 2.5V9.5M5.25 6.75L8 9.5L10.75 6.75" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M2.75 9.75V11.75C2.75 12.3 3.2 12.75 3.75 12.75H12.25C12.8 12.75 13.25 12.3 13.25 11.75V9.75" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

pub static SPARK: Icon = Icon {
    name: "adeck-spark",
    view: 16.0,
    body: r#"<path d="M8 2.75L9.2 6.8L13.25 8L9.2 9.2L8 13.25L6.8 9.2L2.75 8L6.8 6.8Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/>"#,
};

pub static COMPARE: Icon = Icon {
    name: "adeck-compare",
    view: 16.0,
    body: r#"<path d="M3 5H13M3 8H13M3 11H13" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" opacity="0.35"/><path d="M5 3.5V12.5M11 3.5V12.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/>"#,
};

pub static FILM: Icon = Icon {
    name: "adeck-film",
    view: 16.0,
    body: r#"<rect x="2.75" y="3.75" width="10.5" height="8.5" rx="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M7 6.3V9.7L9.8 8Z" fill="currentColor"/>"#,
};

pub static TABLE: Icon = Icon {
    name: "adeck-table",
    view: 16.0,
    body: r#"<rect x="2.75" y="3.25" width="10.5" height="9.5" rx="1.5" stroke="currentColor" stroke-width="1.2" fill="none"/><path d="M2.75 6.5H13.25M2.75 9.5H13.25M6.5 6.5V12.75" stroke="currentColor" stroke-width="1.2"/>"#,
};
