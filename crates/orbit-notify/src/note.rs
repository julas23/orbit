//! A notification, as a program sends it (`Notify` on the bus), and the
//! history kept of them.
//!
//! What the history keeps is the text, the icon and where it came from; the
//! actions and the sender's bus name belong to the running program, and
//! are gone after a restart — an old notification can only open its app.

use std::path::PathBuf;

use gtk::{gdk, glib};
use serde::{Deserialize, Serialize};

/// Bitmaps sent in the hints are kept at this size at most: a contact's
/// photo can come at 1000 px.
const IMAGE_MAX: i32 = 96;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: u32,
    /// The sender's name for itself ("Firefox", "notify-send").
    pub app: String,
    /// An icon name or a file (path or `file://`).
    pub icon: String,
    pub summary: String,
    /// Pango markup, already checked (see [`body_markup`]).
    pub body: String,
    /// The sender's `.desktop` file, without the extension.
    pub desktop: String,
    /// 0 low, 1 normal, 2 critical.
    pub urgency: u8,
    /// Unix time, seconds.
    pub time: i64,
    /// Seen in the centre.
    pub seen: bool,

    /// (key, label) pairs; "default" is the click on the notification.
    #[serde(skip)]
    pub actions: Vec<(String, String)>,
    #[serde(skip)]
    pub image: Option<gdk::Texture>,
    /// The bus name it came from, while that program is still there to
    /// hear of clicks.
    #[serde(skip)]
    pub sender: Option<String>,
    /// The bubble's time, ms, as the sender asked (-1: ours; 0: forever).
    #[serde(skip)]
    pub expire: i32,
    /// Bubble only: not kept in the centre.
    #[serde(skip)]
    pub transient: bool,
    /// Stays after an action is invoked.
    #[serde(skip)]
    pub resident: bool,
    /// A progress value 0–100 (volume, brightness).
    #[serde(skip)]
    pub value: Option<i32>,
}

impl Note {
    /// Reads the arguments of `Notify`: `(susssasa{sv}i)`.
    pub fn from_call(id: u32, params: &glib::Variant, sender: Option<&str>) -> Note {
        let string = |i: usize| params.child_value(i).str().unwrap_or_default().to_owned();
        let list: Vec<String> = params.child_value(5).get().unwrap_or_default();
        let actions = list.chunks(2).filter(|p| p.len() == 2).map(|p| (p[0].clone(), p[1].clone())).collect();
        let hints = glib::VariantDict::new(Some(&params.child_value(6)));
        let hint_str = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| hints.lookup_value(k, None).and_then(|v| v.str().map(str::to_owned)))
                .unwrap_or_default()
        };
        let hint_bool = |key: &str| {
            hints.lookup_value(key, None).and_then(|v| v.get::<bool>()).unwrap_or(false)
        };
        let urgency = hints
            .lookup_value("urgency", None)
            .and_then(|v| v.get::<u8>().or_else(|| v.get::<u32>().map(|u| u as u8)).or_else(|| v.get::<i32>().map(|u| u as u8)))
            .unwrap_or(1)
            .min(2);
        let image = ["image-data", "image_data", "icon_data"]
            .iter()
            .find_map(|k| hints.lookup_value(k, None))
            .and_then(|v| texture(&v));
        // The image wins over the app's icon, a file image over a named one.
        let mut icon = hint_str(&["image-path", "image_path"]);
        if icon.is_empty() {
            icon = string(2);
        }
        let value = hints
            .lookup_value("value", None)
            .and_then(|v| v.get::<i32>().or_else(|| v.get::<u32>().map(|u| u as i32)))
            .map(|v| v.clamp(0, 100));
        Note {
            id,
            app: string(0),
            icon,
            summary: string(3),
            body: body_markup(&string(4)),
            desktop: hint_str(&["desktop-entry"]).trim_end_matches(".desktop").to_owned(),
            urgency,
            time: glib::real_time() / 1_000_000,
            seen: false,
            actions,
            image,
            sender: sender.map(str::to_owned),
            expire: params.child_value(7).get::<i32>().unwrap_or(-1),
            transient: hint_bool("transient"),
            resident: hint_bool("resident"),
            value,
        }
    }

    /// The group in the centre: the app's desktop file, or its name.
    pub fn group(&self) -> &str {
        if self.desktop.is_empty() { &self.app } else { &self.desktop }
    }

    /// Whether clicking it can tell the program.
    pub fn has_default(&self) -> bool {
        self.sender.is_some() && self.actions.iter().any(|(k, _)| k == "default")
    }

    /// The actions shown as buttons: all but the click.
    pub fn buttons(&self) -> impl Iterator<Item = &(String, String)> {
        self.actions.iter().filter(|(k, l)| k != "default" && !l.is_empty())
    }
}

