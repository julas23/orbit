//! The pages of Orbit-Settings: their groups, their order, what the search
//! finds in each, and the sidebar that shows them (docs/PLANO-SETTINGS.md
//! §1–§2).
//!
//! A page's address is its id: `orbit-settings services` opens it (the
//! panel's modules, Orbit-Run and the other applications use it). The
//! search looks through every page's title, its keywords and the labels of
//! the options it holds, in the user's language, with or without accents.

use gtk::prelude::*;
use orbit_app::i18n::tr;

/// The sidebar's groups, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    System,
    People,
    Orbit,
}

impl Group {
    const ALL: [Group; 3] = [Group::System, Group::People, Group::Orbit];

    fn title(self) -> String {
        tr(match self {
            Group::System => "settings-group-system",
            Group::People => "settings-group-people",
            Group::Orbit => "settings-group-orbit",
        })
    }
}

/// A page of the window.
pub struct Page {
    pub id: &'static str,
    pub group: Group,
    /// Its title's message.
    pub title: &'static str,
    pub icon: &'static str,
    /// More words that find it: a message of comma-separated words.
    pub keywords: &'static str,
    /// The messages of the options it holds: what the search finds in it.
    pub options: &'static [&'static str],
}

/// Every page, in the sidebar's order within its group.
pub const PAGES: &[Page] = &[
    Page {
        id: "network",
        group: Group::System,
        title: "settings-network",
        icon: "network-workgroup-symbolic",
        keywords: "settings-network-keywords",
        options: &[
            "settings-net-wifi",
            "settings-net-wired",
            "settings-net-hidden",
            "settings-net-advanced",
            "settings-net-vpn",
            "settings-net-import",
            "settings-net-paste",
        ],
    },
    Page {
        id: "bluetooth",
        group: Group::System,
        title: "settings-bt",
        icon: "bluetooth-symbolic",
        keywords: "settings-bt-keywords",
        options: &["settings-bt-visible", "settings-bt-mine", "settings-bt-nearby", "settings-bt-pair"],
    },
    Page {
        id: "sound",
        group: Group::System,
        title: "settings-audio",
        icon: "audio-speakers-symbolic",
        keywords: "settings-audio-keywords",
        options: &["settings-sound-output", "settings-sound-input", "settings-sound-apps", "settings-sound-cards"],
    },
    Page {
        id: "power",
        group: Group::System,
        title: "settings-power",
        icon: "battery-good-symbolic",
        keywords: "settings-power-keywords",
        options: &[
            "settings-power-profile",
            "settings-power-lock-after",
            "settings-power-suspend-mains",
            "settings-power-suspend-battery",
            "settings-power-effects",
            "settings-power-battery",
            "settings-power-health",
            "settings-power-key",
            "settings-power-lid-battery",
        ],
    },
    Page {
        id: "keyboard",
        group: Group::System,
        title: "settings-kb",
        icon: "input-keyboard-symbolic",
        keywords: "settings-kb-keywords",
        options: &["settings-kb-layouts", "settings-kb-switch", "settings-kb-compose", "settings-kb-delay", "settings-kb-rate"],
    },
    Page {
        id: "mouse",
        group: Group::System,
        title: "settings-mouse-page",
        icon: "input-mouse-symbolic",
        keywords: "settings-mouse-keywords",
        options: &[
            "settings-mouse-speed",
            "settings-mouse-natural",
            "settings-mouse-left",
            "settings-touchpad-tap",
            "settings-touchpad-typing",
            "settings-touchpad-gestures",
        ],
    },
    Page {
        id: "monitors",
        group: Group::System,
        title: "settings-monitors",
        icon: "video-display-symbolic",
        keywords: "settings-monitors-keywords",
        options: &[
            "settings-resolution",
            "settings-refresh",
            "settings-scale",
            "settings-rotation",
            "settings-off",
        ],
    },
    Page {
        id: "users",
        group: Group::People,
        title: "settings-users",
        icon: "system-users-symbolic",
        keywords: "settings-users-keywords",
        options: &["settings-users-password", "settings-users-picture", "settings-users-add", "settings-users-admin", "settings-users-groups", "settings-users-boot"],
    },
    Page {
        id: "lock",
        group: Group::People,
        title: "settings-lock",
        icon: "system-lock-screen-symbolic",
        keywords: "settings-lock-keywords",
        options: &["settings-power-lock-after", "settings-lock-screen-off", "settings-lock-people", "settings-lock-now"],
    },
    Page {
        id: "privacy",
        group: Group::People,
        title: "settings-privacy",
        icon: "preferences-system-privacy-symbolic",
        keywords: "settings-privacy-keywords",
        options: &["settings-privacy-remember", "settings-privacy-clear-recent", "settings-privacy-old-trash", "settings-privacy-empty-trash", "settings-privacy-apps"],
    },
    Page {
        id: "datetime",
        group: Group::People,
        title: "settings-time",
        icon: "preferences-system-time-symbolic",
        keywords: "settings-time-keywords",
        options: &["settings-time-zone", "settings-time-ntp"],
    },
    Page {
        id: "region",
        group: Group::People,
        title: "settings-region",
        icon: "preferences-desktop-locale-symbolic",
        keywords: "settings-region-keywords",
        options: &["settings-region-language", "settings-region-formats"],
    },
    Page {
        id: "appearance",
        group: Group::Orbit,
        title: "settings-look",
        icon: "preferences-desktop-appearance-symbolic",
        keywords: "settings-look-keywords",
        options: &[
            "settings-look-scheme",
            "settings-look-accent",
            "settings-look-theme",
            "settings-look-icons",
            "settings-look-cursor",
            "settings-look-fonts",
            "settings-look-text-scale",
        ],
    },
    Page {
        id: "wallpaper",
        group: Group::Orbit,
        title: "settings-wallpaper",
        icon: "preferences-desktop-wallpaper-symbolic",
        keywords: "settings-wallpaper-keywords",
        options: &[
            "settings-show",
            "settings-mode",
            "settings-color",
            "settings-effect",
            "settings-colors",
            "settings-speed",
            "settings-sound",
            "settings-saver",
        ],
    },
    Page {
        id: "panel",
        group: Group::Orbit,
        title: "settings-panel",
        icon: "view-dual-symbolic",
        keywords: "settings-panel-keywords",
        options: &["settings-panel-edge", "settings-panel-monitor", "settings-panel-reserve", "settings-panel-modules"],
    },
    Page {
        id: "dock",
        group: Group::Orbit,
        title: "settings-dock",
        icon: "user-bookmarks-symbolic",
        keywords: "settings-dock-keywords",
        options: &["settings-dock-launchers", "settings-dock-add", "settings-dock-icon-size", "settings-dock-align"],
    },
    Page {
        id: "apps",
        group: Group::Orbit,
        title: "settings-apps",
        icon: "preferences-desktop-default-applications-symbolic",
        keywords: "settings-apps-keywords",
        options: &["settings-apps-browser", "settings-apps-mail", "settings-apps-files", "settings-apps-other", "settings-apps-terminal", "settings-apps-login"],
    },
    Page {
        id: "notifications",
        group: Group::Orbit,
        title: "settings-notify",
        icon: "preferences-system-notifications-symbolic",
        keywords: "settings-notify-keywords",
        options: &["settings-notify-dnd", "settings-notify-bubbles", "settings-notify-history", "settings-notify-never"],
    },
    Page {
        id: "windows",
        group: Group::Orbit,
        title: "settings-win",
        icon: "preferences-system-windows-symbolic",
        keywords: "settings-win-keywords",
        options: &[
            "settings-win-layout",
            "settings-win-gaps",
            "settings-win-border",
            "settings-win-corners",
            "settings-win-focused",
            "settings-win-shadows",
            "settings-win-animations",
            "settings-win-focus-follows",
            "settings-win-areas",
            "settings-win-rules",
        ],
    },
    Page {
        id: "shortcuts",
        group: Group::Orbit,
        title: "settings-keys",
        icon: "preferences-desktop-keyboard-shortcuts-symbolic",
        keywords: "settings-keys-keywords",
        options: &["settings-keys-mod", "settings-keys-add"],
    },
    Page {
        id: "services",
        group: Group::Orbit,
        title: "settings-services",
        icon: "network-server-symbolic",
        keywords: "settings-services-keywords",
        options: &["settings-svc-firewall", "settings-svc-remote", "settings-svc-open-files"],
    },
    Page {
        id: "about",
        group: Group::Orbit,
        title: "settings-about",
        icon: "help-about-symbolic",
        keywords: "settings-about-keywords",
        options: &[
            "settings-about-name",
            "settings-about-cpu",
            "settings-about-memory",
            "settings-about-gpu",
            "settings-about-disk",
            "settings-about-kernel",
            "settings-about-copy",
        ],
    },
];

