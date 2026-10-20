//! Expressions: parsing and evaluation.
//!
//! The usual precedence — `^` above unary minus above `×` `÷` above `+` `−`
//! — with parentheses, postfix `%` and `!`, functions (`sin`, `√`, …), the
//! constants `π` and `e`, and implicit multiplication (`2π`, `3(4+1)`).
//! Parentheses left open at the end are closed, so a half-typed expression
//! still has a value to preview.
//!
//! Percent follows desk calculators: `a + b%` adds b percent *of a*
//! (200 + 10% = 220), while elsewhere `b%` is just b / 100 (50 × 10% = 5).
//!
//! The text is the calculator's own: ASCII operators (`*`, `/`, `-`) and `.`
//! as the decimal point. The typographic forms (`×`, `÷`, `−`) are accepted
//! too, so displayed text can be pasted back.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Angle {
    Degrees,
    Radians,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Malformed or incomplete expression.
    Syntax,
    DivisionByZero,
    /// Outside a function's domain: √−1, ln 0, 2.5!.
    Domain,
    /// Too large to represent.
    Overflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Error::Syntax => "syntax",
            Error::DivisionByZero => "division by zero",
            Error::Domain => "domain",
            Error::Overflow => "overflow",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Ln,
    Log,
    Sqrt,
    Abs,
}

impl Func {
    /// The function's name as typed; longest names first, so `asin` is
    /// matched before `sin`.
    const NAMES: &[(&str, Func)] = &[
        ("asin", Func::Asin),
        ("acos", Func::Acos),
        ("atan", Func::Atan),
        ("sqrt", Func::Sqrt),
        ("sin", Func::Sin),
        ("cos", Func::Cos),
        ("tan", Func::Tan),
        ("log", Func::Log),
        ("abs", Func::Abs),
        ("ln", Func::Ln),
        ("√", Func::Sqrt),
    ];

    fn apply(self, x: f64, angle: Angle) -> Result<f64, Error> {
        let to_rad = |x: f64| match angle {
            Angle::Degrees => x.to_radians(),
            Angle::Radians => x,
        };
        let from_rad = |x: f64| match angle {
            Angle::Degrees => x.to_degrees(),
            Angle::Radians => x,
        };
        let y = match self {
            Func::Sin => exact_trig(to_rad(x), x, angle, f64::sin),
            Func::Cos => exact_trig(to_rad(x), x, angle, f64::cos),
            Func::Tan => {
                // tan 90° (π/2) has no value; in radians the float never
                // lands exactly on π/2, so within rounding counts.
                let odd_right_angle = match angle {
                    Angle::Degrees => (x - 90.0).rem_euclid(180.0) == 0.0,
                    Angle::Radians => {
                        let k = (x - std::f64::consts::FRAC_PI_2) / std::f64::consts::PI;
                        (k - k.round()).abs() < 1e-9
                    }
                };
                if odd_right_angle {
                    return Err(Error::Domain);
                }
                exact_trig(to_rad(x), x, angle, f64::tan)
            }
            Func::Asin | Func::Acos if !(-1.0..=1.0).contains(&x) => return Err(Error::Domain),
            Func::Asin => from_rad(x.asin()),
            Func::Acos => from_rad(x.acos()),
            Func::Atan => from_rad(x.atan()),
            Func::Ln | Func::Log if x <= 0.0 => return Err(Error::Domain),
            Func::Ln => x.ln(),
            Func::Log => x.log10(),
            Func::Sqrt if x < 0.0 => return Err(Error::Domain),
            Func::Sqrt => x.sqrt(),
            Func::Abs => x.abs(),
        };
        Ok(y)
    }
}