/// The body as Pango markup. The specification allows `<b> <i> <u> <a
/// href> <img>`; GTK shows the first four. Images are dropped, line breaks
/// kept; a body that is not valid markup (plain text with a `&` or a `<`)
/// is shown as it came.
pub fn body_markup(body: &str) -> String {
    let mut text = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find('<') {
        text.push_str(&rest[..start]);
        let tag = &rest[start..];
        let lower = tag.get(..4).unwrap_or(tag).to_ascii_lowercase();
        if lower.starts_with("<img") || lower.starts_with("<br") {
            match tag.find('>') {
                Some(end) => {
                    if lower.starts_with("<br") {
                        text.push('\n');
                    }
                    rest = &tag[end + 1..];
                    continue;
                }
                None => {
                    text.push_str(tag);
                    rest = "";
                    break;
                }
            }
        }
        text.push('<');
        rest = &tag[1..];
    }
    text.push_str(rest);
    let text = text.trim().to_owned();
    // Links are GTK's, not Pango's: checked as spans.
    if gtk::pango::parse_markup(&links_as_spans(&text), '\0').is_ok() { text } else { glib::markup_escape_text(&text).to_string() }
}

fn links_as_spans(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<a ") {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => {
                out.push_str("<span>");
                rest = &rest[start + end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.replace("</a>", "</span>")
}

/// An `image-data` hint, `(iiibiiay)`, as a texture no larger than
/// [`IMAGE_MAX`].
fn texture(v: &glib::Variant) -> Option<gdk::Texture> {
    if v.n_children() < 7 {
        return None;
    }
    let width = v.child_value(0).get::<i32>()?;
    let height = v.child_value(1).get::<i32>()?;
    let stride = v.child_value(2).get::<i32>()?;
    let alpha = v.child_value(3).get::<bool>()?;
    let bits = v.child_value(4).get::<i32>()?;
    let bytes = glib::Bytes::from(v.child_value(6).data());
    if width <= 0 || height <= 0 || bits != 8 || bytes.len() < (stride * (height - 1) + width * if alpha { 4 } else { 3 }) as usize {
        return None;
    }
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::from_bytes(&bytes, gtk::gdk_pixbuf::Colorspace::Rgb, alpha, bits, width, height, stride);
    let side = width.max(height);
    let pixbuf = if side > IMAGE_MAX {
        let scale = IMAGE_MAX as f64 / side as f64;
        pixbuf.scale_simple(
            ((width as f64 * scale).round() as i32).max(1),
            ((height as f64 * scale).round() as i32).max(1),
            gtk::gdk_pixbuf::InterpType::Bilinear,
        )?
    } else {
        pixbuf
    };
    Some(gdk::Texture::for_pixbuf(&pixbuf))
}

/// `$XDG_STATE_HOME/orbit/notify/history.json`.
fn history_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| glib::home_dir().join(".local").join("state"))
        .join("orbit")
        .join("notify")
        .join("history.json")
}

/// The history and the do-not-disturb switch, as saved.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub dnd: bool,
    pub notes: Vec<Note>,
}

pub fn load() -> Saved {
    std::fs::read_to_string(history_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(saved: &Saved) {
    let path = history_path();
    let write = || -> std::io::Result<()> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string(saved).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        // Notifications can carry private text: readable by their owner only.
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(text.as_bytes())?;
        std::fs::rename(&tmp, &path)
    };
    if let Err(err) = write() {
        log::warn!("history: {}: {err}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::body_markup;

    #[test]
    fn markup() {
        assert_eq!(body_markup("<b>oi</b> tudo bem"), "<b>oi</b> tudo bem");
        assert_eq!(body_markup("a & b < c"), "a &amp; b &lt; c");
        assert_eq!(body_markup("foto <img src=\"x.png\" alt=\"x\"/> aqui"), "foto  aqui");
        assert_eq!(body_markup("linha<br/>outra"), "linha\noutra");
        assert_eq!(body_markup("<a href=\"https://x.org\">link</a>"), "<a href=\"https://x.org\">link</a>");
        assert_eq!(body_markup("eu <3 rust"), "eu &lt;3 rust");
    }
}
