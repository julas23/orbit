//! `.editorconfig`: what a project says about its files' indentation,
//! line endings and spaces, read from the file's folder up to the one
//! with `root = true` (the nearer file wins). The parts used:
//! `indent_style`, `indent_size`, `tab_width`, `end_of_line`,
//! `trim_trailing_whitespace` and `insert_final_newline`.

use std::path::Path;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Settings {
    pub spaces: Option<bool>,
    pub width: Option<u32>,
    pub crlf: Option<bool>,
    pub trim_trailing: Option<bool>,
    pub final_newline: Option<bool>,
}

/// The settings for `file`.
pub fn for_file(file: &Path) -> Settings {
    // From the farthest folder in, so the nearer ones win.
    let mut found = Vec::new();
    let mut dir = file.parent();
    while let Some(d) = dir {
        let config = d.join(".editorconfig");
        if let Ok(text) = std::fs::read_to_string(&config) {
            let root = is_root(&text);
            found.push((d.to_path_buf(), text));
            if root {
                break;
            }
        }
        dir = d.parent();
    }
    let mut out = Settings::default();
    for (dir, text) in found.iter().rev() {
        let Ok(relative) = file.strip_prefix(dir) else { continue };
        apply(&mut out, text, &relative.to_string_lossy());
    }
    out
}

fn is_root(text: &str) -> bool {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            return false;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case("root")
        {
            return v.trim().eq_ignore_ascii_case("true");
        }
    }
    false
}

/// The sections of one `.editorconfig` that match `relative` (the file's
/// path from that file's folder), applied in order.
fn apply(out: &mut Settings, text: &str, relative: &str) {
    let mut matching = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            matching = glob_matches(section, relative);
            continue;
        }
        if !matching {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim().to_lowercase();
        let flag = || match value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        match key.as_str() {
            "indent_style" => out.spaces = (value != "unset").then(|| value == "space"),
            "indent_size" if value != "tab" => out.width = value.parse().ok().or(out.width),
            "tab_width" if out.width.is_none() => out.width = value.parse().ok(),
            "end_of_line" => out.crlf = (value != "unset").then(|| value == "crlf"),
            "trim_trailing_whitespace" => out.trim_trailing = flag(),
            "insert_final_newline" => out.final_newline = flag(),
            _ => {}
        }
    }
}

/// EditorConfig's globs: `*` within a folder, `**` across, `?`, `{a,b}`,
/// `[abc]`; with no `/`, matched against the file's name alone.
pub fn glob_matches(glob: &str, relative: &str) -> bool {
    let (glob, target) = if glob.contains('/') {
        (glob.trim_start_matches('/').to_string(), relative.to_string())
    } else {
        (
            glob.to_string(),
            relative.rsplit('/').next().unwrap_or(relative).to_string(),
        )
    };
    let mut re = String::from("^");
    let mut chars = glob.chars().peekable();
    let mut braces = 0;
    while let Some(c) = chars.next() {
        match c {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                re.push_str(".*");
            }
            '*' => re.push_str("[^/]*"),
            '?' => re.push_str("[^/]"),
            '{' => {
                braces += 1;
                re.push_str("(?:");
            }
            '}' if braces > 0 => {
                braces -= 1;
                re.push(')');
            }
            ',' if braces > 0 => re.push('|'),
            '[' => {
                re.push('[');
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    if c == '!' {
                        re.push('^');
                    } else {
                        re.push(c);
                    }
                }
                re.push(']');
            }
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    regex::Regex::new(&re).is_ok_and(|r| r.is_match(&target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_matches("*", "src/main.rs"));
        assert!(glob_matches("*.rs", "src/main.rs"));
        assert!(glob_matches("*.{js,ts}", "a/b.ts"));
        assert!(!glob_matches("*.{js,ts}", "a/b.rs"));
        assert!(glob_matches("Makefile", "Makefile"));
        assert!(glob_matches("src/**.rs", "src/a/b.rs"));
        assert!(!glob_matches("src/*.rs", "lib/a.rs"));
    }

    #[test]
    fn sections() {
        let text = "root = true\n[*]\nindent_style = space\nindent_size = 4\n[Makefile]\nindent_style = tab\n";
        let mut s = Settings::default();
        apply(&mut s, text, "Makefile");
        assert_eq!(s.spaces, Some(false));
        assert_eq!(s.width, Some(4));
        assert!(is_root(text));
    }
}
