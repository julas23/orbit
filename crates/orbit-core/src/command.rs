//! Sway's command language, the part Orbit uses (PLANO-CORE §5): what the
//! panel, the dock, scripts and `orbitctl` send.
//!
//! `;` separates commands; `,` chains commands on the same criteria;
//! criteria in brackets pick the windows (`[con_id=12]`,
//! `[app_id="(?i)firefox"]`, `[title="…"]`), else the focused one is meant.
//! A command the core does not do yet answers `success: false` with why —
//! never a silent success.

use regex::RegexBuilder;
use serde_json::{Value, json};
use smithay::desktop::Window;

use crate::{state::Orbit, workspace};

/// Runs `line`; one result per command, as sway answers.
pub fn run(state: &mut Orbit, line: &str) -> Vec<Value> {
    let mut results = Vec::new();
    for statement in split(line, ';') {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        let (criteria, rest) = match statement.strip_prefix('[').and_then(|s| s.split_once(']')) {
            Some((c, rest)) => (Some(c.to_owned()), rest),
            None => (None, statement),
        };
        let targets = match &criteria {
            Some(c) => match select(state, c) {
                Ok(windows) => Some(windows),
                Err(err) => {
                    results.push(fail(&err));
                    continue;
                }
            },
            None => None,
        };
        for command in split(rest, ',') {
            let command = command.trim();
            if command.is_empty() {
                continue;
            }
            let outcome = execute(state, command, targets.as_deref());
            results.push(match outcome {
                Ok(()) => json!({ "success": true }),
                Err(err) => fail(&err),
            });
        }
    }
    if results.is_empty() {
        results.push(fail("no command"));
    }
    results
}

fn fail(error: &str) -> Value {
    json!({ "success": false, "parse_error": false, "error": error })
}

/// Splits on `sep` outside quotes and brackets.
fn split(text: &str, sep: char) -> Vec<String> {
    let (mut parts, mut current) = (Vec::new(), String::new());
    let (mut quote, mut depth) = (None, 0);
    for c in text.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => depth += 1,
            (None, ']') => depth -= 1,
            (None, c) if c == sep && depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    parts.push(current);
    parts
}

/// The words of a command, quotes removed.
fn words(text: &str) -> Vec<String> {
    let (mut words, mut current, mut quote, mut any) = (Vec::new(), String::new(), None, false);
    for c in text.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                any = false;
            }
            _ => current.push(c),
        }
    }
    if any || !current.is_empty() {
        words.push(current);
    }
    words
}

/// The windows the criteria pick (`key=value` pairs, all must match).
fn select(state: &Orbit, criteria: &str) -> Result<Vec<Window>, String> {
    let mut windows: Vec<Window> = state.all_windows().cloned().collect();
    for pair in words(criteria) {
        let (key, value) = pair.split_once('=').map(|(k, v)| (k.trim(), v.trim())).unwrap_or((pair.as_str(), ""));
        match key {
            "con_id" => match value {
                "__focused__" => {
                    let focused = state.focused_window();
                    windows.retain(|w| Some(w) == focused.as_ref());
                }
                id => {
                    let id: u64 = id.parse().map_err(|_| format!("con_id: not a number: {id}"))?;
                    windows.retain(|w| workspace::id(w) == id);
                }
            },
            "app_id" | "title" => {
                let pattern = RegexBuilder::new(value).build().map_err(|e| format!("{key}: {e}"))?;
                windows.retain(|w| {
                    let v = state.window_json(w);
                    pattern.is_match(v[if key == "app_id" { "app_id" } else { "name" }].as_str().unwrap_or_default())
                });
            }
            "workspace" => windows.retain(|w| state.workspace_of(w).is_some_and(|n| n.to_string() == value)),
            // X11 windows: none until XWayland.
            "class" | "instance" | "window_role" => windows.clear(),
            other => return Err(format!("criteria not supported yet: {other}")),
        }
    }
    Ok(windows)
}

