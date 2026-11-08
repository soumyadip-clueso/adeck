//! The Shared tab: every page shared from adeck, newest first, with its link, what it holds
//! and where its audio went. Take one down and its page and audio both go; pages taken down
//! elsewhere show as gone and their S3 audio is cleared.

use rune::{El, Tx, col, row, spacer, text};
use rune_kit::{Kit, Spinner};

use crate::app::{App, Tab};
use crate::share::Share;
use crate::ui::px;

pub fn build(s: &App, t: &Tab, width: f32, k: &Kit) -> El {
    let live = s.shares.iter().filter(|x| !x.gone).count();
    let gone = s.shares.len() - live;
    let mut lamps = row().gap(14.0).center_y().flex_wrap();
    if s.shares.is_empty() {
        lamps = lamps.child(px::lamp("Nothing shared yet", px::SILK));
    } else {
        lamps = lamps.child(px::lamp(&format!("{live} live"), px::GOOD));
        if gone > 0 {
            lamps = lamps.child(px::lamp(&format!("{gone} taken down elsewhere"), px::SILK));
        }
    }
    match &s.shares_checked {
        None if s.cfg.share.command().is_some() => lamps = lamps.child(row().gap(6.0).center_y().child(Spinner::new().size(10.0).build(k)).child(px::label("Checking…"))),
        Some(Err(e)) => lamps = lamps.child(px::lamp(&format!("Couldn't check: {e}"), px::BAD)),
        _ => {}
    }
    let tools = row()
        .gap(8.0)
        .center_y()
        .child(crate::ui::widgets::search_field(t, "Search", 200.0, k))
        .child(px::key("REFRESH", false).key("shared-refresh").help("Check which pages are still up").on_press(|s: &mut App| s.check_shares()));
    let head = px::header("shared-head", "Shared", lamps, tools);
    let mut list = col().key("shared-list").w_full().gap(8.0).pr(10.0);
    if s.cfg.share.command().is_none() {
        list = list.child(
            col()
                .w_full()
                .gap(8.0)
                .px(16.0)
                .py(16.0)
                .radius(6.0)
                .fill(px::SCREEN)
                .border(1.0, px::LINE)
                .child(text("SHARING ISN'T SET UP").font(px::pixel(13.0)).color(px::LED))
                .child(
                    text("Add a [share] section to ~/.config/adeck/config.toml naming the program that publishes a page (see the README).")
                        .font(rune::TextStyle::new(13.0, 400.0))
                        .color(px::SILK)
                        .wrap(),
                ),
        );
    }
    let q = t.filter.text.trim().to_lowercase();
    let shown: Vec<_> = s.shares.iter().filter(|x| q.is_empty() || x.title.to_lowercase().contains(&q) || x.url.to_lowercase().contains(&q)).collect();
    for sh in &shown {
        list = list.child(entry(s, sh, width));
    }
    if shown.is_empty() && !s.shares.is_empty() {
        list = list.child(text(format!("No shared page matches “{}”.", t.filter.text.trim())).font(rune::TextStyle::new(13.0, 400.0)).color(px::FAINT).py(16.0));
    }
    col().key("shared").w_full().h_full().min_h(0.0).gap(14.0).child(head).child(col().w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
}

fn entry(s: &App, sh: &Share, width: f32) -> El {
    let busy = s.unsharing.contains(&sh.slug);
    let confirm = s.confirm_unshare.as_deref() == Some(sh.slug.as_str());
    let audio = if sh.audio == "bundle" { "audio in the page" } else { "audio on S3" };
    let facts = format!("{} · {} sound{} · {audio}", ago(sh.created), sh.files, if sh.files == 1 { "" } else { "s" });
    let mut name = row().gap(10.0).center_y().min_w(0.0).child(text(sh.title.to_uppercase()).font(px::pixel(16.0)).color(if sh.gone { px::SILK } else { px::LED }).truncate());
    name = name.child(if sh.gone { px::tag("DOWN", px::SILK) } else { px::lamp("Live", px::GOOD) });
    let info = col()
        .gap(5.0)
        .min_w(0.0)
        .shrink()
        .child(name)
        .child(text(sh.url.clone()).font(rune::TextStyle::new(12.0, 400.0).mono()).color(px::SOFT).truncate().selectable(true))
        .child(text(facts.to_uppercase()).font(px::pixel(10.0)).color(px::FAINT));
    let (url, url2, slug) = (sh.url.clone(), sh.url.clone(), sh.slug.clone());
    let mut actions = row().gap(6.0).center_y();
    if !sh.gone {
        actions = actions
            .child(px::key("OPEN", false).key(format!("share-open-{}", sh.slug)).on_press(move |_: &mut App| {
                let _ = std::process::Command::new("open").arg(&url).spawn();
            }))
            .child(px::key("COPY LINK", false).key(format!("share-copy-{}", sh.slug)).on_press_tx(move |s: &mut App, tx: &mut Tx| {
                tx.copy(url2.clone());
                s.toast = Some(("Link copied".into(), false));
            }));
    }
    let label = if busy {
        "TAKING DOWN…"
    } else if confirm {
        "CONFIRM: DELETE PAGE + AUDIO"
    } else if sh.gone {
        "FORGET"
    } else {
        "TAKE DOWN"
    };
    let mut down = px::key(label, false).key(format!("share-down-{}", sh.slug)).help(if sh.gone {
        "Already down; removes it from this list and clears any S3 audio"
    } else {
        "Deletes the page and its audio. Press twice."
    });
    if confirm {
        down = down.fill(px::BAD).border(1.0, px::BAD);
    }
    if !busy {
        down = down.on_press(move |s: &mut App| s.unshare(&slug, false));
    }
    actions = actions.child(down);
    let mut r = row()
        .key(format!("share-{}", sh.slug))
        .w_full()
        .gap(12.0)
        .center_y()
        .px(16.0)
        .py(12.0)
        .radius(6.0)
        .fill(px::SCREEN)
        .border(1.0, px::LINE)
        .motion(rune::Spring::SNAPPY)
        .transition(rune::Transition::opacity());
    if sh.gone {
        r = r.opacity(0.6);
    }
    if width < 640.0 {
        r = r.flex_wrap();
    }
    r.child(info).child(spacer()).child(actions)
}

fn ago(t: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let d = now.saturating_sub(t);
    match d {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", d / 60),
        3600..86400 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86400),
    }
}
