//! Numbers and expressions as shown, in the user's notation.
//!
//! Inside, the calculator works in its own text: `.` as the decimal point
//! and ASCII operators. On screen the decimal and digit-group separators come
//! from the translation catalog (`1.234,5` in Portuguese, `1,234.5` in
//! English) and the operators are the typographic `×`, `÷`, `−`.

/// Results keep 12 significant digits: enough for any everyday use, and few
/// enough that binary rounding never shows (0.1 + 0.2 is 0.3).
const DIGITS: i32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Separators {
    pub decimal: char,
    pub group: Option<char>,
}

impl Separators {
    /// From the catalog: `calc-decimal-separator`, `calc-group-separator`
    /// (empty for no grouping).
    pub fn from_catalog() -> Self {
        let decimal = orbit_app::i18n::tr("calc-decimal-separator")
            .chars()
            .next()
            .unwrap_or('.');
        let group = orbit_app::i18n::tr("calc-group-separator")
            .chars()
            .next()
            .filter(|&c| c != decimal);
        Self { decimal, group }
    }
}

/// `x` rounded to [`DIGITS`] significant digits, in scientific notation when
/// very large or very small.
pub fn number(x: f64, sep: Separators) -> String {
    if x == 0.0 || !x.is_finite() {
        return "0".into();
    }
    let magnitude = x.abs().log10().floor() as i32;
    let text = if !(-9..15).contains(&magnitude) {
        let s = format!("{:.*e}", (DIGITS - 1) as usize, x);
        let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        format!("{}e{exp:+}", trim_zeros(mantissa))
    } else {
        let decimals = (DIGITS - 1 - magnitude).clamp(0, 20) as usize;
        trim_zeros(&format!("{x:.decimals$}")).to_string()
    };
    let text = if text == "-0" { "0".into() } else { text };
    localize_number(&text, sep)
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// One number in the calculator's text (`-1234.5`, `1.5e+20`) in the user's
/// notation: groups in the integer part, the local decimal separator.
fn localize_number(text: &str, sep: Separators) -> String {
    let (body, exp) = match text.find('e') {
        Some(i) => text.split_at(i),
        None => (text, ""),
    };
    let (sign, body) = match body.strip_prefix('-') {
        Some(rest) => ("−", rest),
        None => ("", body),
    };
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    let mut out = String::from(sign);
    out.push_str(&group(int, sep.group));
    if let Some(frac) = frac {
        out.push(sep.decimal);
        out.push_str(frac);
    }
    out.push_str(&exp.replace('-', "−"));
    out
}

/// `1234567` -> `1.234.567` with `.` as the group separator.
fn group(int: &str, separator: Option<char>) -> String {
    let Some(separator) = separator else {
        return int.to_string();
    };
    let len = int.chars().count();
    let mut out = String::with_capacity(len + len / 3);
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(separator);
        }
        out.push(c);
    }
    out
}

/// The calculator's text as shown: typographic operators, numbers in the
/// user's notation.
pub fn expression(text: &str, sep: Separators) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = chars.peek() {
                if d.is_ascii_digit() || d == '.' {
                    end = j + d.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            out.push_str(&localize_number(&text[i..end], sep));
            continue;
        }
        match c {
            '*' => out.push('×'),
            '/' => out.push('÷'),
            '-' => out.push('−'),
            _ => out.push(c),
        }
    }
    out
}

/// Text pasted or typed in the user's notation, in the calculator's text:
/// group separators dropped, the decimal separator made `.`.
pub fn internal(text: &str, sep: Separators) -> String {
    text.chars()
        .filter(|&c| Some(c) != sep.group)
        .map(|c| if c == sep.decimal { '.' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PT: Separators = Separators {
        decimal: ',',
        group: Some('.'),
    };
    const EN: Separators = Separators {
        decimal: '.',
        group: Some(','),
    };

    #[test]
    fn numbers() {
        assert_eq!(number(0.1 + 0.2, PT), "0,3");
        assert_eq!(number(1234567.5, PT), "1.234.567,5");
        assert_eq!(number(1234567.5, EN), "1,234,567.5");
        assert_eq!(number(-42.0, PT), "−42");
        assert_eq!(number(2.0 / 3.0, PT), "0,666666666667");
        assert_eq!(number(100.0, PT), "100");
        assert_eq!(number(-0.0, PT), "0");
        assert_eq!(number(1e20, PT), "1e+20");
        assert_eq!(number(1.5e-12, PT), "1,5e−12");
        assert_eq!(number(123456789012345.0, PT), "123.456.789.012.345");
        assert_eq!(number(std::f64::consts::PI, EN), "3.14159265359");
    }

    #[test]
    fn expressions() {
        assert_eq!(expression("1234.5*2-3/4", PT), "1.234,5×2−3÷4");
        assert_eq!(expression("sin(30)+√2", EN), "sin(30)+√2");
        assert_eq!(internal("1.234,5×2", PT), "1234.5×2");
        assert_eq!(internal("1,234.5", EN), "1234.5");
    }
}
