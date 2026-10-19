//! com.canonical.dbusmenu: the context menus of tray items.
//!
//! The item exports its menu as a tree (`GetLayout`); this turns it into a
//! GTK popover menu — separators start sections, nested items become
//! submenus, check/radio items show their state — and sends `Event(id,
//! "clicked")` back when an entry is chosen.

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

const IFACE: &str = "com.canonical.dbusmenu";

/// Opens the menu at `path` on `bus` under `anchor`.
pub fn show(conn: &gio::DBusConnection, bus: &str, path: &str, anchor: &impl IsA<gtk::Widget>) {
    let call = |method: &str, args: glib::Variant| {
        conn.call_sync(
            Some(bus),
            path,
            IFACE,
            method,
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            1000,
            gio::Cancellable::NONE,
        )
    };
    // Lets the application refresh the menu before it is read.
    let _ = call("AboutToShow", (0i32,).to_variant());
    let layout = match call(
        "GetLayout",
        (0i32, -1i32, Vec::<String>::new()).to_variant(),
    ) {
        Ok(reply) => reply.child_value(1),
        Err(err) => {
            log::warn!("tray: menu of {bus}{path}: {err}");
            return;
        }
    };

    let group = gio::SimpleActionGroup::new();
    let model = gio::Menu::new();
    let clicked: Rc<dyn Fn(i32)> = {
        let conn = conn.clone();
        let (bus, path) = (bus.to_string(), path.to_string());
        Rc::new(move |id| event(&conn, &bus, &path, id, "clicked"))
    };
    build(&layout, &model, &group, &clicked);

    let popover = gtk::PopoverMenu::from_model(Some(&model));
    popover.insert_action_group("m", Some(&group));
    popover.set_has_arrow(false);
    popover.set_parent(anchor);
    {
        let conn = conn.clone();
        let (bus, path) = (bus.to_string(), path.to_string());
        popover.connect_closed(move |popover| {
            event(&conn, &bus, &path, 0, "closed");
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
    }
    event(conn, bus, path, 0, "opened");
    popover.popup();
}

/// Appends the children of `node` — `(i id, a{sv} props, av children)` — to
/// `menu`, one section per run between separators. Choosing an entry calls
/// `clicked` with its id.
fn build(
    node: &glib::Variant,
    menu: &gio::Menu,
    group: &gio::SimpleActionGroup,
    clicked: &Rc<dyn Fn(i32)>,
) {
    let mut section = gio::Menu::new();
    let children = node.child_value(2);
    for i in 0..children.n_children() {
        let Some(child) = children.child_value(i).as_variant() else {
            continue;
        };
        let id = child.child_value(0).get::<i32>().unwrap_or_default();
        let props = glib::VariantDict::new(Some(&child.child_value(1)));
        let get_bool =
            |key: &str, default: bool| props.lookup::<bool>(key).ok().flatten().unwrap_or(default);
        let get_str = |key: &str| {
            props
                .lookup::<String>(key)
                .ok()
                .flatten()
                .unwrap_or_default()
        };

        if !get_bool("visible", true) {
            continue;
        }
        if get_str("type") == "separator" {
            if section.n_items() > 0 {
                menu.append_section(None, &section);
                section = gio::Menu::new();
            }
            continue;
        }
        // GTK menus read `_` (access key) and `__` (a literal underscore)
        // exactly as dbusmenu writes them, so the label goes in unchanged.
        let label = get_str("label");
        let has_children = child.child_value(2).n_children() > 0;
        if get_str("children-display") == "submenu" || has_children {
            let sub = gio::Menu::new();
            build(&child, &sub, group, clicked);
            section.append_submenu(Some(&label), &sub);
            continue;
        }

        let name = format!("i{id}");
        let action = match get_str("toggle-type").as_str() {
            "checkmark" | "radio" => {
                let on = props.lookup::<i32>("toggle-state").ok().flatten() == Some(1);
                gio::SimpleAction::new_stateful(&name, None, &on.to_variant())
            }
            _ => gio::SimpleAction::new(&name, None),
        };
        action.set_enabled(get_bool("enabled", true));
        {
            let clicked = clicked.clone();
            action.connect_activate(move |_, _| clicked(id));
        }
        group.add_action(&action);
        section.append(Some(&label), Some(&format!("m.{name}")));
    }
    if section.n_items() > 0 {
        menu.append_section(None, &section);
    }
}

/// `Event(id, event, data, timestamp)`.
fn event(conn: &gio::DBusConnection, bus: &str, path: &str, id: i32, name: &str) {
    let data = glib::Variant::from_variant(&0i32.to_variant());
    let time = u32::try_from(glib::monotonic_time() / 1000).unwrap_or_default();
    conn.call(
        Some(bus),
        path,
        IFACE,
        "Event",
        Some(&(id, name, data, time).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        1000,
        gio::Cancellable::NONE,
        |_| {},
    );
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gtk::{gio, glib, prelude::*};

    /// Section/submenu labels of a menu model, as text, for comparisons.
    fn outline(model: &gio::MenuModel, depth: usize, out: &mut Vec<String>) {
        for i in 0..model.n_items() {
            let label = model
                .item_attribute_value(i, "label", None)
                .and_then(|v| v.str().map(str::to_owned));
            if let Some(label) = label {
                out.push(format!("{}{label}", "  ".repeat(depth)));
            }
            for link in ["section", "submenu"] {
                if let Some(child) = model.item_link(i, link) {
                    if link == "section" && depth == 0 && i > 0 {
                        out.push("--".into());
                    }
                    outline(&child, depth + usize::from(link == "submenu"), out);
                }
            }
        }
    }

    fn convert(text: &str) -> (Vec<String>, gio::SimpleActionGroup, Rc<Cell<i32>>) {
        let reply = glib::Variant::parse(None, text).expect("valid GVariant text");
        let model = gio::Menu::new();
        let group = gio::SimpleActionGroup::new();
        let last = Rc::new(Cell::new(0));
        let clicked: Rc<dyn Fn(i32)> = {
            let last = last.clone();
            Rc::new(move |id| last.set(id))
        };
        super::build(&reply.child_value(1), &model, &group, &clicked);
        let mut out = Vec::new();
        outline(model.upcast_ref(), 0, &mut out);
        (out, group, last)
    }

    /// Same shape as nm-applet's menu: disabled header, separator, a
    /// submenu, a hidden entry, a checkmark and a literal underscore.
    const LAYOUT: &str = "(uint32 1, (0, {'children-display': <'submenu'>}, [\
        <(1, {'enabled': <false>, 'label': <'Rede ethernet'>}, @av [])>,\
        <(2, {'label': <'Desconectar'>}, @av [])>,\
        <(3, {'type': <'separator'>}, @av [])>,\
        <(4, {'children-display': <'submenu'>, 'label': <'Re_des'>}, [\
            <(5, {'label': <'CASA__2G'>}, @av [])>]) >,\
        <(6, {'visible': <false>, 'label': <'Escondido'>}, @av [])>,\
        <(7, {'label': <'_Ativar rede'>, 'toggle-type': <'checkmark'>, 'toggle-state': <1>}, @av [])>]))";

    #[test]
    fn sections_submenus_hidden_and_labels() {
        let (out, _, _) = convert(LAYOUT);
        assert_eq!(
            out,
            [
                "Rede ethernet",
                "Desconectar",
                "--",
                "Re_des",
                "  CASA__2G",
                "_Ativar rede"
            ]
        );
    }

    #[test]
    fn states_and_clicks() {
        let (_, group, last) = convert(LAYOUT);
        assert!(!group.is_action_enabled("i1"), "disabled stays disabled");
        assert_eq!(
            group.action_state("i7"),
            Some(true.to_variant()),
            "checkmark on"
        );
        group.activate_action("i2", None);
        assert_eq!(last.get(), 2, "choosing an entry sends its id");
    }

    /// Local check against a real menu dump (not in the repository: it holds
    /// nearby Wi-Fi names). Run with ORBIT_DBUSMENU_DUMP=path.
    #[test]
    fn real_dump_if_given() {
        let Ok(path) = std::env::var("ORBIT_DBUSMENU_DUMP") else {
            return;
        };
        let text = std::fs::read_to_string(path).expect("readable dump");
        let (out, _, _) = convert(text.trim());
        assert!(!out.is_empty());
        eprintln!(
            "{} entries, first: {:?}",
            out.len(),
            &out[..out.len().min(8)]
        );
    }
}
