//! What the browser keeps of the pages: the history (the browser's own
//! tabs only — the services are apps, not browsing), the bookmarks, and
//! the tabs closed this run (Ctrl+Shift+T). All of it local, in
//! `~/.local/share/orbit/pathfinder/` (history.json, bookmarks.json),
//! readable by its owner only; `history: false` in pathfinder.yaml keeps
//! none.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use gtk::glib;
use serde::{Deserialize, Serialize};

/// Pages kept in the history, at most: the oldest go first.
const MOST_HISTORY: usize = 10_000;
const MOST_CLOSED: usize = 25;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Visit {
    pub url: String,
    #[serde(default)]
    pub title: String,
    /// Seconds since 1970, of the last visit.
    pub time: i64,
    #[serde(default)]
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub time: i64,
}

pub struct Library {
    dir: PathBuf,
    keep_history: bool,
    history: RefCell<Vec<Visit>>,
    bookmarks: RefCell<Vec<Bookmark>>,
    closed: RefCell<Vec<String>>,
    save_soon: RefCell<Option<glib::SourceId>>,
    dirty: Cell<bool>,
}

fn now() -> i64 {
    glib::real_time() / 1_000_000
}

impl Library {
    pub fn load(dir: PathBuf, keep_history: bool) -> Rc<Self> {
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        let history = if keep_history {
            serde_json::from_str(&read("history.json")).unwrap_or_default()
        } else {
            let _ = std::fs::remove_file(dir.join("history.json"));
            Vec::new()
        };
        let bookmarks = serde_json::from_str(&read("bookmarks.json")).unwrap_or_default();
        Rc::new(Self {
            dir,
            keep_history,
            history: RefCell::new(history),
            bookmarks: RefCell::new(bookmarks),
            closed: RefCell::default(),
            save_soon: RefCell::default(),
            dirty: Cell::new(false),
        })
    }

    /// A page of the browser was shown (`title` may come later: then the
    /// same visit gets it).
    pub fn visited(self: &Rc<Self>, url: &str, title: &str) {
        if !self.keep_history || !(url.starts_with("http://") || url.starts_with("https://")) {
            return;
        }
        {
            let mut history = self.history.borrow_mut();
            let time = now();
            match history.iter().position(|v| v.url == url) {
                Some(i) => {
                    let mut v = history.remove(i);
                    // The title arriving after the address is the same visit.
                    if time - v.time > 2 || title.is_empty() {
                        v.count += 1;
                    }
                    v.time = time;
                    if !title.is_empty() {
                        v.title = title.to_string();
                    }
                    history.push(v);
                }
                None => history.push(Visit {
                    url: url.to_string(),
                    title: title.to_string(),
                    time,
                    count: 1,
                }),
            }
            let len = history.len();
            if len > MOST_HISTORY {
                history.drain(..len - MOST_HISTORY);
            }
        }
        self.save_later();
    }

    /// Newest first.
    pub fn history(&self) -> Vec<Visit> {
        self.history.borrow().iter().rev().cloned().collect()
    }

    pub fn forget(self: &Rc<Self>, url: &str) {
        self.history.borrow_mut().retain(|v| v.url != url);
        self.save_later();
    }

    pub fn clear_history(self: &Rc<Self>) {
        self.history.borrow_mut().clear();
        self.save_later();
    }

    /// Pages whose address or title has `text`, most visited and newest
    /// first: what the address bar offers while typing.
    pub fn suggest(&self, text: &str, most: usize) -> Vec<(String, String)> {
        let text = text.trim().to_lowercase();
        if text.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<(u32, i64, String, String)> = Vec::new();
        for b in self.bookmarks.borrow().iter() {
            if b.url.to_lowercase().contains(&text) || b.title.to_lowercase().contains(&text) {
                // Bookmarks first.
                found.push((u32::MAX, b.time, b.url.clone(), b.title.clone()));
            }
        }
        for v in self.history.borrow().iter() {
            if found.iter().any(|f| f.2 == v.url) {
                continue;
            }
            let url = v.url.to_lowercase();
            let bare = url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches("www.");
            if bare.starts_with(&text) {
                found.push((v.count + 1000, v.time, v.url.clone(), v.title.clone()));
            } else if url.contains(&text) || v.title.to_lowercase().contains(&text) {
                found.push((v.count, v.time, v.url.clone(), v.title.clone()));
            }
        }
        found.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        found.into_iter().take(most).map(|f| (f.2, f.3)).collect()
    }

