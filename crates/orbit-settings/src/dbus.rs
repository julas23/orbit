//! The system services' D-Bus, for the pages that talk to them
//! (docs/PLANO-SETTINGS.md §1 rule 3): hostnamed, timedated, localed… A
//! call may ask for a password: polkit's agent (orbit-polkit) does, as the
//! call allows interactive authorisation.

use gtk::{gio, glib, prelude::*};

/// The proxy of `name`'s `/org/freedesktop/<last>` object, interface `name`
/// (`org.freedesktop.timedate1` → `/org/freedesktop/timedate1`).
pub fn system(name: &str) -> Option<gio::DBusProxy> {
    let path = format!("/{}", name.replace('.', "/"));
    gio::DBusProxy::for_bus_sync(
        gio::BusType::System,
        gio::DBusProxyFlags::NONE,
        None,
        name,
        &path,
        name,
        gio::Cancellable::NONE,
    )
    .map_err(|e| log::warn!("{name}: {e}"))
    .ok()
}

/// A string property, or empty.
pub fn string(proxy: &gio::DBusProxy, property: &str) -> String {
    proxy
        .cached_property(property)
        .and_then(|v| v.get::<String>())
        .unwrap_or_default()
}

pub fn boolean(proxy: &gio::DBusProxy, property: &str) -> bool {
    proxy
        .cached_property(property)
        .and_then(|v| v.get::<bool>())
        .unwrap_or_default()
}

pub fn strings(proxy: &gio::DBusProxy, property: &str) -> Vec<String> {
    proxy
        .cached_property(property)
        .and_then(|v| v.get::<Vec<String>>())
        .unwrap_or_default()
}

/// Calls `method`; polkit may ask for a password. The error as the user
/// should read it (a refused password says so).
pub async fn call(proxy: &gio::DBusProxy, method: &str, args: glib::Variant) -> Result<glib::Variant, String> {
    proxy
        .call_future(method, Some(&args), gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION, -1)
        .await
        .map_err(|e| {
            let text = e.to_string();
            // GDBus prefixes the remote error's name: what follows is enough.
            match text.split_once(": ") {
                Some((prefix, rest)) if prefix.starts_with("GDBus.Error") => rest.to_owned(),
                _ => text,
            }
        })
}

/// The proxy of `path` on the system bus's `name`, interface `interface`
/// (for the services whose objects do not follow the name: logind's
/// Manager, UPower's devices).
pub fn system_at(name: &str, path: &str, interface: &str) -> Option<gio::DBusProxy> {
    gio::DBusProxy::for_bus_sync(
        gio::BusType::System,
        gio::DBusProxyFlags::NONE,
        None,
        name,
        path,
        interface,
        gio::Cancellable::NONE,
    )
    .map_err(|e| log::warn!("{name} {path}: {e}"))
    .ok()
}

/// Sets a property through org.freedesktop.DBus.Properties; polkit may ask
/// for a password.
pub async fn set_property(proxy: &gio::DBusProxy, property: &str, value: glib::Variant) -> Result<(), String> {
    let (Some(name), Some(interface)) = (proxy.name(), Some(proxy.interface_name())) else {
        return Err("no name".into());
    };
    proxy
        .connection()
        .call_future(
            Some(&name),
            &proxy.object_path(),
            "org.freedesktop.DBus.Properties",
            "Set",
            Some(&(interface.as_str(), property, value).to_variant()),
            None,
            gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
            -1,
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}
