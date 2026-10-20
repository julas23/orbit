//! The clipboard history and where it is kept.
//!
//! Newest first, at most `limit` items. Copying something already in the
//! history moves it to the top instead of repeating it. A selection made by
//! dragging the mouse grows one character at a time, and each step changes
//! the primary selection: a primary-selection text that extends or shrinks
//! the one on top replaces it, so only the final selection stays.
//!
//! On disk, in `~/.local/share/orbit/clip/`: `history.json` with the items,
//! and each image in `images/<id>.<ext>`, as copied.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    Image { mime: String, bytes: Vec<u8> },
}

impl Content {
    fn same(&self, other: &Content) -> bool {
        match (self, other) {
            (Content::Text(a), Content::Text(b)) => a == b,
            (Content::Image { bytes: a, .. }, Content::Image { bytes: b, .. }) => a == b,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: u64,
    pub content: Content,
    /// Seconds since the epoch of the last copy.
    pub time: i64,
    /// Came from the primary selection (selected with the mouse).
    pub primary: bool,
}

#[derive(Debug, Clone)]
pub struct History {
    items: Vec<Item>,
    limit: usize,
    next_id: u64,
}

impl History {
    pub fn new(limit: usize) -> Self {
        History {
            items: Vec::new(),
            limit: limit.max(1),
            next_id: 1,
        }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// Adds a copy; returns whether the history changed.
    pub fn add(&mut self, content: Content, primary: bool, time: i64) -> bool {
        if let Content::Text(text) = &content
            && text.trim().is_empty()
        {
            return false;
        }
        if let Some(pos) = self.items.iter().position(|i| i.content.same(&content)) {
            if pos == 0 {
                return false;
            }
            let mut item = self.items.remove(pos);
            item.time = time;
            self.items.insert(0, item);
            return true;
        }
        if primary
            && let (Some(top), Content::Text(new)) = (self.items.first_mut(), &content)
            && top.primary
            && let Content::Text(old) = &top.content
            && grows(old, new)
        {
            top.content = content;
            top.time = time;
            return true;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.items.insert(
            0,
            Item {
                id,
                content,
                time,
                primary,
            },
        );
        self.items.truncate(self.limit);
        true
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|i| i.id != id);
        self.items.len() != before
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// The items whose text contains every word of `query`, ignoring case.
    /// Images match an empty query only.
    pub fn search<'a>(&'a self, query: &str) -> impl Iterator<Item = &'a Item> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.items.iter().filter(move |item| match &item.content {
            _ if words.is_empty() => true,
            Content::Text(text) => {
                let text = text.to_lowercase();
                words.iter().all(|w| text.contains(w.as_str()))
            }
            Content::Image { .. } => false,
        })
    }
}

/// Whether `new` is `old` extended or shrunk at either end — the same mouse
/// selection, still being dragged.
fn grows(old: &str, new: &str) -> bool {
    new.starts_with(old) || new.ends_with(old) || old.starts_with(new) || old.ends_with(new)
}

#[derive(Serialize, Deserialize)]
struct Saved {
    id: u64,
    time: i64,
    #[serde(default)]
    primary: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    /// For images: the file in images/ and its MIME type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mime: Option<String>,
}

/// The history's directory on disk.
pub struct Store {
    dir: PathBuf,
}

fn extension(mime: &str) -> &str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

impl Store {
    pub fn new(dir: &Path) -> Self {
        Store {
            dir: dir.to_path_buf(),
        }
    }

    fn images(&self) -> PathBuf {
        self.dir.join("images")
    }

