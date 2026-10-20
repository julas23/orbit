//! Menu module: the application-menu button and its menu, after the
//! Whisker menu of XFCE — a search bar on top, the categories down the
//! left, their applications on the right, and the power button at the
//! bottom, which opens the power-off / reboot / log-out choice over the
//! blurred screen.
//!
//! Typing searches every application (name, generic name, keywords,
//! program), accents and case aside; Enter starts the first one found or,
//! with none found, runs what was typed as a command.
//!
//! The menu is a layer-shell surface of its own over the whole monitor,
//! less the panel: it takes the keyboard while open, and a click outside
//! the menu closes it, as does Escape or the button again. With `command`
//! set in `modules/menu.yaml`, the button runs that instead (`rofi -show
//! drun`…).

use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

use gio_unix::prelude::*;
use gtk::{gdk, gio, glib, prelude::*};
use gtk4_layer_shell::{Edge as LayerEdge, KeyboardMode, Layer, LayerShell};
use orbit_app::i18n::{text, tr, tr_args};
use orbit_common::config::Edge;
use serde::Deserialize;

use crate::session::{self, SessionConfig};

/// `modules/menu.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MenuConfig {
    /// Run on click instead of opening the menu, e.g. `rofi -show drun`.
    pub command: Option<String>,
    /// Icon name from the theme, or a path to an image file.
    pub icon: Option<String>,
    pub tooltip: Option<String>,
}

impl Default for MenuConfig {
    fn default() -> Self {
        Self {
            command: None,
            icon: Some("start-here".into()),
            tooltip: None,
        }
    }
}

/// The menu's size, on the 5 px grid.
const WIDTH: i32 = 600;
const HEIGHT: i32 = 550;
/// From the panel and the screen's edges.
const GAP: i32 = 5;

/// The categories, in the menu's order: (message id, icon, the
/// freedesktop.org categories that fall in it). An application goes in the
/// first that matches; the last takes the rest.
const CATEGORIES: &[(&str, &str, &[&str])] = &[
    ("menu-category-utilities", "applications-utilities", &["Utility", "Accessibility"]),
    ("menu-category-development", "applications-development", &["Development"]),
    ("menu-category-education", "applications-science", &["Education", "Science"]),
    ("menu-category-games", "applications-games", &["Game"]),
    ("menu-category-graphics", "applications-graphics", &["Graphics"]),
    ("menu-category-internet", "applications-internet", &["Network"]),
    ("menu-category-multimedia", "applications-multimedia", &["AudioVideo", "Audio", "Video"]),
    ("menu-category-office", "applications-office", &["Office"]),
    ("menu-category-settings", "preferences-desktop", &["Settings", "DesktopSettings"]),
    ("menu-category-system", "applications-system", &["System"]),
    ("menu-category-other", "applications-other", &[]),
];

pub fn new(config: &MenuConfig, edge: Edge, session: SessionConfig) -> gtk::Widget {
    let button = gtk::Button::builder().css_classes(["menu"]).build();
    // The icon at the start, the label beside it.
    let content = gtk::Box::builder()
        .spacing(10)
        .halign(gtk::Align::Start)
        .build();
    if let Some(icon) = &config.icon {
        // 30 px + 5 px padding each side = 40 px, one column of the grid.
        content.append(&orbit_app::icon(icon, 30));
    }
    content.append(
        &gtk::Label::builder()
            .label(tr("menu-label"))
            .css_classes(["label"])
            .build(),
    );
    button.set_child(Some(&content));
    button.set_tooltip_text(config.tooltip.as_deref());

    if let Some(command) = config.command.clone() {
        button.connect_clicked(move |_| {
            if let Err(err) = glib::spawn_command_line_async(&command) {
                log::error!("running {command:?}: {err}");
            }
        });
        return button.upcast();
    }

    let session = Rc::new(session);
    let open: Rc<RefCell<Weak<Menu>>> = Rc::default();
    button.connect_clicked(move |button| {
        // The button again closes it.
        if let Some(menu) = open.borrow().upgrade() {
            menu.close();
            return;
        }
        let menu = Menu::new(button, edge, session.clone());
        *open.borrow_mut() = Rc::downgrade(&menu);
        menu.window.present();
    });
    button.upcast()
}

struct App {
    info: gio_unix::DesktopAppInfo,
    /// Folded name, generic name, keywords and program, for the search.
    haystack: String,
    folded_name: String,
    category: usize,
}

struct Menu {
    window: gtk::Window,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    /// Shown when nothing matches: Enter runs what was typed.
    hint: gtk::Label,
    apps: HashMap<gtk::ListBoxRow, App>,
    /// `None`: all.
    category: RefCell<Option<usize>>,
    query: RefCell<String>,
    session: Rc<SessionConfig>,
    /// Keeps the menu alive while it is open.
    this: RefCell<Option<Rc<Menu>>>,
}

