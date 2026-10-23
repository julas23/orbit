//! The wake word, found in what Whisper wrote. Whisper spells a name it
//! does not know as it hears it — "Jarvis" comes out "Javis", "Já viz",
//! "Jávis" — so the first words are compared without accents, spaces or
//! punctuation, allowing a letter or two to differ. Only at the start: a
//! name in the middle of a sentence is talk about it, not to it.

/// Words looked at from the start.
const FIRST_WORDS: usize = 3;
/// Said before the name, and skipped.
const GREETINGS: [&str; 9] = ["hey", "hei", "ei", "ey", "ok", "okay", "oi", "ola", "hi"];

/// Lower case, no accents, letters and digits only.
fn fold(word: &str) -> String {
    word.chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| {
            Some(match c {
                'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
                'é' | 'ê' | 'è' | 'ë' => 'e',
                'í' | 'î' | 'ì' | 'ï' => 'i',
                'ó' | 'ô' | 'õ' | 'ò' | 'ö' => 'o',
                'ú' | 'û' | 'ù' | 'ü' => 'u',
                'ç' => 'c',
                c if c.is_alphanumeric() => c,
                _ => return None,
            })
        })
        .collect()
}

fn distance(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(ca != cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

/// How many letters may differ for a wake word this long.
fn allowed(len: usize) -> usize {
    match len {
        0..=3 => 0,
        4 => 1,
        _ => 2,
    }
}

/// When `text` starts with one of `words`, the rest of it (the request,
/// maybe empty), its punctuation trimmed.
pub fn split(text: &str, words: &[String]) -> Option<String> {
    find(text, words, 0).map(|(_, rest)| rest)
}

/// `split`, with how many of `text`'s words (split at spaces) the wake
/// word took.
pub fn split_at(text: &str, words: &[String]) -> Option<(usize, String)> {
    find(text, words, 0)
}

/// Whether `text` starts with something a letter further from one of
/// `words` than `split` allows ("Já visse" for "Jarvis"): worth hearing
/// again with a better model.
pub fn near(text: &str, words: &[String]) -> bool {
    find(text, words, 1).is_some()
}

/// `split`, `slack` letters more lenient.
fn find(text: &str, words: &[String], slack: usize) -> Option<(usize, String)> {
    let spoken: Vec<&str> = text.split_whitespace().collect();
    let skip = spoken.first().is_some_and(|first| GREETINGS.contains(&fold(first).as_str()));
    let spoken = &spoken[usize::from(skip)..];
    for wake in words {
        let wake: Vec<char> = fold(wake).chars().collect();
        if wake.is_empty() {
            continue;
        }
        let mut joined = String::new();
        for (taken, word) in spoken.iter().take(FIRST_WORDS).enumerate() {
            joined.push_str(&fold(word));
            let letters: Vec<char> = joined.chars().collect();
            // The words so far against the wake word: a letter shorter
            // ("javis") to a letter longer ("órbita"), no more, or
            // "orbital" would be "orbit". And the same first letter, or
            // "o Jarvis" would be "Jarvis".
            let close = letters.first() == wake.first()
                && letters.len() + 1 + slack >= wake.len()
                && letters.len() <= wake.len() + 1 + slack
                && distance(&letters, &wake) <= allowed(wake.len()) + slack;
            if close {
                let rest = spoken[taken + 1..].join(" ");
                let rest = rest.trim_start_matches(|c: char| !c.is_alphanumeric());
                return Some((usize::from(skip) + taken + 1, rest.to_owned()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> Vec<String> {
        vec!["orbit".into(), "jarvis".into()]
    }

    #[test]
    fn heard_spellings_wake_it() {
        for (text, rest) in [
            ("Jarvis, abra o terminal.", "abra o terminal."),
            ("Javis, abra o terminal e me diga quanto espaço livre tem no disco.", "abra o terminal e me diga quanto espaço livre tem no disco."),
            ("Já viz, abro terminal.", "abro terminal."),
            ("Jávis. Abro terminal.", "Abro terminal."),
            ("Orbit, list the files in my home folder.", "list the files in my home folder."),
            ("Órbita, que horas são?", "que horas são?"),
            ("Hey Jarvis!", ""),
            ("Jarvis.", ""),
        ] {
            assert_eq!(split(text, &words()).as_deref(), Some(rest), "{text}");
        }
    }

    #[test]
    fn near_misses_are_worth_another_try() {
        assert_eq!(split("já visse, abro terminal", &words()), None);
        assert!(near("já visse, abro terminal", &words()));
        assert!(!near("Amanhã eu vou ao mercado comprar pão.", &words()));
    }

    #[test]
    fn says_how_many_words_the_name_took() {
        assert_eq!(split_at("Hey Jarvis, que horas são?", &words()), Some((2, "que horas são?".into())));
        assert_eq!(split_at("Já viz, abro", &words()), Some((2, "abro".into())));
    }

    #[test]
    fn other_talk_does_not() {
        for text in [
            "Amanhã eu vou ao mercado comprar pão.",
            "O Jarvis do filme é muito bom.",
            "Obrigado.",
            "",
            "Já vi isso antes",
            "Orbital mechanics is hard",
        ] {
            assert_eq!(split(text, &words()), None, "{text}");
        }
    }
}
