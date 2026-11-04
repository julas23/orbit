//! Small media buttons — previous, play/pause, next — for whatever player
//! is on the session bus (MPRIS: Spotify, browsers, Orbit-Play, mpv…).
//! The player playing now is the one controlled; with none playing, the
//! first one found. The middle button shows play or pause as the player is,
//! checked every two seconds while the panel runs.

use std::time::Duration;

use gtk::{gio, glib, prelude::*};
use orbit_app::i18n::tr;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

const PREVIOUS: &str = "\u{f04ae}"; // 󰒮
const PLAY: &str = "\u{f040a}"; // 󰐊
const PAUSE: &str = "\u{f03e4}"; // 󰏤
const NEXT: &str = "\u{f04ad}"; // 󰒭

/// The bus names of the players, and whether each is playing.
async fn players(bus: &gio::DBusConnection) -> Vec<(String, bool)> {
    let names = bus
        .call_future(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ListNames",
            None,
            Some(glib::VariantTy::new("(as)").expect("a type")),
            gio::DBusCallFlags::NONE,
            1000,
        )
        .await
        .ok()
        .and_then(|v| v.child_value(0).get::<Vec<String>>())
        .unwrap_or_default();
    let mut found = Vec::new();
    for name in names.into_iter().filter(|n| n.starts_with(PREFIX)) {
        let status = bus
            .call_future(
                Some(&name),
                PATH,
                "org.freedesktop.DBus.Properties",
                "Get",
                Some(&(PLAYER, "PlaybackStatus").to_variant()),
                Some(glib::VariantTy::new("(v)").expect("a type")),
                gio::DBusCallFlags::NONE,
                1000,
            )
            .await
            .ok()
            .and_then(|v| v.child_value(0).as_variant())
            .and_then(|v| v.get::<String>());
        found.push((name, status.as_deref() == Some("Playing")));
    }
    found
}

/// The player to act on: the one playing, else the first.
async fn current(bus: &gio::DBusConnection) -> Option<(String, bool)> {
    let players = players(bus).await;
    players.iter().find(|(_, playing)| *playing).or(players.first()).cloned()
}

fn button(glyph: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(glyph)
        .tooltip_text(tooltip)
        .css_classes(["flat", "media-button"])
        .hexpand(true)
        .build()
}

/// The three buttons, in a row.
pub fn controls() -> gtk::Widget {
    let row = gtk::Box::builder()
        .css_classes(["media-controls"])
        .homogeneous(true)
        .build();
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
        return row.upcast();
    };
    let previous = button(PREVIOUS, &tr("media-previous"));
    let play = button(PLAY, &tr("media-play-pause"));
    let next = button(NEXT, &tr("media-next"));
    row.append(&previous);
    row.append(&play);
    row.append(&next);

    let update = {
        let (bus, play) = (bus.clone(), play.downgrade());
        move || {
            let (bus, play) = (bus.clone(), play.clone());
            glib::spawn_future_local(async move {
                let state = current(&bus).await;
                if let Some(play) = play.upgrade() {
                    play.set_label(if state.as_ref().is_some_and(|(_, p)| *p) { PAUSE } else { PLAY });
                    play.set_sensitive(state.is_some());
                }
            });
        }
    };

    for (widget, method) in [(&previous, "Previous"), (&play, "PlayPause"), (&next, "Next")] {
        let (bus, update) = (bus.clone(), update.clone());
        widget.connect_clicked(move |_| {
            let (bus, update) = (bus.clone(), update.clone());
            glib::spawn_future_local(async move {
                if let Some((name, _)) = current(&bus).await {
                    let sent = bus
                        .call_future(Some(&name), PATH, PLAYER, method, None, None, gio::DBusCallFlags::NONE, 2000)
                        .await;
                    if let Err(err) = sent {
                        log::warn!("media: {name} {method}: {err}");
                    }
                }
                // The player takes a moment to change its state.
                glib::timeout_add_local_once(Duration::from_millis(300), update);
            });
        });
    }

    update();
    let weak = row.downgrade();
    glib::timeout_add_seconds_local(2, move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        update();
        glib::ControlFlow::Continue
    });
    row.upcast()
}