/// The page an address names: `services`, or `services/firewall` (the part
/// after the slash is the section, for the pages that have them).
pub fn find(address: &str) -> Option<&'static Page> {
    let id = address.split('/').next().unwrap_or(address);
    PAGES.iter().find(|p| p.id == id)
}

/// Lowercase and without accents, for the search: "Configurações" and
/// "configuracoes" meet.
pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// What the search finds for `query` (already folded): the page, and the
/// option's label when an option matched rather than the page itself.
fn search(query: &str) -> Vec<(&'static Page, Option<String>)> {
    let mut found = Vec::new();
    for page in PAGES {
        let words = tr(page.keywords);
        if fold(&tr(page.title)).contains(query) || words.split(',').any(|w| fold(w.trim()).contains(query)) {
            found.push((page, None));
        }
    }
    for page in PAGES {
        for option in page.options {
            let label = tr(option);
            if fold(&label).contains(query) {
                found.push((page, Some(label)));
            }
        }
    }
    found
}

/// The sidebar: the search over the pages, in their groups; a row chosen
/// shows its page in `stack`.
pub fn sidebar(stack: &gtk::Stack) -> (gtk::Box, gtk::SearchEntry) {
    let entry = gtk::SearchEntry::builder()
        .placeholder_text(tr("settings-search"))
        .css_classes(["search"])
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["pages"])
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let side = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(5)
        .css_classes(["sidebar"])
        .build();
    side.append(&entry);
    side.append(&scroll);

    fill(&list, "", stack);
    entry.connect_search_changed({
        let list = list.clone();
        let stack = stack.clone();
        move |entry| fill(&list, &fold(entry.text().trim()), &stack)
    });
    // Enter in the search opens the first thing it found.
    entry.connect_activate({
        let list = list.clone();
        move |_| {
            let mut i = 0;
            while let Some(row) = list.row_at_index(i) {
                if row.is_activatable() {
                    row.activate();
                    return;
                }
                i += 1;
            }
        }
    });
    list.connect_row_activated({
        let stack = stack.clone();
        move |_, row| {
            let name = row.widget_name();
            if !name.is_empty() {
                stack.set_visible_child_name(&name);
            }
        }
    });
    // The page shown, chosen in the list.
    stack.connect_visible_child_name_notify({
        let list = list.clone();
        move |stack| select(&list, stack.visible_child_name().as_deref().unwrap_or_default())
    });
    (side, entry)
}

