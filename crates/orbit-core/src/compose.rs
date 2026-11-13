//! Dead keys and the Compose key for programs that type through the
//! text-input protocol.
//!
//! From GTK 4.20 on, a GTK program on Wayland leaves composing to an input
//! method; without one (Orbit runs none) a dead key followed by "a" gives a
//! plain "a". So the core acts as the input method: the keys of a sequence
//! stop here, and what they make goes to the program as text
//! (`commit_string`). The table is the locale's, with `~/.XCompose`.
//!
//! Programs without text-input enabled — kitty, X11 programs — get the keys
//! as before and compose them themselves. A real input method (fcitx5…)
//! takes over once one connects. Keys typed with Ctrl, Alt or the logo key
//! are never composed.

use std::ffi::OsString;

use smithay::{
    input::keyboard::{
        Keysym, ModifiersState,
        xkb::{self, compose},
    },
    wayland::{input_method::InputMethodSeat, text_input::TextInputSeat},
};

use crate::state::Orbit;

/// What a pressed key does to the sequence being composed.
#[derive(Debug, PartialEq)]
pub enum Step {
    /// Not part of a sequence: the program gets the key.
    Pass,
    /// Part of a sequence not finished yet: held here.
    Hold,
    /// The sequence's end: this text instead of the key.
    Text(String),
}

pub struct Compose {
    state: Option<compose::State>,
}

impl Compose {
    pub fn new() -> Self {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .filter_map(std::env::var_os)
            .find(|v| !v.is_empty())
            .unwrap_or_else(|| OsString::from("C"));
        let table = compose::Table::new_from_locale(&context, &locale, compose::COMPILE_NO_FLAGS)
            .or_else(|()| compose::Table::new_from_locale(&context, &OsString::from("C"), compose::COMPILE_NO_FLAGS));
        let state = match table {
            Ok(table) => Some(compose::State::new(&table, compose::STATE_NO_FLAGS)),
            Err(()) => {
                tracing::warn!("no compose table for {locale:?}: dead keys left to the programs");
                None
            }
        };
        Self { state }
    }

    pub fn reset(&mut self) {
        if let Some(state) = &mut self.state {
            state.reset();
        }
    }

    /// A pressed key's keysym.
    pub fn feed(&mut self, sym: Keysym) -> Step {
        let Some(state) = &mut self.state else {
            return Step::Pass;
        };
        // Shift and the like: neither part of a sequence nor its end.
        if state.feed(sym) == compose::FeedResult::Ignored {
            return Step::Pass;
        }
        match state.status() {
            compose::Status::Composing => Step::Hold,
            compose::Status::Composed => {
                let text = state
                    .utf8()
                    .filter(|t| !t.is_empty())
                    .or_else(|| state.keysym().map(xkb::keysym_to_utf8))
                    .map(|t| t.trim_end_matches('\0').to_owned())
                    .unwrap_or_default();
                state.reset();
                if text.is_empty() { Step::Hold } else { Step::Text(text) }
            }
            // A key that ends no sequence: the sequence is dropped, the key
            // goes on.
            compose::Status::Cancelled => {
                state.reset();
                Step::Pass
            }
            compose::Status::Nothing => Step::Pass,
        }
    }
}

impl Orbit {
    /// A pressed key, given to the composer when the program focused types
    /// through text-input; whether the key stops here.
    pub fn compose_key(&mut self, mods: &ModifiersState, sym: Keysym) -> bool {
        let text_input = self.seat.text_input().clone();
        let mut enabled = false;
        text_input.with_active_text_input(|_, _| enabled = true);
        if !enabled || mods.ctrl || mods.alt || mods.logo || self.seat.input_method().has_instance() {
            self.compose.reset();
            return false;
        }
        match self.compose.feed(sym) {
            Step::Pass => false,
            Step::Hold => true,
            Step::Text(text) => {
                text_input.with_active_text_input(|ti, _| ti.commit_string(Some(text.clone())));
                text_input.done(false);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn composer() -> Compose {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let table =
            compose::Table::new_from_locale(&context, &OsString::from("en_US.UTF-8"), compose::COMPILE_NO_FLAGS)
                .expect("the en_US compose table (libx11-data)");
        Compose {
            state: Some(compose::State::new(&table, compose::STATE_NO_FLAGS)),
        }
    }

    #[test]
    fn a_dead_key_and_a_letter_make_one_letter() {
        let mut c = composer();
        assert_eq!(c.feed(Keysym::dead_acute), Step::Hold);
        assert_eq!(c.feed(Keysym::a), Step::Text("á".into()));
        assert_eq!(c.feed(Keysym::dead_tilde), Step::Hold);
        assert_eq!(c.feed(Keysym::Shift_L), Step::Pass);
        assert_eq!(c.feed(Keysym::A), Step::Text("Ã".into()));
        assert_eq!(c.feed(Keysym::dead_cedilla), Step::Hold);
        assert_eq!(c.feed(Keysym::c), Step::Text("ç".into()));
    }

    #[test]
    fn a_dead_key_and_space_make_the_mark_itself() {
        let mut c = composer();
        assert_eq!(c.feed(Keysym::dead_acute), Step::Hold);
        assert_eq!(c.feed(Keysym::space), Step::Text("'".into()));
    }

    #[test]
    fn plain_keys_pass() {
        let mut c = composer();
        assert_eq!(c.feed(Keysym::a), Step::Pass);
        assert_eq!(c.feed(Keysym::Return), Step::Pass);
    }
}