    pub fn bookmarks(&self) -> Vec<Bookmark> {
        self.bookmarks.borrow().clone()
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.bookmarks.borrow().iter().any(|b| b.url == url)
    }

    /// Ctrl+D: bookmarks the page, or takes the bookmark away; whether it
    /// is one now.
    pub fn toggle_bookmark(self: &Rc<Self>, url: &str, title: &str) -> bool {
        let now_marked = {
            let mut marks = self.bookmarks.borrow_mut();
            if let Some(i) = marks.iter().position(|b| b.url == url) {
                marks.remove(i);
                false
            } else {
                marks.push(Bookmark {
                    url: url.to_string(),
                    title: title.to_string(),
                    time: now(),
                });
                true
            }
        };
        self.save_later();
        now_marked
    }

    pub fn remove_bookmark(self: &Rc<Self>, url: &str) {
        self.bookmarks.borrow_mut().retain(|b| b.url != url);
        self.save_later();
    }

    pub fn closed(&self, url: &str) {
        if url.is_empty() || url == "about:blank" {
            return;
        }
        let mut closed = self.closed.borrow_mut();
        closed.retain(|u| u != url);
        closed.insert(0, url.to_string());
        closed.truncate(MOST_CLOSED);
    }

    pub fn reopen(&self) -> Option<String> {
        let mut closed = self.closed.borrow_mut();
        (!closed.is_empty()).then(|| closed.remove(0))
    }

    fn save_later(self: &Rc<Self>) {
        self.dirty.set(true);
        if self.save_soon.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_secs(5), move || {
            if let Some(l) = weak.upgrade() {
                l.save_soon.borrow_mut().take();
                l.save();
            }
        });
        *self.save_soon.borrow_mut() = Some(id);
    }

    /// Writes what changed (also when the window closes).
    pub fn save(&self) {
        if !self.dirty.replace(false) {
            return;
        }
        if let Err(err) = std::fs::create_dir_all(&self.dir) {
            log::warn!("{}: {err}", self.dir.display());
            return;
        }
        let write = |name: &str, text: String| {
            use std::os::unix::fs::PermissionsExt;
            let path = self.dir.join(name);
            let temporary = self.dir.join(format!(".{name}.new"));
            let done = std::fs::write(&temporary, text)
                .and_then(|()| std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600)))
                .and_then(|()| std::fs::rename(&temporary, &path));
            if let Err(err) = done {
                log::warn!("{}: {err}", path.display());
            }
        };
        if self.keep_history
            && let Ok(text) = serde_json::to_string(&*self.history.borrow())
        {
            write("history.json", text);
        }
        if let Ok(text) = serde_json::to_string_pretty(&*self.bookmarks.borrow()) {
            write("bookmarks.json", text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_bookmarks_then_history() {
        let dir = std::env::temp_dir().join(format!("pf-lib-{}", std::process::id()));
        let lib = Library::load(dir.clone(), true);
        lib.history.borrow_mut().push(Visit {
            url: "https://github.com/x".into(),
            title: "X".into(),
            time: 1,
            count: 5,
        });
        lib.history.borrow_mut().push(Visit {
            url: "https://example.org/github".into(),
            title: "".into(),
            time: 2,
            count: 9,
        });
        lib.bookmarks.borrow_mut().push(Bookmark {
            url: "https://docs.rs".into(),
            title: "GitHub docs".into(),
            time: 0,
        });
        let s = lib.suggest("github", 10);
        assert_eq!(s[0].0, "https://docs.rs");
        // Starting with what was typed beats containing it.
        assert_eq!(s[1].0, "https://github.com/x");
        let _ = std::fs::remove_dir_all(dir);
    }
}
