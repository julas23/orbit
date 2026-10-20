//! Sorting names as a person would: ignoring case, with numbers compared
//! as numbers (`foto2` before `foto10`).

use std::{cmp::Ordering, path::Path};

/// Orders paths by their file names, naturally.
pub fn by_name(a: &Path, b: &Path) -> Ordering {
    natural(&name(a), &name(b))
}

fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Compares runs of digits by value and everything else by character.
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (da, db) = (take(&mut a), take(&mut b));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let order = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_sort_by_value() {
        let mut names = vec!["foto10.jpg", "foto2.jpg", "foto1.jpg", "Foto3.jpg"];
        names.sort_by(|a, b| natural(&a.to_lowercase(), &b.to_lowercase()));
        assert_eq!(names, ["foto1.jpg", "foto2.jpg", "Foto3.jpg", "foto10.jpg"]);
    }

    #[test]
    fn leading_zeros_and_prefixes() {
        assert_eq!(natural("a007", "a7"), Ordering::Equal);
        assert_eq!(natural("a", "a1"), Ordering::Less);
        assert_eq!(
            natural("captura-2026-10-20_09-05-01", "captura-2026-10-20_10-00-00"),
            Ordering::Less
        );
    }
}