    /// The saved history; an empty one when there is none yet.
    pub fn load(&self, limit: usize) -> History {
        let mut history = History::new(limit);
        let path = self.dir.join("history.json");
        let saved: Vec<Saved> = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(saved) => saved,
                Err(err) => {
                    log::warn!("{}: {err}; starting a new history", path.display());
                    return history;
                }
            },
            Err(_) => return history,
        };
        for s in saved {
            let content = match (s.text, s.image) {
                (Some(text), _) => Content::Text(text),
                (None, Some(file)) => match std::fs::read(self.images().join(&file)) {
                    Ok(bytes) => Content::Image {
                        mime: s.mime.unwrap_or_else(|| "image/png".into()),
                        bytes,
                    },
                    Err(err) => {
                        log::warn!("clip image {file}: {err}");
                        continue;
                    }
                },
                (None, None) => continue,
            };
            history.next_id = history.next_id.max(s.id + 1);
            history.items.push(Item {
                id: s.id,
                content,
                time: s.time,
                primary: s.primary,
            });
        }
        history.items.truncate(history.limit);
        history
    }

    /// Writes the history: the list, the images not yet on disk, and drops
    /// the files of images no longer in it.
    pub fn save(&self, history: &History) -> anyhow::Result<()> {
        let images = self.images();
        std::fs::create_dir_all(&images)
            .with_context(|| format!("creating {}", images.display()))?;
        let mut kept = HashSet::new();
        let mut saved = Vec::new();
        for item in &history.items {
            let mut s = Saved {
                id: item.id,
                time: item.time,
                primary: item.primary,
                text: None,
                image: None,
                mime: None,
            };
            match &item.content {
                Content::Text(text) => s.text = Some(text.clone()),
                Content::Image { mime, bytes } => {
                    let file = format!("{}.{}", item.id, extension(mime));
                    let path = images.join(&file);
                    if !path.exists() {
                        std::fs::write(&path, bytes)
                            .with_context(|| format!("writing {}", path.display()))?;
                    }
                    kept.insert(file.clone());
                    s.image = Some(file);
                    s.mime = Some(mime.clone());
                }
            }
            saved.push(s);
        }
        // Write a new file and rename it over the old one: a crash midway
        // never leaves a half-written history.
        let path = self.dir.join("history.json");
        let temporary = self.dir.join("history.json.new");
        std::fs::write(&temporary, serde_json::to_vec(&saved)?)
            .with_context(|| format!("writing {}", temporary.display()))?;
        std::fs::rename(&temporary, &path)
            .with_context(|| format!("replacing {}", path.display()))?;
        for entry in std::fs::read_dir(&images)?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !kept.contains(&name) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Content {
        Content::Text(s.into())
    }

    fn texts(h: &History) -> Vec<String> {
        h.items()
            .iter()
            .map(|i| match &i.content {
                Content::Text(t) => t.clone(),
                Content::Image { .. } => "<image>".into(),
            })
            .collect()
    }

    #[test]
    fn order_repeats_and_limit() {
        let mut h = History::new(3);
        assert!(h.add(text("a"), false, 1));
        assert!(h.add(text("b"), false, 2));
        assert!(h.add(text("c"), false, 3));
        assert_eq!(texts(&h), ["c", "b", "a"]);
        assert!(h.add(text("a"), false, 4), "a repeat moves to the top");
        assert_eq!(texts(&h), ["a", "c", "b"]);
        assert_eq!(h.items()[0].time, 4);
        assert!(!h.add(text("a"), false, 5), "already on top");
        assert!(h.add(text("d"), false, 6));
        assert_eq!(texts(&h), ["d", "a", "c"], "the oldest goes");
        assert!(!h.add(text("  \n"), false, 7), "blank text is ignored");
    }

    #[test]
    fn a_growing_mouse_selection_is_one_item() {
        let mut h = History::new(10);
        h.add(text("before"), false, 1);
        for s in ["h", "he", "hel", "hello", "hello w", "hello world"] {
            h.add(text(s), true, 2);
        }
        assert_eq!(texts(&h), ["hello world", "before"]);
        // A new selection elsewhere is a new item.
        h.add(text("other"), true, 3);
        assert_eq!(texts(&h), ["other", "hello world", "before"]);
        // Clipboard copies never merge.
        h.add(text("other words"), false, 4);
        assert_eq!(h.items().len(), 4);
    }

    #[test]
    fn search_and_remove() {
        let mut h = History::new(10);
        h.add(text("Ola Mundo"), false, 1);
        h.add(text("mundo novo"), false, 2);
        h.add(
            Content::Image {
                mime: "image/png".into(),
                bytes: vec![1, 2, 3],
            },
            false,
            3,
        );
        let found: Vec<u64> = h.search("MUNDO").map(|i| i.id).collect();
        assert_eq!(found, [2, 1]);
        assert_eq!(h.search("novo mundo").count(), 1, "every word, any order");
        assert_eq!(h.search("").count(), 3);
        assert!(h.remove(2));
        assert!(!h.remove(2));
        assert_eq!(texts(&h), ["<image>", "Ola Mundo"]);
    }

    #[test]
    fn saved_and_loaded() {
        let dir = std::env::temp_dir().join(format!("orbit-clip-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(&dir);
        let mut h = History::new(10);
        h.add(text("um"), false, 1);
        h.add(
            Content::Image {
                mime: "image/png".into(),
                bytes: vec![137, 80, 78, 71],
            },
            false,
            2,
        );
        h.add(text("dois"), true, 3);
        store.save(&h).unwrap();
        let loaded = store.load(10);
        assert_eq!(loaded.items(), h.items());
        // New items get new ids after a reload.
        let mut loaded = loaded;
        loaded.add(text("três"), false, 4);
        assert_eq!(loaded.items()[0].id, 4);
        // A dropped image's file goes too.
        let image_id = h.items()[1].id;
        h.remove(image_id);
        store.save(&h).unwrap();
        assert_eq!(std::fs::read_dir(dir.join("images")).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
