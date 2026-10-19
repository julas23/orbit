//! CSS loading with live reload.
//!
//! Two providers are installed: the built-in theme at application priority
//! and the user stylesheet on top of it at user priority. The user file is
//! watched and re-applied whenever it changes.

use std::path::Path;

use gtk::{gdk, gio, prelude::*};

pub struct Style {
    // Kept alive for as long as the style is: dropping it stops the watch.
    _monitor: Option<gio::FileMonitor>,
}

impl Style {
    pub fn install(display: &gdk::Display, builtin_css: &str, user_path: &Path) -> Self {
        let builtin = gtk::CssProvider::new();
        builtin.load_from_string(builtin_css);
        gtk::style_context_add_provider_for_display(
            display,
            &builtin,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let user = gtk::CssProvider::new();
        user.connect_parsing_error(|_, section, err| {
            log::warn!("css: {}: {}", section.to_str(), err.message());
        });
        gtk::style_context_add_provider_for_display(
            display,
            &user,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );

        let file = gio::File::for_path(user_path);
        load_user(&user, &file);

        let monitor = file
            .monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            .inspect_err(|err| log::warn!("cannot watch {}: {err}", user_path.display()))
            .ok();
        if let Some(monitor) = &monitor {
            let user = user.clone();
            monitor.connect_changed(move |_, file, _, event| {
                use gio::FileMonitorEvent as E;
                if matches!(
                    event,
                    E::ChangesDoneHint | E::Created | E::Deleted | E::MovedIn | E::Renamed
                ) {
                    log::info!("stylesheet changed, reloading");
                    load_user(&user, file);
                }
            });
        }

        Self { _monitor: monitor }
    }
}

fn load_user(provider: &gtk::CssProvider, file: &gio::File) {
    if file.query_exists(gio::Cancellable::NONE) {
        provider.load_from_file(file);
    } else {
        // Clear any previously loaded rules, e.g. after the file is deleted.
        provider.load_from_string("");
    }
}