/// The list's rows: every page in its group, or what `query` finds.
fn fill(list: &gtk::ListBox, query: &str, stack: &gtk::Stack) {
    list.remove_all();
    if query.is_empty() {
        for group in Group::ALL {
            let pages: Vec<&Page> = PAGES.iter().filter(|p| p.group == group).collect();
            if pages.is_empty() {
                continue;
            }
            list.append(&heading(&group.title()));
            for page in pages {
                list.append(&page_row(page, None));
            }
        }
    } else {
        let found = search(query);
        if found.is_empty() {
            list.append(&heading(&tr("settings-search-none")));
        }
        for (page, option) in found {
            list.append(&page_row(page, option.as_deref()));
        }
    }
    select(list, stack.visible_child_name().as_deref().unwrap_or_default());
}

fn heading(text: &str) -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["group"])
        .build();
    gtk::ListBoxRow::builder()
        .child(&label)
        .selectable(false)
        .activatable(false)
        .build()
}

/// A page's row; for a found option, the option's label over the page's.
fn page_row(page: &Page, option: Option<&str>) -> gtk::ListBoxRow {
    let line = gtk::Box::builder().spacing(10).build();
    line.append(&gtk::Image::from_icon_name(page.icon));
    let text = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
    match option {
        Some(option) => {
            text.append(&gtk::Label::builder().label(option).xalign(0.0).build());
            text.append(
                &gtk::Label::builder()
                    .label(tr(page.title))
                    .xalign(0.0)
                    .css_classes(["dim"])
                    .build(),
            );
        }
        None => text.append(&gtk::Label::builder().label(tr(page.title)).xalign(0.0).build()),
    }
    line.append(&text);
    let row = gtk::ListBoxRow::builder().child(&line).build();
    row.set_widget_name(page.id);
    row
}

/// Selects the first row of page `id`, without activating it.
fn select(list: &gtk::ListBox, id: &str) {
    let mut i = 0;
    while let Some(row) = list.row_at_index(i) {
        if row.widget_name() == id {
            list.select_row(Some(&row));
            return;
        }
        i += 1;
    }
    list.select_row(None::<&gtk::ListBoxRow>);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding() {
        assert_eq!(fold("Configurações"), "configuracoes");
        assert_eq!(fold("RESOLUÇÃO"), "resolucao");
    }

    #[test]
    fn addresses() {
        assert_eq!(find("services").map(|p| p.id), Some("services"));
        assert_eq!(find("services/firewall").map(|p| p.id), Some("services"));
        assert!(find("nowhere").is_none());
    }

    #[test]
    fn ids_are_unique() {
        for (i, a) in PAGES.iter().enumerate() {
            assert!(PAGES[i + 1..].iter().all(|b| b.id != a.id), "{} twice", a.id);
        }
    }
}
