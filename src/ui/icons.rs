//! Icons: OpenCode's catalog where it has one, our own 16 point line icons where it doesn't.

use rune::Icon;

pub use rune_theme_opencode::icons::{CHECK, CHEVRON_DOWN, CHEVRON_RIGHT, CLOSE, FOLDER, MAGNIFYING_GLASS as SEARCH, OPEN_FILE, PLUS, STOP, WARNING};

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

pub static BOARD: Icon = Icon {
    name: "adeck-board",
    view: 16.0,
    body: r#"<rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="9" y="2.5" width="4.5" height="4.5" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="2.5" y="9" width="4.5" height="4.5" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/><rect x="9" y="9" width="4.5" height="4.5" rx="1" stroke="currentColor" stroke-width="1.2" fill="none"/>"#,
};

pub static DIFF: Icon = Icon {
    name: "adeck-diff",
    view: 16.0,
    body: r#"<path d="M5 2.5V9.5M2.5 6H7.5M9 12H13.5M11.25 2.5L11.25 9" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#,
};

pub static SPEAKER: Icon = Icon {
    name: "adeck-speaker",
    view: 16.0,
    body: r#"<path d="M2.75 6.25H5L8.25 3.5V12.5L5 9.75H2.75Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><path d="M10.5 6C11.2 6.6 11.2 9.4 10.5 10M12.25 4.5C13.7 5.9 13.7 10.1 12.25 11.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#,
};

pub static CAMERA: Icon = Icon {
    name: "adeck-camera",
    view: 16.0,
    body: r#"<path d="M2.5 5.25C2.5 4.7 2.95 4.25 3.5 4.25H5.25L6.25 2.75H9.75L10.75 4.25H12.5C13.05 4.25 13.5 4.7 13.5 5.25V11.75C13.5 12.3 13.05 12.75 12.5 12.75H3.5C2.95 12.75 2.5 12.3 2.5 11.75Z" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" fill="none"/><circle cx="8" cy="8.5" r="2.25" stroke="currentColor" stroke-width="1.2" fill="none"/>"#,
};

pub static SHARE: Icon = Icon {
    name: "adeck-share",
    view: 16.0,
    body: r#"<path d="M8 2.5V10M5.25 5L8 2.25L10.75 5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" stroke-linejoin="round" fill="none"/><path d="M4.5 7.5H3.75C3.2 7.5 2.75 7.95 2.75 8.5V12.5C2.75 13.05 3.2 13.5 3.75 13.5H12.25C12.8 13.5 13.25 13.05 13.25 12.5V8.5C13.25 7.95 12.8 7.5 12.25 7.5H11.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" fill="none"/>"#,
};