/// `f(rad)`, but exactly 0 where the true value is 0 at whole multiples of
/// 90° (π/2): sin 180° and sin 2π are 0, not 1.2e-16 — in degrees when the
/// angle is exact, in radians when it is within rounding of the multiple.
fn exact_trig(rad: f64, x: f64, angle: Angle, f: fn(f64) -> f64) -> f64 {
    let y = f(rad);
    let on_multiple = match angle {
        Angle::Degrees => x.rem_euclid(90.0) == 0.0,
        Angle::Radians => {
            let k = x / std::f64::consts::FRAC_PI_2;
            // At 0 itself the float is exact; tiny angles keep their value.
            k.round() != 0.0 && (k - k.round()).abs() < 1e-9
        }
    };
    if on_multiple && y.abs() < 1e-9 {
        0.0
    } else {
        y
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Plus,
    Minus,
    Times,
    Divide,
    Power,
    Open,
    Close,
    Percent,
    Factorial,
    Func(Func),
}

fn tokenize(text: &str) -> Result<Vec<Token>, Error> {
    let mut tokens = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c.is_whitespace() {
            rest = &rest[c.len_utf8()..];
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let end = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(rest.len());
            let mut end = end;
            // Exponent: 1.5e-3 (only when digits follow, so `2e` is 2 × e).
            let tail = &rest[end..];
            if let Some(exp) = tail.strip_prefix(['e', 'E']) {
                let sign = usize::from(exp.starts_with(['+', '-']));
                let digits = exp[sign..]
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(exp.len() - sign);
                if digits > 0 {
                    end += 1 + sign + digits;
                }
            }
            let number: f64 = rest[..end].parse().map_err(|_| Error::Syntax)?;
            tokens.push(Token::Num(number));
            rest = &rest[end..];
            continue;
        }
        if let Some((name, func)) = Func::NAMES.iter().find(|(name, _)| rest.starts_with(name)) {
            tokens.push(Token::Func(*func));
            rest = &rest[name.len()..];
            continue;
        }
        let token = match c {
            '+' => Token::Plus,
            '-' | '−' => Token::Minus,
            '*' | '×' | '·' => Token::Times,
            '/' | '÷' => Token::Divide,
            '^' => Token::Power,
            '(' => Token::Open,
            ')' => Token::Close,
            '%' => Token::Percent,
            '!' => Token::Factorial,
            'π' => Token::Num(std::f64::consts::PI),
            'e' => Token::Num(std::f64::consts::E),
            _ => return Err(Error::Syntax),
        };
        tokens.push(token);
        rest = &rest[c.len_utf8()..];
    }
    Ok(tokens)
}

#[derive(Debug)]
enum Node {
    Num(f64),
    Neg(Box<Node>),
    Add(Box<Node>, Box<Node>),
    Sub(Box<Node>, Box<Node>),
    Mul(Box<Node>, Box<Node>),
    Div(Box<Node>, Box<Node>),
    Pow(Box<Node>, Box<Node>),
    Percent(Box<Node>),
    Factorial(Box<Node>),
    Call(Func, Box<Node>),
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn sum(&mut self) -> Result<Node, Error> {
        let mut node = self.product()?;
        loop {
            if self.eat(&Token::Plus) {
                node = Node::Add(Box::new(node), Box::new(self.product()?));
            } else if self.eat(&Token::Minus) {
                node = Node::Sub(Box::new(node), Box::new(self.product()?));
            } else {
                return Ok(node);
            }
        }
    }

    fn product(&mut self) -> Result<Node, Error> {
        let mut node = self.unary()?;
        loop {
            if self.eat(&Token::Times) {
                node = Node::Mul(Box::new(node), Box::new(self.unary()?));
            } else if self.eat(&Token::Divide) {
                node = Node::Div(Box::new(node), Box::new(self.unary()?));
            } else if matches!(
                self.peek(),
                Some(Token::Num(_) | Token::Open | Token::Func(_))
            ) {
                // Implicit multiplication: 2π, 3(4+1), 2√9.
                node = Node::Mul(Box::new(node), Box::new(self.power()?));
            } else {
                return Ok(node);
            }
        }
    }

    fn unary(&mut self) -> Result<Node, Error> {
        if self.eat(&Token::Minus) {
            Ok(Node::Neg(Box::new(self.unary()?)))
        } else if self.eat(&Token::Plus) {
            self.unary()
        } else {
            self.power()
        }
    }

