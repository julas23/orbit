//! Window rules (`rules:` in core.yaml, as sway's `for_window`): what a
//! window does when it opens — float, go to a workspace, take a size or a
//! place, open fullscreen or maximized, or not take the keyboard — picked
//! by its app_id (an X window's class) and title, regular expressions.
//! The first rule that matches applies, once: at the window's first frame
//! (a Wayland window names itself only after it is made), or when it maps
//! (X).

use serde::Deserialize;
use smithay::desktop::Window;

use crate::state::Orbit;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rule {
    /// A regular expression over the app_id (or the X class); none: any.
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub floating: Option<bool>,
    pub workspace: Option<i32>,
    /// Width and height, floating.
    pub size: Option<(i32, i32)>,
    /// Top-left corner on its monitor, in logical pixels, floating.
    pub position: Option<(i32, i32)>,
    pub fullscreen: Option<bool>,
    pub maximized: Option<bool>,
    /// `false`: it opens without taking the keyboard.
    pub focus: Option<bool>,
}

impl Rule {
    pub fn matches(&self, app_id: &str, title: &str) -> bool {
        let fits = |pattern: &Option<String>, text: &str| match pattern {
            None => true,
            Some(p) => match regex::Regex::new(p) {
                Ok(re) => re.is_match(text),
                Err(err) => {
                    tracing::warn!("rule {p:?}: {err}");
                    false
                }
            },
        };
        (self.app_id.is_some() || self.title.is_some()) && fits(&self.app_id, app_id) && fits(&self.title, title)
    }
}

/// Set on a window once its rules were looked at.
struct Ruled;

impl Orbit {
    /// Applies the first rule matching `window`, once it has a name — its
    /// app_id, which GTK gives after the title; at its first frame
    /// (`settled`) with whatever it has. True if it was looked at now.
    pub fn apply_rules(&mut self, window: &Window, settled: bool) -> bool {
        if window.user_data().get::<Ruled>().is_some() || self.config.rules.is_empty() {
            return false;
        }
        let (title, app_id) = crate::protocols::names(window);
        if (title.is_empty() && app_id.is_empty()) || (app_id.is_empty() && !settled) {
            return false;
        }
        window.user_data().insert_if_missing(|| Ruled);
        let Some(rule) = self.config.rules.iter().find(|r| r.matches(&app_id, &title)).cloned() else {
            return true;
        };
        tracing::info!("rule for {app_id} ({title})");
        if let Some(num) = rule.workspace {
            let num = self.resolve_for(window, num);
            self.move_to_workspace(window, num);
        }
        let floating = rule.floating.unwrap_or(false) || rule.size.is_some() || rule.position.is_some();
        // The place is on the monitor the window opens on.
        let origin = self
            .workspace_of(window)
            .and_then(|n| self.ws_output(n))
            .and_then(|o| self.output_named(&o))
            .and_then(|o| self.space.output_geometry(&o))
            .map_or((0, 0), |g| (g.loc.x, g.loc.y));
        if floating {
            self.set_floating(window, true);
            let mut info = crate::workspace::info(window).borrow_mut();
            if let Some(size) = rule.size {
                info.float_size = Some(size);
            }
            if let Some((x, y)) = rule.position {
                info.float_at = Some((origin.0 + x, origin.1 + y));
            }
        } else if rule.floating == Some(false) {
            self.set_floating(window, false);
        }
        if let Some(on) = rule.maximized {
            self.set_maximized(window, on);
        }
        if let Some(on) = rule.fullscreen {
            self.set_fullscreen(window, on);
        }
        self.arrange();
        if rule.focus == Some(false) && self.focused_window().as_ref() == Some(window) {
            // Back to the window that had the keyboard before.
            let previous = self.mru.iter().find(|w| *w != window).cloned();
            self.focus_window(previous);
        } else if rule.workspace.is_some() && self.focused_window().as_ref() == Some(window) {
            self.focus_anywhere(window);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching() {
        let rule = Rule {
            app_id: Some("(?i)^firefox$".into()),
            ..Default::default()
        };
        assert!(rule.matches("Firefox", "x"));
        assert!(!rule.matches("firefox-esr", "x"));
        let rule = Rule {
            title: Some("Picture-in-Picture".into()),
            ..Default::default()
        };
        assert!(rule.matches("any", "Picture-in-Picture"));
        // A rule naming nothing matches nothing.
        assert!(!Rule::default().matches("a", "b"));
    }
}
