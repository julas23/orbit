//! Follows the system's colours: light or dark, and the accent colour.
//!
//! Both come from the XDG desktop portal (`org.freedesktop.appearance`:
//! `color-scheme` and `accent-color`), which desktops and their settings
//! apps keep up to date, and which announces changes — so the panel and the
//! dock switch live. When the system states no preference, the GTK theme's
//! name decides — `GTK_THEME` if set, else the configured theme
//! ("Greybird-dark" -> dark).
//!
//! The colours themselves are two palettes of `@define-color` names
//! (data/palette-dark.css, data/palette-light.css), loaded in their own CSS
//! provider above the built-in theme and below the user's style.css, which
//! can still override any of them.

use std::rc::Rc;

use gtk::{gdk, gio, glib, prelude::*};

const DARK: &str = include_str!("../../../data/palette-dark.css");
const LIGHT: &str = include_str!("../../../data/palette-light.css");

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const APPEARANCE: &str = "org.freedesktop.appearance";

/// Keeps the palette provider and the portal subscription alive.
pub struct Theme {
    _changes: Option<gio::SignalSubscription>,
}

struct State {
    provider: gtk::CssProvider,
    portal: Option<gio::DBusConnection>,
}

/// Installs the palette for `display` and keeps it in step with the system.
pub fn install(display: &gdk::Display) -> Theme {
    let provider = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        // Above the built-in theme (APPLICATION), below style.css (USER).
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    let portal = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok();
    let state = Rc::new(State { provider, portal });
    state.apply();

    let changes = state.portal.as_ref().map(|conn| {
        let state = state.clone();
        conn.subscribe_to_signal(
            Some(PORTAL),
            Some(SETTINGS),
            Some("SettingChanged"),
            Some(PORTAL_PATH),
            Some(APPEARANCE),
            gio::DBusSignalFlags::NONE,
            move |_| state.apply(),
        )
    });
    Theme { _changes: changes }
}

/// The accent colour to draw with directly (outside CSS): the system's, or
/// the palette's own when the desktop sets none.
pub fn accent() -> gdk::RGBA {
    let state = State {
        provider: gtk::CssProvider::new(),
        portal: gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).ok(),
    };
    match state.accent() {
        Some((r, g, b)) => gdk::RGBA::new(r as f32, g as f32, b as f32, 1.0),
        None => gdk::RGBA::parse(if state.dark() { "#4682b4" } else { "#3b6ea5" })
            .expect("valid colour"),
    }
}

impl State {
    fn apply(&self) {
        let dark = self.dark();
        if let Some(settings) = gtk::Settings::default() {
            // GTK's own widgets (menus, sliders) follow the same choice.
            settings.set_gtk_application_prefer_dark_theme(dark);
        }
        let mut css = String::from(if dark { DARK } else { LIGHT });
        if let Some((r, g, b)) = self.accent() {
            css.push_str(&format!(
                "\n@define-color orbit_accent rgb({}, {}, {});\n",
                channel(r),
                channel(g),
                channel(b)
            ));
        }
        self.provider.load_from_string(&css);
        log::info!(
            "theme: {} palette{}",
            if dark { "dark" } else { "light" },
            if self.accent().is_some() {
                ", system accent"
            } else {
                ""
            }
        );
    }

    /// Reads one appearance setting from the portal.
    fn read(&self, key: &str) -> Option<glib::Variant> {
        let conn = self.portal.as_ref()?;
        let call = |method: &str| {
            conn.call_sync(
                Some(PORTAL),
                PORTAL_PATH,
                SETTINGS,
                method,
                Some(&(APPEARANCE, key).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                500,
                gio::Cancellable::NONE,
            )
            .ok()
        };
        // ReadOne returns (v); the older Read wraps the value once more.
        call("ReadOne")
            .and_then(|r| r.child_value(0).as_variant())
            .or_else(|| {
                call("Read")
                    .and_then(|r| r.child_value(0).as_variant())
                    .and_then(|v| v.as_variant())
            })
    }

    /// 1 = prefer dark, 2 = prefer light, 0 or unknown = no preference.
    fn dark(&self) -> bool {
        match self.read("color-scheme").and_then(|v| v.get::<u32>()) {
            Some(1) => true,
            Some(2) => false,
            // No preference: the theme GTK uses — GTK_THEME when set, which
            // overrides the configured one, else the configured one.
            _ => std::env::var("GTK_THEME")
                .ok()
                .filter(|t| !t.is_empty())
                .or_else(|| {
                    gtk::Settings::default()
                        .and_then(|s| s.gtk_theme_name())
                        .map(|n| n.to_string())
                })
                .is_some_and(|name| name.to_lowercase().contains("dark")),
        }
    }

    /// The accent colour as (r, g, b) in 0..=1, when the desktop sets one.
    fn accent(&self) -> Option<(f64, f64, f64)> {
        let (r, g, b) = self.read("accent-color")?.get::<(f64, f64, f64)>()?;
        let valid = |c: f64| (0.0..=1.0).contains(&c);
        (valid(r) && valid(g) && valid(b)).then_some((r, g, b))
    }
}

fn channel(c: f64) -> u8 {
    (c * 255.0).round() as u8
}
