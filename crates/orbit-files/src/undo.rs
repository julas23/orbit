//! Ctrl+Z: what was done to files, undone, last first — a move or a
//! rename moved back, a copy or a new file or folder sent to the trash,
//! what went to the trash brought back from it. Deleting for good cannot
//! be undone. Every window of the program shares the one list.

use std::{cell::RefCell, path::PathBuf};

use orbit_app::i18n::tr;

/// Steps remembered.
const MOST: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Moved: (from, to).
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Renamed: (from, to).
    Renamed(Vec<(PathBuf, PathBuf)>),
    /// Copies made, where they are.
    Copied(Vec<PathBuf>),
    /// Sent to the trash, from where.
    Trashed(Vec<PathBuf>),
    /// A new file or folder.
    Created(PathBuf),
}

impl Step {
    /// "Undo move", for the menu.
    pub fn label(&self) -> String {
        tr(match self {
            Step::Moved(_) => "files-undo-move",
            Step::Renamed(_) => "files-undo-rename",
            Step::Copied(_) => "files-undo-copy",
            Step::Trashed(_) => "files-undo-trash",
            Step::Created(_) => "files-undo-create",
        })
    }

    fn is_empty(&self) -> bool {
        match self {
            Step::Moved(v) | Step::Renamed(v) => v.is_empty(),
            Step::Copied(v) | Step::Trashed(v) => v.is_empty(),
            Step::Created(_) => false,
        }
    }
}

thread_local! {
    static STEPS: RefCell<Vec<Step>> = const { RefCell::new(Vec::new()) };
}

pub fn push(step: Step) {
    if step.is_empty() {
        return;
    }
    STEPS.with(|s| {
        let mut s = s.borrow_mut();
        s.push(step);
        if s.len() > MOST {
            s.remove(0);
        }
    });
}

pub fn pop() -> Option<Step> {
    STEPS.with(|s| s.borrow_mut().pop())
}

/// The step Ctrl+Z would undo.
pub fn last() -> Option<Step> {
    STEPS.with(|s| s.borrow().last().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_first_and_nothing_empty() {
        push(Step::Copied(vec![]));
        assert_eq!(last(), None);
        push(Step::Created("/a".into()));
        push(Step::Trashed(vec!["/b".into()]));
        assert_eq!(pop(), Some(Step::Trashed(vec!["/b".into()])));
        assert_eq!(pop(), Some(Step::Created("/a".into())));
        assert_eq!(pop(), None);
    }
}
