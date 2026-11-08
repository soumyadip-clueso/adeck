//! The Shared tab: every page shared from adeck, newest first, with its link, what it holds
//! and where its audio went. Take one down and its page and audio both go; pages taken down
//! elsewhere show as gone and their S3 audio is cleared.

use rune::{El, Tx, col, row, spacer, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit, Spinner};

use crate::app::App;
use crate::share::Share;

pub fn build(s: &App, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let live = s.shares.iter().filter(|x| !x.gone).count();
    let summary = match s.shares.len() {
        0 => "Nothing shared yet.".to_owned(),
        n if live == n => format!("{n} shared page{}", if n == 1 { "" } else { "s" }),
        n => format!("{live} live · {} taken down elsewhere", n - live),
    };
    let mut status = row().gap(8.0).center_y();
    match &s.shares_checked {
        None if s.cfg.share.command().is_some() => status = status.child(Spinner::new().size(12.0).build(k)).child(text("Checking which are still up…").font(k.ty.small).color(c.text_muted)),
        Some(Err(e)) => status = status.child(text(format!("Couldn't check: {e}")).font(k.ty.small).color(c.danger.fg).truncate()),
        _ => {}
    }
    let head = row()
        .key("shared-head")
        .w_full()
        .gap(12.0)
        .center_y()
        .flex_wrap()
        .child(text(summary).font(k.ty.ui_medium).color(c.text_base))
        .child(status)
        .child(spacer())
        .child(Button::new("Refresh").variant(ButtonVariant::Outline).control_size(ControlSize::Small).on_press(|s: &mut App| s.check_shares()).build(k).key("shared-refresh"));
    let mut list = col().key("shared-list").w_full().gap(8.0).pr(10.0);
    if s.cfg.share.command().is_none() {
        list = list.child(
            col()
                .w_full()
                .gap(6.0)
                .px(16.0)
                .py(16.0)
                .radius(k.radius.lg)
                .border(1.0, c.border_weak)
                .child(text("Sharing isn't set up").font(k.ty.ui_medium).color(c.text_base))
                .child(text("Add a [share] section to ~/.config/adeck/config.toml naming the program that publishes a page (see the README).").font(k.ty.small).color(c.text_muted)),
        );
    }
    for sh in &s.shares {
        list = list.child(entry(s, sh, width, k));
    }
    col().key("shared").w_full().h_full().min_h(0.0).gap(14.0).child(head).child(col().w_full().grow(1.0).min_h(0.0).scroll(rune::ScrollSpec { stick_bottom: false }).child(list))
}

fn entry(s: &App, sh: &Share, width: f32, k: &Kit) -> El {
    let c = &k.c;
    let busy = s.unsharing.contains(&sh.slug);
    let confirm = s.confirm_unshare.as_deref() == Some(sh.slug.as_str());
    let when = ago(sh.created);
    let audio = if sh.audio == "bundle" { "audio in the page".to_owned() } else { "audio on S3".to_owned() };
    let facts = format!("{when} · {} sound{} · {audio}", sh.files, if sh.files == 1 { "" } else { "s" });
    let mut info = col().gap(2.0).min_w(0.0).shrink().child(
        row()
            .gap(8.0)
            .center_y()
            .child(text(sh.title.clone()).font(k.ty.ui_medium).color(if sh.gone { c.text_muted } else { c.text_base }).truncate())
            .child(if sh.gone { text("taken down").font(k.ty.small).color(c.text_faint) } else { text(String::new()) }),
    );
    info = info.child(text(sh.url.clone()).font(k.ty.small.features("'tnum' 1")).color(c.text_muted).truncate().selectable(true)).child(text(facts).font(k.ty.small).color(c.text_faint));
    let url = sh.url.clone();
    let url2 = sh.url.clone();
    let slug = sh.slug.clone();
    let mut actions = row().gap(6.0).center_y();
    if !sh.gone {
        actions = actions
            .child(
                Button::new("Open")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press(move |_: &mut App| {
                        let _ = std::process::Command::new("open").arg(&url).spawn();
                    })
                    .build(k)
                    .key(format!("share-open-{}", sh.slug)),
            )
            .child(
                Button::new("Copy link")
                    .variant(ButtonVariant::Outline)
                    .control_size(ControlSize::Small)
                    .on_press_tx(move |s: &mut App, tx: &mut Tx| {
                        tx.copy(url2.clone());
                        s.toast = Some(("Link copied".into(), false));
                    })
                    .build(k)
                    .key(format!("share-copy-{}", sh.slug)),
            );
    }
    actions = actions.child(
        Button::new(if busy {
            "Taking down…"
        } else if confirm {
            "Confirm: delete page and audio"
        } else if sh.gone {
            "Forget"
        } else {
            "Take down"
        })
        .variant(if confirm { ButtonVariant::Contrast } else { ButtonVariant::Outline })
        .control_size(ControlSize::Small)
        .disabled(busy)
        .help(if sh.gone { "Already down; removes it from this list and clears any S3 audio" } else { "Deletes the page and its audio. Press twice." })
        .on_press(move |s: &mut App| s.unshare(&slug, false))
        .build(k)
        .key(format!("share-down-{}", sh.slug)),
    );
    let mut r = row().key(format!("share-{}", sh.slug)).w_full().gap(12.0).center_y().px(14.0).py(10.0).radius(k.radius.lg).fill(c.layer_01).border(1.0, c.border_weak).motion(rune::Spring::SNAPPY).transition(rune::Transition::opacity());
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
