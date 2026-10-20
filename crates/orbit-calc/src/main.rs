//! Orbit-Calc: a calculator with profiles — standard, scientific, HP-12C
//! and IPv4 subnet.
//!
//! One window, a tab for each profile in its header bar; it always opens on
//! the standard one. The window has a fixed size, each profile's own — which
//! also makes tiling compositors such as sway float it.

mod algebraic;
mod entry;
mod expr;
mod format;
mod hp12c;
mod ipv4;
mod subnet;

use gtk::{glib, prelude::*};
use orbit_app::i18n::tr;
use orbit_common::ConfigDir;

/// (id, message id of its name), in the order of the tabs.
const PROFILES: &[(&str, &str)] = &[
    ("standard", "calc-profile-standard"),
    ("scientific", "calc-profile-scientific"),
    ("hp12c", "calc-profile-financial"),
    ("subnet", "calc-profile-subnet"),
];

fn main() -> glib::ExitCode {
    orbit_app::run_app(orbit_app::App {
        name: "orbit-calc",
        app_id: "io.github.julas23.Orbit.Calc",
        css: include_str!("../../../data/calc.css"),
        build,
        resident: false,
    })
}

fn build(app: &gtk::Application, _dir: &ConfigDir) -> anyhow::Result<gtk::ApplicationWindow> {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(tr("calc-title"))
        .css_classes(["orbit-calc"])
        .resizable(false)
        .build();

    let stack = gtk::Stack::builder()
        .hhomogeneous(false)
        .vhomogeneous(false)
        .build();
    let pages: [gtk::Widget; 4] = [
        algebraic::new(&window, algebraic::Layout::Standard),
        algebraic::new(&window, algebraic::Layout::Scientific),
        hp12c::ui::new(&window),
        subnet::new(),
    ];
    for ((id, title), page) in PROFILES.iter().zip(&pages) {
        stack.add_titled(page, Some(id), &tr(title));
    }
    // Always the standard profile first.
    stack.set_visible_child_name(PROFILES[0].0);
    {
        let window = window.downgrade();
        stack.connect_visible_child_notify(move |_| {
            // To the new profile's size, smaller or larger.
            if let Some(window) = window.upgrade() {
                window.set_default_size(-1, -1);
            }
        });
    }

    // The tabs: one toggle button per profile, each as wide as its name
    // (GTK's StackSwitcher makes them all as wide as the widest, which would
    // stretch the narrow profiles).
    let tabs = gtk::Box::builder().css_classes(["tabs", "linked"]).build();
    let mut group: Option<gtk::ToggleButton> = None;
    for (index, (id, title)) in PROFILES.iter().enumerate() {
        let tab = gtk::ToggleButton::builder()
            .label(tr(title))
            .active(index == 0)
            .focus_on_click(false)
            .build();
        tab.set_group(group.as_ref());
        group.get_or_insert_with(|| tab.clone());
        let stack = stack.clone();
        tab.connect_toggled(move |tab| {
            if tab.is_active() {
                stack.set_visible_child_name(id);
            }
        });
        tabs.append(&tab);
    }
    // A title bar of our own: the tabs at the start, the close button at
    // the end. GTK's HeaderBar centres its title and so reserves the width
    // of its widest end on both sides, which would widen the window.
    let close = gtk::WindowControls::builder()
        .side(gtk::PackType::End)
        // Only closing: a fixed-size window has nothing to maximize.
        .decoration_layout(":close")
        .build();
    let bar = gtk::Box::builder().css_classes(["titlebar-row"]).build();
    bar.append(&tabs);
    bar.append(&gtk::Box::builder().hexpand(true).build());
    bar.append(&close);
    // Dragging the bar moves the window; double-clicking does nothing here.
    let handle = gtk::WindowHandle::builder().child(&bar).build();
    window.set_titlebar(Some(&handle));
    window.set_child(Some(&stack));
    Ok(window)
}
