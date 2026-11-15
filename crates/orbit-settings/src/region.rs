//! The Region and language page (docs/PLANO-SETTINGS.md §3.2), through
//! localed: the system's language (LANG) and its formats (dates, numbers,
//! money, measures, paper: the LC_* the user picks, or the language's). A
//! change asks for an administrator's password once (polkit) and takes
//! effect at the next login. Only the languages this machine has generated
//! are offered (`localectl list-locales`).

use std::{collections::BTreeMap, process::Command, rc::Rc};

use gtk::{glib, prelude::*};
use orbit_app::i18n::tr;

use crate::dbus;

/// The format categories a "Formats" choice sets.
const FORMATS: [&str; 5] = ["LC_TIME", "LC_NUMERIC", "LC_MONETARY", "LC_MEASUREMENT", "LC_PAPER"];

fn locales() -> Vec<String> {
    Command::new("localectl")
        .arg("list-locales")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// A locale as people read it, in its own language for the common ones.
fn display(locale: &str) -> String {
    let code = locale.split('.').next().unwrap_or(locale);
    let name = match code {
        "C" => return format!("C ({})", tr("settings-region-no-language")),
        "en_US" => "English (United States)",
        "en_GB" => "English (United Kingdom)",
        "en_AU" => "English (Australia)",
        "en_CA" => "English (Canada)",
        "pt_BR" => "Português (Brasil)",
        "pt_PT" => "Português (Portugal)",
        "es_ES" => "Español (España)",
        "es_MX" => "Español (México)",
        "es_AR" => "Español (Argentina)",
        "fr_FR" => "Français (France)",
        "de_DE" => "Deutsch (Deutschland)",
        "it_IT" => "Italiano (Italia)",
        "nl_NL" => "Nederlands (Nederland)",
        "pl_PL" => "Polski (Polska)",
        "ru_RU" => "Русский (Россия)",
        "ja_JP" => "日本語 (日本)",
        "zh_CN" => "中文 (中国)",
        "ko_KR" => "한국어 (대한민국)",
        _ => return locale.to_owned(),
    };
    format!("{name} — {locale}")
}

/// localed's `Locale` (`KEY=value` entries) as a map.
fn parse(entries: &[String]) -> BTreeMap<String, String> {
    entries
        .iter()
        .filter_map(|e| e.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
        .collect()
}

/// The entries SetLocale takes for `lang` and, unless the same, `formats`.
fn entries(lang: &str, formats: Option<&str>) -> Vec<String> {
    let mut out = vec![format!("LANG={lang}")];
    if let Some(f) = formats.filter(|f| *f != lang) {
        out.extend(FORMATS.iter().map(|k| format!("{k}={f}")));
    }
    out
}

struct Page {
    proxy: gtk::gio::DBusProxy,
    list: Vec<String>,
    language: gtk::DropDown,
    formats: gtk::DropDown,
    message: gtk::Label,
}

pub fn page() -> gtk::Widget {
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(20)
        .css_classes(["page"])
        .build();
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["card"])
        .build();
    body.append(&card);
    let message = crate::message();
    let Some(proxy) = dbus::system("org.freedesktop.locale1") else {
        crate::say(&message, &tr("settings-region-unavailable"));
        card.append(&message);
        return body.upcast();
    };
    let list = locales();
    let names: Vec<String> = list.iter().map(|l| display(l)).collect();
    let language = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let mut format_names = vec![tr("settings-region-same")];
    format_names.extend(names.iter().cloned());
    let formats = gtk::DropDown::from_strings(&format_names.iter().map(String::as_str).collect::<Vec<_>>());

    let page = Rc::new(Page {
        proxy,
        list,
        language,
        formats,
        message,
    });
    page.show();
    card.append(&crate::row(&tr("settings-region-language"), &page.language));
    let formats_row = crate::row(&tr("settings-region-formats"), &page.formats);
    formats_row.set_tooltip_text(Some(&tr("settings-region-formats-tip")));
    card.append(&formats_row);
    card.append(
        &gtk::Label::builder()
            .label(tr("settings-region-next-login"))
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim"])
            .build(),
    );
    card.append(&page.message);

    for dropdown in [&page.language, &page.formats] {
        dropdown.connect_selected_notify({
            let page = page.clone();
            move |_| page.apply()
        });
    }
    gtk::ScrolledWindow::builder()
        .child(&body)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
        .upcast()
}

impl Page {
    /// The language and formats localed has now, in the dropdowns, without
    /// applying them again.
    fn show(&self) {
        let now = parse(&dbus::strings(&self.proxy, "Locale"));
        let lang = now.get("LANG").cloned().unwrap_or_default();
        let index = |l: &str| self.list.iter().position(|x| x == l).map(|i| i as u32);
        if let Some(i) = index(&lang) {
            self.language.set_selected(i);
        }
        let formats = now.get("LC_TIME").filter(|f| **f != lang).and_then(|f| index(f));
        self.formats.set_selected(formats.map_or(0, |i| i + 1));
    }

    fn apply(self: &Rc<Self>) {
        let Some(lang) = self.list.get(self.language.selected() as usize).cloned() else { return };
        let formats = match self.formats.selected() {
            0 => None,
            i => self.list.get(i as usize - 1).cloned(),
        };
        let now = parse(&dbus::strings(&self.proxy, "Locale"));
        let wanted = entries(&lang, formats.as_deref());
        if parse(&wanted) == now {
            return;
        }
        let page = self.clone();
        glib::spawn_future_local(async move {
            let result = dbus::call(&page.proxy, "SetLocale", (wanted, true).to_variant()).await;
            match result {
                Ok(_) => crate::say(&page.message, ""),
                Err(e) => crate::say(&page.message, &e),
            }
            // What localed has, whatever happened (the property is cached
            // until its change signal: read it again a moment later).
            glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                if let Some(proxy) = dbus::system("org.freedesktop.locale1") {
                    let fresh = Page {
                        proxy,
                        list: page.list.clone(),
                        language: page.language.clone(),
                        formats: page.formats.clone(),
                        message: page.message.clone(),
                    };
                    fresh.show();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_for_localed() {
        assert_eq!(entries("pt_BR.UTF-8", None), ["LANG=pt_BR.UTF-8"]);
        assert_eq!(entries("en_US.UTF-8", Some("en_US.UTF-8")), ["LANG=en_US.UTF-8"]);
        let mixed = entries("en_US.UTF-8", Some("pt_BR.UTF-8"));
        assert_eq!(mixed.len(), 6);
        assert!(mixed.contains(&"LC_TIME=pt_BR.UTF-8".to_owned()));
    }

    #[test]
    fn locales_read() {
        assert!(display("pt_BR.UTF-8").starts_with("Português (Brasil)"));
        assert_eq!(display("xx_YY.UTF-8"), "xx_YY.UTF-8");
        let map = parse(&["LANG=pt_BR.UTF-8".to_owned(), "LC_TIME=en_GB.UTF-8".to_owned()]);
        assert_eq!(map.get("LC_TIME").map(String::as_str), Some("en_GB.UTF-8"));
    }
}