impl Menu {
    fn new(button: &gtk::Button, edge: Edge, session: Rc<SessionConfig>) -> Rc<Self> {
        let window = gtk::Window::builder()
            .css_classes(["orbit", "orbit-menu"])
            .decorated(false)
            .build();

        let search = gtk::SearchEntry::builder()
            .placeholder_text(tr("menu-search"))
            .hexpand(true)
            .build();

        let categories = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["categories"])
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Browse)
            .activate_on_single_click(true)
            .css_classes(["apps"])
            .build();
        let apps = collect(&list);
        let hint = gtk::Label::builder()
            .wrap(true)
            .css_classes(["hint"])
            .build();
        list.set_placeholder(Some(&hint));

        // "All", then the categories that have something.
        let mut used = vec![false; CATEGORIES.len()];
        for app in apps.values() {
            used[app.category] = true;
        }
        let mut category_ids = vec![None];
        categories.append(&category_row("view-app-grid-symbolic", &tr("menu-category-all")));
        for (i, (id, icon, _)) in CATEGORIES.iter().enumerate() {
            if used[i] {
                categories.append(&category_row(icon, &tr(id)));
                category_ids.push(Some(i));
            }
        }

        let user = gtk::Box::builder().spacing(10).hexpand(true).css_classes(["user"]).build();
        user.append(&gtk::Image::builder().icon_name("avatar-default-symbolic").pixel_size(20).build());
        let name = glib::real_name();
        let name = name.to_string_lossy();
        let name = if name.is_empty() || name == "Unknown" {
            glib::user_name().to_string_lossy().into_owned()
        } else {
            name.into_owned()
        };
        user.append(&gtk::Label::new(Some(&name)));
        let power = gtk::Button::builder()
            .icon_name("system-shutdown-symbolic")
            .tooltip_text(tr("menu-power"))
            .css_classes(["power"])
            .build();

        let left = gtk::ScrolledWindow::builder()
            .child(&categories)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(180)
            .build();
        let right = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();
        let middle = gtk::Box::builder().spacing(10).vexpand(true).build();
        middle.append(&left);
        middle.append(&right);
        let bottom = gtk::Box::builder().spacing(10).css_classes(["bottom"]).build();
        bottom.append(&user);
        bottom.append(&power);
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .width_request(WIDTH)
            .height_request(HEIGHT)
            .css_classes(["launcher"])
            .build();
        card.append(&search);
        card.append(&middle);
        card.append(&bottom);

        // Outside the menu, a click closes it.
        let backdrop = gtk::Box::builder().hexpand(true).vexpand(true).build();
        let overlay = gtk::Overlay::builder().child(&backdrop).build();
        overlay.add_overlay(&card);
        window.set_child(Some(&overlay));
        place(&window, &card, button, edge);

        let menu = Rc::new(Self {
            window,
            search,
            list,
            hint,
            apps,
            category: RefCell::new(None),
            query: RefCell::default(),
            session,
            this: RefCell::default(),
        });
        *menu.this.borrow_mut() = Some(menu.clone());
        menu.connect(&categories, category_ids, &backdrop, &power);
        menu
    }

    fn connect(
        self: &Rc<Self>,
        categories: &gtk::ListBox,
        ids: Vec<Option<usize>>,
        backdrop: &gtk::Box,
        power: &gtk::Button,
    ) {
        let weak = Rc::downgrade(self);
        self.list.set_filter_func(move |row| {
            weak.upgrade().is_some_and(|m| m.shows(row))
        });
        let weak = Rc::downgrade(self);
        self.list.set_sort_func(move |a, b| match weak.upgrade() {
            Some(m) => m.order(a, b),
            None => gtk::Ordering::Equal,
        });
        let weak = Rc::downgrade(self);
        self.list.connect_row_activated(move |_, row| {
            if let Some(m) = weak.upgrade() {
                m.launch(row);
            }
        });

        // Picking a category — or just pointing at it, as in the Whisker
        // menu — shows its applications.
        let weak = Rc::downgrade(self);
        categories.connect_row_selected(move |_, row| {
            let (Some(m), Some(row)) = (weak.upgrade(), row) else { return };
            let id = ids.get(row.index() as usize).copied().flatten();
            *m.category.borrow_mut() = id;
            if !m.search.text().is_empty() {
                m.search.set_text("");
            }
            m.refresh();
        });
        let mut child = categories.first_child();
        while let Some(row) = child {
            child = row.next_sibling();
            let Ok(row) = row.downcast::<gtk::ListBoxRow>() else { continue };
            let hover = gtk::EventControllerMotion::new();
            let categories = categories.clone();
            let target = row.clone();
            hover.connect_enter(move |_, _, _| categories.select_row(Some(&target)));
            row.add_controller(hover);
        }
        categories.select_row(categories.row_at_index(0).as_ref());

        let weak = Rc::downgrade(self);
        self.search.connect_search_changed(move |entry| {
            if let Some(m) = weak.upgrade() {
                *m.query.borrow_mut() = fold(&entry.text());
                let typed = entry.text();
                m.hint.set_label(&if typed.trim().is_empty() {
                    String::new()
                } else {
                    tr_args("menu-run", &[("command", text(typed.trim()))])
                });
                entry.remove_css_class("error");
                m.refresh();
            }
        });
        let weak = Rc::downgrade(self);
        self.search.connect_activate(move |entry| {
            let Some(m) = weak.upgrade() else { return };
            match m.first_shown() {
                Some(row) => m.launch(&row),
                None => m.run(&entry.text()),
            }
        });
        // Down from the search goes into the list.
        let weak = Rc::downgrade(self);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(m) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if key == gdk::Key::Down
                && let Some(row) = m.first_shown()
            {
                m.list.select_row(Some(&row));
                row.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.search.add_controller(keys);
        // Typing anywhere in the menu goes to the search.
        self.search.set_key_capture_widget(Some(&self.window));

        // The search entry would take Escape to clear itself: ahead of it.
        let weak = Rc::downgrade(self);
        let escape = gtk::EventControllerKey::new();
        escape.set_propagation_phase(gtk::PropagationPhase::Capture);
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape
                && let Some(m) = weak.upgrade()
            {
                m.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(escape);

        let weak = Rc::downgrade(self);
        let click = gtk::GestureClick::new();
        click.connect_released(move |_, _, _, _| {
            if let Some(m) = weak.upgrade() {
                m.close();
            }
        });
        backdrop.add_controller(click);

        let weak = Rc::downgrade(self);
        power.connect_clicked(move |_| {
            if let Some(m) = weak.upgrade() {
                let session = m.session.clone();
                m.close();
                // Once the menu is off the screen: the blurred backdrop is
                // a photo of it.
                glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
                    session::choose(session);
                });
            }
        });
        GtkWindowExt::set_focus(&self.window, Some(&self.search));
    }

    fn shows(&self, row: &gtk::ListBoxRow) -> bool {
        let Some(app) = self.apps.get(row) else {
            return false;
        };
        let query = self.query.borrow();
        if query.is_empty() {
            return self.category.borrow().is_none_or(|c| c == app.category);
        }
        // Searching looks through every category.
        query.split_whitespace().all(|word| app.haystack.contains(word))
    }

    /// Searching, names that start with the query first; otherwise by name.
    fn order(&self, a: &gtk::ListBoxRow, b: &gtk::ListBoxRow) -> gtk::Ordering {
        let (Some(a), Some(b)) = (self.apps.get(a), self.apps.get(b)) else {
            return gtk::Ordering::Equal;
        };
        let query = self.query.borrow();
        let rank = |app: &App| {
            if query.is_empty() || app.folded_name.starts_with(query.as_str()) {
                0
            } else if app.folded_name.contains(query.as_str()) {
                1
            } else {
                2
            }
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.folded_name.cmp(&b.folded_name))
            .into()
    }

    fn refresh(&self) {
        self.list.invalidate_filter();
        self.list.invalidate_sort();
        match self.first_shown() {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
    }

    fn first_shown(&self) -> Option<gtk::ListBoxRow> {
        let mut index = 0;
        while let Some(row) = self.list.row_at_index(index) {
            if row.is_child_visible() {
                return Some(row);
            }
            index += 1;
        }
        None
    }

    fn launch(&self, row: &gtk::ListBoxRow) {
        let Some(app) = self.apps.get(row) else { return };
        let context = WidgetExt::display(&self.window).app_launch_context();
        if let Err(err) = app.info.launch(&[], Some(&context)) {
            log::error!("menu: starting {}: {err}", app.info.name());
        }
        self.close();
    }

    /// Runs what was typed, when no application matches it.
    fn run(&self, command: &str) {
        let command = command.trim();
        if command.is_empty() {
            return;
        }
        if let Err(err) = glib::spawn_command_line_async(command) {
            log::warn!("menu: running {command:?}: {err}");
            self.search.add_css_class("error");
            return;
        }
        self.close();
    }

    fn close(&self) {
        self.window.destroy();
        // Last: this may drop the menu.
        self.this.borrow_mut().take();
    }
}