    /// Right-associative: 2^3^2 = 2^9. The exponent may carry a sign.
    fn power(&mut self) -> Result<Node, Error> {
        let base = self.postfix()?;
        if self.eat(&Token::Power) {
            Ok(Node::Pow(Box::new(base), Box::new(self.unary()?)))
        } else {
            Ok(base)
        }
    }

    fn postfix(&mut self) -> Result<Node, Error> {
        let mut node = self.primary()?;
        loop {
            if self.eat(&Token::Percent) {
                node = Node::Percent(Box::new(node));
            } else if self.eat(&Token::Factorial) {
                node = Node::Factorial(Box::new(node));
            } else {
                return Ok(node);
            }
        }
    }

    fn primary(&mut self) -> Result<Node, Error> {
        match self.next() {
            Some(Token::Num(n)) => Ok(Node::Num(n)),
            Some(Token::Open) => {
                let inner = self.sum()?;
                // An unclosed parenthesis at the end is closed for the user.
                if !self.eat(&Token::Close) && self.peek().is_some() {
                    return Err(Error::Syntax);
                }
                Ok(inner)
            }
            // sin 30, sin(30), √2: the argument binds like a power base, so
            // √4^2 is √(4^2) and sin 30 + 1 is (sin 30) + 1.
            Some(Token::Func(f)) => Ok(Node::Call(f, Box::new(self.unary_argument()?))),
            _ => Err(Error::Syntax),
        }
    }

    fn unary_argument(&mut self) -> Result<Node, Error> {
        if self.eat(&Token::Minus) {
            Ok(Node::Neg(Box::new(self.unary_argument()?)))
        } else {
            self.power()
        }
    }
}

fn parse(text: &str) -> Result<Node, Error> {
    let mut parser = Parser {
        tokens: tokenize(text)?,
        at: 0,
    };
    let node = parser.sum()?;
    if parser.peek().is_some() {
        return Err(Error::Syntax);
    }
    Ok(node)
}

fn finite(x: f64) -> Result<f64, Error> {
    if x.is_finite() {
        Ok(x)
    } else if x.is_nan() {
        Err(Error::Domain)
    } else {
        Err(Error::Overflow)
    }
}

fn eval(node: &Node, angle: Angle) -> Result<f64, Error> {
    let e = |n: &Node| eval(n, angle);
    let x = match node {
        Node::Num(n) => *n,
        Node::Neg(a) => -e(a)?,
        // a ± b%: b percent of a.
        Node::Add(a, b) | Node::Sub(a, b) => {
            let a_val = e(a)?;
            let b_val = match &**b {
                Node::Percent(p) => a_val * e(p)? / 100.0,
                _ => e(b)?,
            };
            if matches!(node, Node::Add(..)) {
                a_val + b_val
            } else {
                a_val - b_val
            }
        }
        Node::Mul(a, b) => e(a)? * e(b)?,
        Node::Div(a, b) => {
            let d = e(b)?;
            if d == 0.0 {
                return Err(Error::DivisionByZero);
            }
            e(a)? / d
        }
        Node::Pow(a, b) => {
            let (base, exp) = (e(a)?, e(b)?);
            if base == 0.0 && exp < 0.0 {
                return Err(Error::DivisionByZero);
            }
            base.powf(exp)
        }
        Node::Percent(a) => e(a)? / 100.0,
        Node::Factorial(a) => factorial(e(a)?)?,
        Node::Call(f, a) => f.apply(e(a)?, angle)?,
    };
    finite(x)
}

fn factorial(n: f64) -> Result<f64, Error> {
    if n < 0.0 || n.fract() != 0.0 {
        return Err(Error::Domain);
    }
    if n > 170.0 {
        return Err(Error::Overflow);
    }
    Ok((1..=n as u32).fold(1.0, |acc, k| acc * f64::from(k)))
}