fn execute(state: &mut Orbit, command: &str, targets: Option<&[Window]>) -> Result<(), String> {
    let w = words(command);
    let w: Vec<&str> = w.iter().map(String::as_str).collect();
    // The windows a command acts on: the criteria's, or the focused one.
    let windows = || -> Vec<Window> {
        match targets {
            Some(t) => t.to_vec(),
            None => state.focused_window().into_iter().collect(),
        }
    };
    match w.as_slice() {
        ["nop", ..] => Ok(()),
        ["exec" | "exec_always", ..] => {
            let rest = command.trim_start().split_once(char::is_whitespace).map(|(_, r)| r.trim()).unwrap_or_default();
            let rest = rest.strip_prefix("--no-startup-id").unwrap_or(rest).trim();
            if rest.is_empty() {
                return Err("exec: nothing to run".into());
            }
            state.spawn(rest);
            Ok(())
        }
        ["exit"] => {
            state.loop_signal.stop();
            Ok(())
        }
        // Anyone may lock; only the greet unlocks.
        ["lock"] => {
            state.lock();
            Ok(())
        }
        ["reload"] => {
            state.reload_config();
            Ok(())
        }
        ["workspace", rest @ ..] => {
            let num = workspace_number(state, rest)?;
            state.switch_workspace(num);
            Ok(())
        }
        ["focus"] => {
            let Some(window) = windows().into_iter().last() else { return Err("No window matches".into()) };
            state.focus_anywhere(&window);
            Ok(())
        }
        ["focus", dir @ ("left" | "right" | "up" | "down" | "next" | "prev")] => {
            let next = state.neighbour(matches!(*dir, "right" | "down" | "next"));
            if next.is_some() {
                state.focus_window(next);
            }
            Ok(())
        }
        ["focus", "output", which] => {
            let target = state.output_toward(which).ok_or_else(|| format!("There is no output called {which}"))?;
            if let Some(num) = state.visible.get(&target).copied() {
                state.switch_workspace(num);
            }
            Ok(())
        }
        ["kill"] => {
            for window in windows() {
                if let Some(t) = window.toplevel() {
                    t.send_close();
                }
            }
            Ok(())
        }
        ["move", "scratchpad"] | ["move", "to", "scratchpad"] | ["move", "container" | "window", "to", "scratchpad"] => {
            for window in windows() {
                state.move_to_scratchpad(&window);
            }
            Ok(())
        }
        ["scratchpad", "show"] => {
            let Some(window) = state.scratchpad.last().cloned() else { return Err("Scratchpad is empty".into()) };
            state.focus_anywhere(&window);
            Ok(())
        }
        ["move", rest @ ..] if rest.contains(&"workspace") => {
            let at = rest.iter().position(|w| *w == "workspace").unwrap_or(0);
            let num = workspace_number(state, &rest[at + 1..])?;
            for window in windows() {
                state.move_to_workspace(&window, num);
            }
            Ok(())
        }
        ["move", dir @ ("left" | "right" | "up" | "down")] => {
            state.move_focused(matches!(*dir, "right" | "down"));
            Ok(())
        }
        // Every window is tiled: already so.
        ["floating", "disable"] => Ok(()),
        ["input", _, "xkb_switch_layout", which] => {
            state.switch_layout(which)?;
            Ok(())
        }
        ["floating", ..] | ["fullscreen", ..] | ["split" | "splith" | "splitv", ..] | ["layout", ..] | ["resize", ..] | ["gaps", ..] | ["border", ..] | ["sticky", ..] | ["mark", ..] => {
            Err(format!("not in orbit-core yet: {}", w[0]))
        }
        _ => Err(format!("Unknown/invalid command '{command}'")),
    }
}

/// `number 3`, `3`, `next`, `prev`, `back_and_forth`.
fn workspace_number(state: &Orbit, rest: &[&str]) -> Result<i32, String> {
    let nums: Vec<i32> = state.workspaces.iter().map(|w| w.num).collect();
    let pos = nums.iter().position(|n| *n == state.current).unwrap_or(0);
    let pick = |s: &str| s.split(':').next().and_then(|n| n.parse::<i32>().ok());
    match rest {
        ["number", n, ..] => pick(n).ok_or_else(|| format!("not a workspace number: {n}")),
        ["next" | "next_on_output"] => Ok(nums[(pos + 1) % nums.len().max(1)]),
        ["prev" | "prev_on_output"] => Ok(nums[(pos + nums.len().max(1) - 1) % nums.len().max(1)]),
        ["back_and_forth"] => Ok(state.previous_ws.unwrap_or(state.current)),
        [n] => pick(n).ok_or_else(|| format!("named workspaces are not in orbit-core yet: {n}")),
        _ => Err("workspace: which?".into()),
    }
}

impl Orbit {
    /// Gives the keyboard to `window` wherever it is: its workspace is
    /// shown; from the scratchpad it comes to the workspace shown (tiled —
    /// the panel's "restore").
    pub fn focus_anywhere(&mut self, window: &Window) {
        match self.workspace_of(window) {
            None => self.move_to_workspace(window, self.current),
            Some(n) if n != self.current => self.switch_workspace(n),
            _ => {}
        }
        self.focus_window(Some(window.clone()));
    }

    /// `xkb_switch_layout next|prev|N`.
    fn switch_layout(&mut self, which: &str) -> Result<(), String> {
        let Some(keyboard) = self.seat.get_keyboard() else { return Err("no keyboard".into()) };
        let count = self.layouts.0.len().max(1) as u32;
        let target = match which {
            "next" | "prev" => None,
            n => Some(n.parse::<u32>().map_err(|_| format!("not a layout: {n}"))?),
        };
        if target.is_some_and(|n| n >= count) {
            return Err(format!("there are {count} layouts"));
        }
        keyboard.with_xkb_state(self, |mut ctx| match (which, target) {
            (_, Some(n)) => ctx.set_layout(smithay::input::keyboard::Layout(n)),
            ("prev", None) => {
                for _ in 1..count {
                    ctx.cycle_next_layout();
                }
            }
            _ => ctx.cycle_next_layout(),
        });
        self.read_layouts();
        self.input_event();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_outside_quotes_and_brackets() {
        assert_eq!(split("[title=\"a;b\"] focus; workspace 2", ';'), vec!["[title=\"a;b\"] focus", " workspace 2"]);
        assert_eq!(split("move container to workspace number 3, floating disable, focus", ','), vec![
            "move container to workspace number 3",
            " floating disable",
            " focus"
        ]);
    }

    #[test]
    fn words_drop_quotes() {
        assert_eq!(words("app_id=\"(?i)Org Foo\" con_id=3"), vec!["app_id=(?i)Org Foo", "con_id=3"]);
        assert_eq!(words("exec 'a b'"), vec!["exec", "a b"]);
    }
}