/// Every application meant for menus, a row each.
fn collect(list: &gtk::ListBox) -> HashMap<gtk::ListBoxRow, App> {
    let mut apps = HashMap::new();
    for info in gio::AppInfo::all() {
        if !info.should_show() {
            continue;
        }
        let Ok(info) = info.downcast::<gio_unix::DesktopAppInfo>() else {
            continue;
        };
        let name = info.display_name().to_string();
        let mut haystack = vec![name.clone()];
        haystack.extend(info.generic_name().map(|g| g.to_string()));
        haystack.extend(info.keywords().iter().map(|k| k.to_string()));
        if let Some(program) = info.executable().file_name() {
            haystack.push(program.to_string_lossy().into_owned());
        }
        let category = category_of(info.categories().as_deref().unwrap_or(""));

        let row = gtk::ListBoxRow::new();
        let content = gtk::Box::builder().spacing(10).build();
        let icon = gtk::Image::builder().pixel_size(30).build();
        match info.icon() {
            Some(gicon) => icon.set_from_gicon(&gicon),
            None => icon.set_icon_name(Some("application-x-executable")),
        }
        content.append(&icon);
        let labels = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .build();
        labels.append(
            &gtk::Label::builder()
                .label(name.as_str())
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["name"])
                .build(),
        );
        if let Some(comment) = info.description().filter(|c| !c.is_empty()) {
            labels.append(
                &gtk::Label::builder()
                    .label(comment.as_str())
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["comment"])
                    .build(),
            );
        }
        content.append(&labels);
        row.set_child(Some(&content));
        row.set_tooltip_text(info.description().as_deref());
        list.append(&row);
        apps.insert(
            row,
            App {
                info,
                haystack: fold(&haystack.join(" ")),
                folded_name: fold(&name),
                category,
            },
        );
    }
    apps
}