/// The value of `text`.
pub fn evaluate(text: &str, angle: Angle) -> Result<f64, Error> {
    eval(&parse(text)?, angle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> f64 {
        evaluate(text, Angle::Degrees).unwrap_or_else(|e| panic!("{text}: {e}"))
    }

    fn err(text: &str) -> Error {
        evaluate(text, Angle::Degrees).expect_err(text)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(v("2+3*4"), 14.0);
        assert_eq!(v("(2+3)*4"), 20.0);
        assert_eq!(v("10-4-3"), 3.0);
        assert_eq!(v("100/10/5"), 2.0);
        assert_eq!(v("2^3^2"), 512.0);
        assert_eq!(v("-2^2"), -4.0);
        assert_eq!(v("2^-1"), 0.5);
        assert_eq!(v("3--2"), 5.0);
        assert_eq!(v("−5 × 2 ÷ 4"), -2.5);
    }

    #[test]
    fn numbers() {
        assert_eq!(v(".5+.25"), 0.75);
        assert_eq!(v("1.5e3"), 1500.0);
        assert_eq!(v("2e-2"), 0.02);
        assert!(close(v("2e"), 2.0 * std::f64::consts::E));
        assert_eq!(err("1.2.3"), Error::Syntax);
    }

    #[test]
    fn percent() {
        assert_eq!(v("200+10%"), 220.0);
        assert_eq!(v("200-10%"), 180.0);
        assert_eq!(v("50*10%"), 5.0);
        assert_eq!(v("50%"), 0.5);
        assert_eq!(v("200/50%"), 400.0);
    }

    #[test]
    fn implicit_multiplication_and_open_parentheses() {
        assert!(close(v("2π"), 2.0 * std::f64::consts::PI));
        assert_eq!(v("3(4+1)"), 15.0);
        assert_eq!(v("(1+2)(3+4)"), 21.0);
        assert_eq!(v("2√9"), 6.0);
        assert_eq!(v("(2+3"), 5.0);
        assert_eq!(v("2*(3+(4"), 14.0);
        assert_eq!(err("2+"), Error::Syntax);
        assert_eq!(err("(2+3))"), Error::Syntax);
        assert_eq!(err(""), Error::Syntax);
    }

    #[test]
    fn functions() {
        assert!(close(v("sin 30"), 0.5));
        assert!(close(v("sin(30)"), 0.5));
        assert_eq!(v("sin 180"), 0.0);
        assert_eq!(v("cos 90"), 0.0);
        assert!(close(v("tan 45"), 1.0));
        assert_eq!(err("tan 90"), Error::Domain);
        assert!(close(v("asin 1"), 90.0));
        let rad = |t: &str| evaluate(t, Angle::Radians);
        assert!(close(rad("sin(π/2)").unwrap(), 1.0));
        assert_eq!(rad("sin(2π)"), Ok(0.0));
        assert_eq!(rad("cos(π/2)"), Ok(0.0));
        assert_eq!(rad("tan(π/2)"), Err(Error::Domain));
        assert_eq!(rad("sin(1e-13)"), Ok(1e-13), "small angles stay");
        assert_eq!(v("√16"), 4.0);
        assert_eq!(v("sqrt(16)+1"), 5.0);
        assert_eq!(v("√4^2"), 4.0, "√ takes the power: √(4^2)");
        assert_eq!(v("log 1000"), 3.0);
        assert!(close(v("ln e"), 1.0));
        assert_eq!(v("abs(-3)"), 3.0);
        assert_eq!(v("5!"), 120.0);
        assert_eq!(v("0!"), 1.0);
        assert_eq!(v("3!^2"), 36.0);
    }

    #[test]
    fn errors() {
        assert_eq!(err("1/0"), Error::DivisionByZero);
        assert_eq!(err("1/(2-2)"), Error::DivisionByZero);
        assert_eq!(err("0^-1"), Error::DivisionByZero);
        assert_eq!(err("√-1"), Error::Domain);
        assert_eq!(err("ln 0"), Error::Domain);
        assert_eq!(err("2.5!"), Error::Domain);
        assert_eq!(err("asin 2"), Error::Domain);
        assert_eq!(err("171!"), Error::Overflow);
        assert_eq!(err("10^400"), Error::Overflow);
        assert_eq!(err("2 $ 3"), Error::Syntax);
    }
}
