//! What the keys do to the expression being typed.
//!
//! The expression is kept as text (see expr.rs) and evaluated on `=`; until
//! then its value is previewed as it is typed. Keys that would make it
//! malformed are refused or adjusted, like on a desk calculator: a second
//! decimal point in a number is ignored, a new operator replaces the one
//! before it, `)` needs an open `(`.

use crate::expr::{self, Angle, Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Digit(char),
    Point,
    /// `+`, `-`, `*`, `/` or `^`.
    Op(char),
    Open,
    Close,
    Percent,
    Factorial,
    /// A function name, typed with its `(`: `sin`, `√`.
    Func(&'static str),
    /// Text typed as is: `π`, `e^`.
    Text(&'static str),
    /// Text that applies to the value before it: `^2`, `^-1`.
    Postfix(&'static str),
    Back,
    Clear,
    /// Changes the sign of the number being typed.
    Sign,
    Equals,
}

/// After `=`: the expression and its value, until the next key.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub expression: String,
    pub value: f64,
}

#[derive(Debug, Default)]
pub struct Entry {
    text: String,
    done: Option<Done>,
    error: Option<Error>,
}

const OPS: &[char] = &['+', '-', '*', '/', '^'];

impl Entry {
    /// The expression being typed (empty after `=`).
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn done(&self) -> Option<&Done> {
        self.done.as_ref()
    }

    /// Why the last `=` gave no value.
    pub fn error(&self) -> Option<Error> {
        self.error
    }

    /// The value of the expression so far, when it has one.
    pub fn preview(&self, angle: Angle) -> Option<f64> {
        if self.text.is_empty() {
            return None;
        }
        expr::evaluate(&self.text, angle).ok()
    }

    /// Replaces the expression with `text` (already in the calculator's own
    /// notation), as when pasting.
    pub fn set_text(&mut self, text: &str) {
        self.done = None;
        self.error = None;
        self.text = text.trim().to_string();
    }

    pub fn press(&mut self, key: Key, angle: Angle) {
        self.error = None;
        if let Some(done) = self.done.take() {
            match key {
                // An operator continues from the result.
                Key::Op(_) | Key::Percent | Key::Factorial | Key::Postfix(_) => {
                    self.text = plain(done.value)
                }
                Key::Sign => {
                    self.text = plain(-done.value);
                    return;
                }
                Key::Back => {
                    self.text = plain(done.value);
                    return;
                }
                // Anything else starts afresh.
                _ => self.text.clear(),
            }
        }
        let last = self.text.chars().last();
        match key {
            Key::Digit(d) => {
                // "0" then "5" is "5", not "05".
                if self.current_number() == "0" {
                    self.text.pop();
                }
                self.text.push(d);
            }
            Key::Point => {
                let number = self.current_number();
                if number.contains('.') {
                    return;
                }
                if number.is_empty() {
                    self.text.push('0');
                }
                self.text.push('.');
            }
            Key::Op(op) => match last {
                None if op == '-' => self.text.push('-'),
                None => {
                    self.text.push('0');
                    self.text.push(op);
                }
                Some('(') if op == '-' => self.text.push('-'),
                Some('(') => {}
                // 5×− : a negative number follows.
                Some(prev) if op == '-' && matches!(prev, '*' | '/' | '^') => self.text.push('-'),
                Some(prev) if OPS.contains(&prev) => {
                    // Replace the operator (and a sign after it): 5×− then + is 5+.
                    while self.text.ends_with(OPS) {
                        self.text.pop();
                    }
                    if self.text.is_empty() || self.text.ends_with('(') {
                        if op == '-' {
                            self.text.push('-');
                        }
                        return;
                    }
                    self.text.push(op);
                }
                Some(_) => self.text.push(op),
            },
            Key::Open => self.text.push('('),
            Key::Close => {
                let open = self.text.matches('(').count();
                let closed = self.text.matches(')').count();
                if open > closed && last.is_some_and(ends_value) {
                    self.text.push(')');
                }
            }
            Key::Percent | Key::Factorial => {
                if last.is_some_and(ends_value) {
                    self.text.push(if key == Key::Percent { '%' } else { '!' });
                }
            }
            Key::Func(name) => {
                self.text.push_str(name);
                self.text.push('(');
            }
            Key::Text(text) => self.text.push_str(text),
            Key::Postfix(text) => {
                if last.is_some_and(ends_value) {
                    self.text.push_str(text);
                }
            }
            Key::Back => {
                // A function goes as a whole: "sin(" is one key.
                if self.text.ends_with('(') {
                    self.text.pop();
                    let name_len = self
                        .text
                        .chars()
                        .rev()
                        .take_while(|c| c.is_ascii_alphabetic() || *c == '√')
                        .map(char::len_utf8)
                        .sum::<usize>();
                    self.text.truncate(self.text.len() - name_len);
                } else {
                    self.text.pop();
                }
            }
            Key::Clear => self.text.clear(),
            Key::Sign => self.toggle_sign(),
            Key::Equals => {
                if self.text.is_empty() {
                    return;
                }
                match expr::evaluate(&self.text, angle) {
                    Ok(value) => {
                        self.done = Some(Done {
                            expression: std::mem::take(&mut self.text),
                            value,
                        });
                    }
                    Err(err) => self.error = Some(err),
                }
            }
        }
    }

    /// The digits and point of the number at the end of the text.
    fn current_number(&self) -> &str {
        let start = self
            .text
            .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
            .map_or(0, |i| i + 1);
        &self.text[start..]
    }

    /// Negates the number at the end: flips the sign written before it, or
    /// the + or − that joins it to what precedes.
    fn toggle_sign(&mut self) {
        let number = self.current_number();
        if number.is_empty() {
            return;
        }
        let start = self.text.len() - number.len();
        let before = &self.text[..start];
        match before.chars().last() {
            // A sign of its own: at the start, or after an operator or "(".
            Some('-')
                if before.len() == 1
                    || before[..before.len() - 1].ends_with(OPS)
                    || before[..before.len() - 1].ends_with('(') =>
            {
                self.text.remove(start - 1);
            }
            Some('-') => self.text.replace_range(start - 1..start, "+"),
            Some('+') => self.text.replace_range(start - 1..start, "-"),
            None => self.text.insert(0, '-'),
            Some('(' | '*' | '/' | '^') => self.text.insert(start, '-'),
            Some(_) => {}
        }
    }
}

/// Whether text ending in `c` ends with a value (an operator can follow).
fn ends_value(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '.' | ')' | '%' | '!' | 'π' | 'e')
}

/// A value as the calculator's text, at full precision. Rust never writes
/// an f64 in exponent form here, so the keys can go on editing it.
fn plain(value: f64) -> String {
    format!("{value}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(keys: &str) -> Entry {
        let mut e = Entry::default();
        for c in keys.chars() {
            let key = match c {
                '0'..='9' => Key::Digit(c),
                '.' => Key::Point,
                '+' | '-' | '*' | '/' | '^' => Key::Op(c),
                '(' => Key::Open,
                ')' => Key::Close,
                '%' => Key::Percent,
                '!' => Key::Factorial,
                '<' => Key::Back,
                'C' => Key::Clear,
                '~' => Key::Sign,
                '=' => Key::Equals,
                's' => Key::Func("sin"),
                'r' => Key::Func("√"),
                'p' => Key::Text("π"),
                'q' => Key::Postfix("^2"),
                _ => panic!("no key for {c}"),
            };
            e.press(key, Angle::Degrees);
        }
        e
    }

    fn value(keys: &str) -> f64 {
        typed(keys).done().expect(keys).value
    }

    #[test]
    fn typing() {
        assert_eq!(typed("12+3").text(), "12+3");
        assert_eq!(typed("1.2.3").text(), "1.23", "one point per number");
        assert_eq!(typed(".5").text(), "0.5");
        assert_eq!(typed("007").text(), "7");
        assert_eq!(typed("5+*").text(), "5*", "operator replaced");
        assert_eq!(typed("5*-").text(), "5*-", "negative factor");
        assert_eq!(typed("5*-+").text(), "5+");
        assert_eq!(typed("*3").text(), "0*3");
        assert_eq!(typed("-3").text(), "-3");
        assert_eq!(typed("2)").text(), "2", "nothing to close");
        assert_eq!(typed("(2)").text(), "(2)");
        assert_eq!(typed("+%").text(), "0+");
        assert_eq!(typed("s30<").text(), "sin(3");
        assert_eq!(typed("s<").text(), "", "a function is erased whole");
        assert_eq!(typed("12C").text(), "");
    }

    #[test]
    fn sign() {
        assert_eq!(typed("5~").text(), "-5");
        assert_eq!(typed("5~~").text(), "5");
        assert_eq!(typed("3+5~").text(), "3-5");
        assert_eq!(typed("3-5~").text(), "3+5");
        assert_eq!(typed("3*5~").text(), "3*-5");
        assert_eq!(typed("3*5~~").text(), "3*5");
        assert_eq!(typed("(5~").text(), "(-5");
        assert_eq!(typed("~").text(), "");
    }

    #[test]
    fn equals_and_after() {
        assert_eq!(value("2+3*4="), 14.0);
        assert_eq!(value("200+10%="), 220.0);
        assert_eq!(value("2+3=*2="), 10.0, "operator continues from the result");
        assert_eq!(typed("2+3=7").text(), "7", "digit starts afresh");
        assert_eq!(value("2+3=~="), -5.0);
        assert_eq!(value("(2+3=").to_string(), "5");
        assert!((value("s30=") - 0.5).abs() < 1e-12);
        assert_eq!(value("r16="), 4.0);
        let e = typed("1/0=");
        assert_eq!(e.error(), Some(Error::DivisionByZero));
        assert_eq!(e.text(), "1/0", "kept to fix");
        assert_eq!(typed("=").done(), None);
        assert_eq!(typed("2+3").preview(Angle::Degrees), Some(5.0));
        assert_eq!(typed("2+").preview(Angle::Degrees), None);
        assert!((value("2p=") - 2.0 * std::f64::consts::PI).abs() < 1e-12);
        assert_eq!(value("3q="), 9.0);
        assert_eq!(typed("q").text(), "", "x² needs a value");
        assert_eq!(value("2+3=q="), 25.0, "x² continues from the result");
    }
}