fn category_of(categories: &str) -> usize {
    let names: Vec<&str> = categories.split(';').collect();
    CATEGORIES
        .iter()
        .position(|(_, _, keys)| keys.iter().any(|k| names.contains(k)))
        .unwrap_or(CATEGORIES.len() - 1)
}

fn category_row(icon: &str, label: &str) -> gtk::ListBoxRow {
    let content = gtk::Box::builder().spacing(10).build();
    content.append(&gtk::Image::builder().icon_name(icon).pixel_size(20).build());
    content.append(&gtk::Label::builder().label(label).xalign(0.0).build());
    gtk::ListBoxRow::builder().child(&content).build()
}

/// Lower case, without accents: "Câmera" finds "camera".
fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            other => other,
        })
        .collect()
}

/// Covers `button`'s monitor, less the panel, and puts the menu beside the
/// button: down from it on a vertical panel, along it on a horizontal one.
fn place(window: &gtk::Window, card: &gtk::Box, button: &gtk::Button, edge: Edge) {
    let display = button.display();
    let monitor = button
        .native()
        .and_then(|n| n.surface())
        .and_then(|s| display.monitor_at_surface(&s));
    let screen = monitor.as_ref().map(|m| m.geometry());
    let at = button
        .root()
        .and_then(|root| button.compute_point(&root, &gtk::graphene::Point::new(0.0, 0.0)))
        .map(|p| (p.x() as i32, p.y() as i32))
        .unwrap_or((0, 0));
    let along = |offset: i32, length: Option<i32>, size: i32| {
        let most = length.map_or(offset, |l| (l - size - GAP).max(GAP));
        offset.clamp(GAP, most.max(GAP))
    };
    match edge {
        Edge::Left | Edge::Right => {
            card.set_halign(if edge == Edge::Left { gtk::Align::Start } else { gtk::Align::End });
            card.set_valign(gtk::Align::Start);
            card.set_margin_top(along(at.1, screen.map(|g| g.height()), HEIGHT));
            card.set_margin_start(GAP);
            card.set_margin_end(GAP);
        }
        Edge::Top | Edge::Bottom => {
            card.set_valign(if edge == Edge::Top { gtk::Align::Start } else { gtk::Align::End });
            card.set_halign(gtk::Align::Start);
            card.set_margin_start(along(at.0, screen.map(|g| g.width()), WIDTH));
            card.set_margin_top(GAP);
            card.set_margin_bottom(GAP);
        }
    }

    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Top);
        window.set_namespace(Some("orbit-menu"));
        window.set_monitor(monitor.as_ref());
        for side in [LayerEdge::Top, LayerEdge::Bottom, LayerEdge::Left, LayerEdge::Right] {
            window.set_anchor(side, true);
        }
        // Clear of the panel and the dock, which stay clickable.
        window.set_exclusive_zone(0);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
    } else {
        window.set_default_size(WIDTH, HEIGHT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_accents_and_case() {
        assert_eq!(fold("Câmera Ação"), "camera acao");
    }

    #[test]
    fn categories_take_the_first_match() {
        let other = CATEGORIES.len() - 1;
        assert_eq!(CATEGORIES[category_of("Graphics;Viewer;")].0, "menu-category-graphics");
        assert_eq!(CATEGORIES[category_of("GTK;Network;WebBrowser;")].0, "menu-category-internet");
        assert_eq!(category_of("Viewer;"), other);
        assert_eq!(category_of(""), other);
    }
}
