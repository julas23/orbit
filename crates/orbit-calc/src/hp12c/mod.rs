//! The financial profile: an RPN calculator after the HP 12c.
//!
//! The engine here is driven by the 39 physical keys, so the keypad on
//! screen and the tests speak the same language as the 12c's manual: the
//! `f` and `g` keys select a key's gold or blue function, `STO` and `RCL`
//! wait for a register. Programming (P/R, R/S, GTO …) is not implemented;
//! its keys do nothing.
//!
//! The stack has four levels (X, Y, Z, T) with LSTx, and the 12c's stack
//! lift: ENTER, CLx and Σ+ leave it disabled, so the next number keyed
//! replaces X instead of pushing it.

mod dates;
mod fin;
pub mod ui;

use dates::Date;
use fin::{Bond, Depreciation, Flows, Tvm};

/// The keys, left to right, top to bottom; ENTER spans the last two rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pad {
    N,
    I,
    Pv,
    Pmt,
    Fv,
    Chs,
    Digit(u8),
    Div,
    Yx,
    Recip,
    PctT,
    DeltaPct,
    Pct,
    Eex,
    Mul,
    Rs,
    Sst,
    RollDown,
    Swap,
    Clx,
    Enter,
    Sub,
    On,
    F,
    G,
    Sto,
    Rcl,
    Point,
    SigmaPlus,
    Add,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shift {
    F,
    G,
}

/// A register being chosen after STO or RCL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Sto { op: Option<Pad>, dot: bool },
    Rcl { dot: bool },
}

/// The number being keyed in.
#[derive(Debug, Clone, Default, PartialEq)]
struct Typing {
    /// Digits and at most one point.
    mantissa: String,
    negative: bool,
    /// The exponent's digits after EEX (at most two) and its sign.
    exponent: Option<(bool, String)>,
}

impl Typing {
    fn value(&self) -> f64 {
        let mantissa = if self.mantissa.is_empty() || self.mantissa == "." {
            "0"
        } else {
            &self.mantissa
        };
        let mut x: f64 = mantissa.parse().unwrap_or(0.0);
        if let Some((neg, digits)) = &self.exponent {
            let e: i32 = digits.parse().unwrap_or(0);
            x *= 10f64.powi(if *neg { -e } else { e });
        }
        if self.negative { -x } else { x }
    }

    fn digit_count(&self) -> usize {
        self.mantissa.chars().filter(char::is_ascii_digit).count()
    }
}

/// The 12c's error numbers: 0 math, 2 statistics, 3 IRR, 5 financial,
/// 6 registers and cash flows, 8 calendar.
pub type ErrorCode = u8;

const MAX: f64 = 9.999_999_999e99;

#[derive(Debug, Clone)]
pub struct Hp12c {
    /// X, Y, Z, T.
    stack: [f64; 4],
    last_x: f64,
    lift: bool,
    typing: Option<Typing>,
    /// R0–R9 and R.0–R.9; R1–R6 also hold the statistics sums.
    regs: [f64; 20],
    n: f64,
    i: f64,
    pv: f64,
    pmt: f64,
    fv: f64,
    begin: bool,
    flows: Flows,
    /// Decimals shown (f 0 … f 9).
    fix: u8,
    /// Scientific notation (f .).
    sci: bool,
    /// D.MY date order instead of M.DY.
    dmy: bool,
    shift: Option<Shift>,
    pending: Option<Pending>,
    error: Option<ErrorCode>,
    /// The last key stored or computed a financial register: the next
    /// financial key computes instead of storing.
    after_fin: bool,
    /// Shown beside a date computed by DATE: 1 = Monday … 7 = Sunday.
    weekday: Option<u8>,
}

impl Default for Hp12c {
    fn default() -> Self {
        Hp12c {
            stack: [0.0; 4],
            last_x: 0.0,
            lift: true,
            typing: None,
            regs: [0.0; 20],
            n: 0.0,
            i: 0.0,
            pv: 0.0,
            pmt: 0.0,
            fv: 0.0,
            begin: false,
            flows: Flows::default(),
            fix: 2,
            sci: false,
            dmy: false,
            shift: None,
            pending: None,
            error: None,
            after_fin: false,
            weekday: None,
        }
    }
}

/// What the display shows besides the number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Annunciators {
    pub f: bool,
    pub g: bool,
    pub begin: bool,
    pub dmy: bool,
    pub weekday: Option<u8>,
}

impl Hp12c {
    pub fn x(&self) -> f64 {
        match &self.typing {
            Some(t) => t.value(),
            None => self.stack[0],
        }
    }

    pub fn annunciators(&self) -> Annunciators {
        Annunciators {
            f: self.shift == Some(Shift::F),
            g: self.shift == Some(Shift::G),
            begin: self.begin,
            dmy: self.dmy,
            weekday: self.weekday,
        }
    }

    /// Keys in a number from outside (a paste), as if typed.
    pub fn paste(&mut self, x: f64) {
        self.error = None;
        self.put(x);
    }

    pub fn error(&self) -> Option<ErrorCode> {
        self.error
    }

    /// Ends number entry, leaving its value in X.
    fn finish(&mut self) {
        if let Some(t) = self.typing.take() {
            self.stack[0] = t.value();
        }
    }

    fn push(&mut self) {
        self.stack = [self.stack[0], self.stack[0], self.stack[1], self.stack[2]];
    }

    /// Puts a value in X, pushing the stack first when lift is enabled.
    fn put(&mut self, value: f64) {
        self.finish();
        if self.lift {
            self.push();
        }
        self.stack[0] = value;
        self.lift = true;
    }

    /// Replaces X with `value`, remembering the old X in LSTx.
    fn unary(&mut self, value: Result<f64, ErrorCode>) {
        match value.and_then(checked) {
            Ok(v) => {
                self.last_x = self.stack[0];
                self.stack[0] = v;
                self.lift = true;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Combines Y and X into X, dropping the stack (T is copied down).
    fn binary(&mut self, op: impl Fn(f64, f64) -> Result<f64, ErrorCode>) {
        let [x, y, z, t] = self.stack;
        match op(y, x).and_then(checked) {
            Ok(v) => {
                self.last_x = x;
                self.stack = [v, z, t, t];
                self.lift = true;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Like `unary`, but Y stays: the percentages.
    fn percent(&mut self, op: impl Fn(f64, f64) -> Result<f64, ErrorCode>) {
        let [x, y, ..] = self.stack;
        self.unary(op(y, x));
    }

    fn tvm(&self) -> Tvm {
        Tvm {
            n: self.n,
            i: self.i,
            pv: self.pv,
            pmt: self.pmt,
            fv: self.fv,
            begin: self.begin,
        }
    }

    pub fn press(&mut self, key: Pad) {
        // An error stays until a key is pressed; that key only clears it.
        if self.error.take().is_some() {
            return;
        }
        self.weekday = None;

        if let Some(pending) = self.pending.take() {
            self.register(pending, key);
            return;
        }
        let shift = self.shift.take();
        let was_fin = std::mem::take(&mut self.after_fin);
        match (shift, key) {
            // The prefix keys: pressing one replaces the other.
            (Some(Shift::F), Pad::F) | (Some(Shift::G), Pad::G) => {}
            (_, Pad::F) => self.shift = Some(Shift::F),
            (_, Pad::G) => self.shift = Some(Shift::G),
            (None, k) => self.plain(k, was_fin),
            (Some(Shift::F), k) => self.gold(k),
            (Some(Shift::G), k) => self.blue(k),
        }
    }

    fn plain(&mut self, key: Pad, was_fin: bool) {
        match key {
            Pad::Digit(d) => self.digit(d),
            Pad::Point => self.point(),
            Pad::Eex => {
                let t = self.typing_mut();
                if t.exponent.is_none() {
                    if t.mantissa.is_empty() {
                        t.mantissa.push('1');
                    }
                    t.exponent = Some((false, String::new()));
                }
            }
            Pad::Chs => match &mut self.typing {
                Some(Typing {
                    exponent: Some((neg, _)),
                    ..
                }) => *neg = !*neg,
                Some(t) => t.negative = !t.negative,
                None => self.stack[0] = -self.stack[0],
            },
            Pad::Enter => {
                self.finish();
                self.push();
                self.lift = false;
            }
            Pad::Clx => {
                self.typing = None;
                self.stack[0] = 0.0;
                self.lift = false;
            }
            Pad::Add | Pad::Sub | Pad::Mul | Pad::Div | Pad::Yx => {
                self.finish();
                self.binary(move |y, x| match key {
                    Pad::Add => Ok(y + x),
                    Pad::Sub => Ok(y - x),
                    Pad::Mul => Ok(y * x),
                    Pad::Div if x == 0.0 => Err(0),
                    Pad::Div => Ok(y / x),
                    _ => power(y, x),
                });
            }
            Pad::Recip => {
                self.finish();
                let x = self.stack[0];
                self.unary(if x == 0.0 { Err(0) } else { Ok(1.0 / x) });
            }
            Pad::Pct => {
                self.finish();
                self.percent(|y, x| Ok(y * x / 100.0));
            }
            Pad::DeltaPct => {
                self.finish();
                self.percent(|y, x| {
                    if y == 0.0 {
                        Err(0)
                    } else {
                        Ok(100.0 * (x - y) / y)
                    }
                });
            }
            Pad::PctT => {
                self.finish();
                self.percent(|y, x| if y == 0.0 { Err(0) } else { Ok(100.0 * x / y) });
            }
            Pad::RollDown => {
                self.finish();
                let [x, y, z, t] = self.stack;
                self.stack = [y, z, t, x];
                self.lift = true;
            }
            Pad::Swap => {
                self.finish();
                self.stack.swap(0, 1);
                self.lift = true;
            }
            Pad::Sto => {
                self.finish();
                self.pending = Some(Pending::Sto {
                    op: None,
                    dot: false,
                });
            }
            Pad::Rcl => {
                self.finish();
                self.pending = Some(Pending::Rcl { dot: false });
            }
            Pad::SigmaPlus => self.sigma(1.0),
            Pad::N | Pad::I | Pad::Pv | Pad::Pmt | Pad::Fv => self.financial(key, was_fin),
            // Programming and power: not implemented.
            Pad::Rs | Pad::Sst | Pad::On | Pad::F | Pad::G => {}
        }
    }

    /// The gold functions, printed above the keys.
    fn gold(&mut self, key: Pad) {
        match key {
            Pad::Digit(d) => {
                self.finish();
                self.fix = d;
                self.sci = false;
            }
            Pad::Point => {
                self.finish();
                self.sci = true;
            }
            Pad::N => self.amortize(),
            Pad::I => {
                self.finish();
                let (i360, i365) = fin::simple_interest(self.n, self.i, self.pv);
                // X: 360-day interest; Y: principal; Z: 365-day interest.
                self.stack = [i360, -self.pv, i365, self.stack[3]];
                self.lift = true;
            }
            Pad::Pv => {
                self.finish();
                let npv = self.flows.npv(self.i);
                self.pv = npv;
                self.put(npv);
            }
            Pad::Pmt => {
                self.finish();
                let x = self.stack[0];
                self.unary(Ok(self.round_to_display(x)));
            }
            Pad::Fv => {
                self.finish();
                match self.flows.irr() {
                    Ok(irr) => {
                        self.i = irr;
                        self.put(irr);
                    }
                    Err(_) => self.error = Some(3),
                }
            }
            Pad::Yx | Pad::Recip => self.bond(key),
            Pad::PctT | Pad::DeltaPct | Pad::Pct => self.depreciate(key),
            // CLEAR Σ: statistics registers and the stack.
            Pad::Sst => {
                self.typing = None;
                self.regs[1..=6].fill(0.0);
                self.stack = [0.0; 4];
            }
            // CLEAR FIN.
            Pad::Swap => {
                self.finish();
                (self.n, self.i, self.pv, self.pmt, self.fv) = (0.0, 0.0, 0.0, 0.0, 0.0);
            }
            // CLEAR REG: everything but the display settings.
            Pad::Clx => {
                *self = Hp12c {
                    fix: self.fix,
                    sci: self.sci,
                    dmy: self.dmy,
                    begin: self.begin,
                    ..Hp12c::default()
                };
            }
            // CLEAR PREFIX (already cleared) and the program functions.
            _ => {}
        }
    }

    /// The blue functions, printed on the keys' lower face.
    fn blue(&mut self, key: Pad) {
        if key != Pad::Sub {
            self.finish();
        }
        let x = self.stack[0];
        match key {
            Pad::N => self.store_fin(Pad::N, x * 12.0),
            Pad::I => self.store_fin(Pad::I, x / 12.0),
            Pad::Pv => {
                self.flows = Flows {
                    cf0: x,
                    groups: Vec::new(),
                };
                self.n = 0.0;
                self.lift = true;
            }
            Pad::Pmt => {
                if self.flows.groups.len() >= 19 {
                    self.error = Some(6);
                } else {
                    self.flows.groups.push((x, 1));
                    self.n = self.flows.groups.len() as f64;
                    self.lift = true;
                }
            }
            Pad::Fv => match self.flows.groups.last_mut() {
                Some(group) if (1.0..=99.0).contains(&x) && x.fract() == 0.0 => {
                    group.1 = x as u32;
                    self.lift = true;
                }
                _ => self.error = Some(6),
            },
            Pad::Chs => self.date(),
            Pad::Digit(7) => self.begin = true,
            Pad::Digit(8) => self.begin = false,
            Pad::Yx => self.unary(if x < 0.0 { Err(0) } else { Ok(x.sqrt()) }),
            Pad::Recip => self.unary(Ok(x.exp())),
            Pad::PctT => self.unary(if x <= 0.0 { Err(0) } else { Ok(x.ln()) }),
            Pad::DeltaPct => self.unary(Ok(x.fract())),
            Pad::Pct => self.unary(Ok(x.trunc())),
            Pad::Eex => self.delta_days(),
            Pad::Digit(4) => self.dmy = true,
            Pad::Digit(5) => self.dmy = false,
            Pad::Digit(6) => self.weighted_mean(),
            Pad::Mul => self.unary(Ok(x * x)),
            Pad::Digit(1) => self.estimate(true),
            Pad::Digit(2) => self.estimate(false),
            Pad::Digit(3) => self.unary(factorial(x)),
            // ←: erases the last digit keyed, or clears X.
            Pad::Sub => match &mut self.typing {
                Some(t) => {
                    match &mut t.exponent {
                        Some((_, digits)) if !digits.is_empty() => {
                            digits.pop();
                        }
                        Some(_) => t.exponent = None,
                        None => {
                            t.mantissa.pop();
                        }
                    }
                    if t.mantissa.is_empty() && t.exponent.is_none() {
                        self.typing = None;
                        self.stack[0] = 0.0;
                        self.lift = false;
                    }
                }
                None => {
                    self.stack[0] = 0.0;
                    self.lift = false;
                }
            },
            Pad::Digit(0) => self.means(),
            Pad::Point => self.deviations(),
            Pad::SigmaPlus => self.sigma(-1.0),
            Pad::Add => self.put(self.last_x),
            // MEM, PSE, BST, GTO, x≤y, x=0 belong to programming.
            _ => {}
        }
    }

    fn typing_mut(&mut self) -> &mut Typing {
        if self.typing.is_none() {
            if self.lift {
                self.push();
            }
            self.lift = true;
            self.typing = Some(Typing::default());
        }
        self.typing.as_mut().expect("just set")
    }

    fn digit(&mut self, d: u8) {
        let t = self.typing_mut();
        let c = char::from(b'0' + d);
        match &mut t.exponent {
            Some((_, digits)) => {
                // Two exponent digits; a third shifts the first out.
                if digits.len() == 2 {
                    digits.remove(0);
                }
                digits.push(c);
            }
            None => {
                if t.digit_count() < 10 {
                    t.mantissa.push(c);
                }
            }
        }
    }

    fn point(&mut self) {
        let t = self.typing_mut();
        if t.exponent.is_none() && !t.mantissa.contains('.') {
            t.mantissa.push('.');
        }
    }

    fn register(&mut self, pending: Pending, key: Pad) {
        match (pending, key) {
            (Pending::Sto { op: None, dot }, Pad::Add | Pad::Sub | Pad::Mul | Pad::Div) if !dot => {
                self.pending = Some(Pending::Sto {
                    op: Some(key),
                    dot: false,
                });
            }
            (Pending::Sto { op, dot: false }, Pad::Point) => {
                self.pending = Some(Pending::Sto { op, dot: true });
            }
            (Pending::Rcl { dot: false }, Pad::Point) => {
                self.pending = Some(Pending::Rcl { dot: true });
            }
            (Pending::Sto { op, dot }, Pad::Digit(d)) => {
                let r = usize::from(d) + if dot { 10 } else { 0 };
                let x = self.stack[0];
                let value = match op {
                    None => Ok(x),
                    Some(Pad::Add) => Ok(self.regs[r] + x),
                    Some(Pad::Sub) => Ok(self.regs[r] - x),
                    Some(Pad::Mul) => Ok(self.regs[r] * x),
                    Some(_) if x == 0.0 => Err(0),
                    Some(_) => Ok(self.regs[r] / x),
                };
                match value.and_then(checked) {
                    Ok(v) => self.regs[r] = v,
                    Err(e) => self.error = Some(e),
                }
                self.lift = true;
            }
            (Pending::Rcl { dot }, Pad::Digit(d)) => {
                let r = usize::from(d) + if dot { 10 } else { 0 };
                self.put(self.regs[r]);
            }
            (
                Pending::Sto {
                    op: None,
                    dot: false,
                },
                k @ (Pad::N | Pad::I | Pad::Pv | Pad::Pmt | Pad::Fv),
            ) => {
                let x = self.stack[0];
                self.store_fin(k, x);
            }
            (Pending::Rcl { dot: false }, k @ (Pad::N | Pad::I | Pad::Pv | Pad::Pmt | Pad::Fv)) => {
                let v = self.fin_value(k);
                self.put(v);
            }
            // STO EEX toggles the compound-interest flag for odd periods on
            // the 12c; not implemented. Anything else cancels.
            _ => {}
        }
    }

    fn fin_value(&self, key: Pad) -> f64 {
        match key {
            Pad::N => self.n,
            Pad::I => self.i,
            Pad::Pv => self.pv,
            Pad::Pmt => self.pmt,
            _ => self.fv,
        }
    }

    fn store_fin(&mut self, key: Pad, value: f64) {
        match key {
            Pad::N => self.n = value,
            Pad::I => self.i = value,
            Pad::Pv => self.pv = value,
            Pad::Pmt => self.pmt = value,
            _ => self.fv = value,
        }
        self.stack[0] = value;
        self.lift = true;
        self.after_fin = true;
    }

    /// n, i, PV, PMT, FV: stores X, or — right after another financial
    /// key — computes the register from the other four.
    fn financial(&mut self, key: Pad, was_fin: bool) {
        let keyed = self.typing.is_some();
        self.finish();
        if keyed || !was_fin {
            let x = self.stack[0];
            self.store_fin(key, x);
            return;
        }
        let tvm = self.tvm();
        let result = match key {
            Pad::N => tvm.solve_n(),
            Pad::I => tvm.solve_i(),
            Pad::Pv => tvm.solve_pv(),
            Pad::Pmt => tvm.solve_pmt(),
            _ => tvm.solve_fv(),
        };
        match result.ok().and_then(|v| checked(v).ok()) {
            Some(v) => {
                if self.lift {
                    self.push();
                }
                self.store_fin(key, v);
            }
            None => self.error = Some(5),
        }
    }

    fn amortize(&mut self) {
        self.finish();
        let payments = self.stack[0];
        if payments <= 0.0 || payments.fract() != 0.0 {
            self.error = Some(5);
            return;
        }
        let decimals = if self.sci { 9 } else { self.fix };
        let (interest, principal, balance) = fin::amortize(&self.tvm(), payments as u32, decimals);
        self.n += payments;
        self.pv = balance;
        // X: interest; Y: principal; Z: the number of payments.
        self.stack = [interest, principal, payments, self.stack[1]];
        self.lift = true;
    }

    fn depreciate(&mut self, key: Pad) {
        self.finish();
        let method = match key {
            Pad::PctT => Depreciation::StraightLine,
            Pad::DeltaPct => Depreciation::SumOfYearsDigits,
            _ => Depreciation::DecliningBalance,
        };
        let year = self.stack[0];
        match fin::depreciate(method, self.pv, self.fv, self.n, self.i, year) {
            Ok((dep, left)) => {
                self.last_x = year;
                // X: the year's depreciation; Y: the depreciable value left.
                self.stack = [dep, left, self.stack[1], self.stack[2]];
                self.lift = true;
            }
            Err(_) => self.error = Some(5),
        }
    }

    /// PRICE (f yˣ) and YTM (f 1/x): settlement date in Y, maturity in X,
    /// coupon in PMT; the yield in i (PRICE) or the price in PV (YTM).
    fn bond(&mut self, key: Pad) {
        self.finish();
        let [x, y, ..] = self.stack;
        let (Some(settlement), Some(maturity)) = (
            Date::from_number(y, self.dmy),
            Date::from_number(x, self.dmy),
        ) else {
            self.error = Some(8);
            return;
        };
        let bond = Bond {
            coupon: self.pmt,
            settlement,
            maturity,
        };
        match key {
            Pad::Yx => match bond.price(self.i) {
                Ok((price, accrued)) => {
                    self.last_x = x;
                    self.pv = -price;
                    self.stack = [price, accrued, self.stack[2], self.stack[3]];
                    self.lift = true;
                }
                Err(_) => self.error = Some(5),
            },
            _ => match bond.yield_to_maturity(self.pv.abs()) {
                Ok(ytm) => {
                    self.last_x = x;
                    self.i = ytm;
                    self.stack[0] = ytm;
                    self.lift = true;
                }
                Err(_) => self.error = Some(5),
            },
        }
    }

    /// DATE: the date in Y moved by X days.
    fn date(&mut self) {
        let [x, y, ..] = self.stack;
        let result = Date::from_number(y, self.dmy)
            .filter(|_| x.fract() == 0.0)
            .and_then(|d| d.add_days(x as i64));
        match result {
            Some(d) => {
                self.weekday = Some(d.weekday());
                let value = d.to_number(self.dmy);
                self.binary(|_, _| Ok(value));
            }
            None => self.error = Some(8),
        }
    }

    /// ΔDYS: days between the dates in Y and X; actual in X, 30/360 in Y.
    fn delta_days(&mut self) {
        let [x, y, z, _] = self.stack;
        match (
            Date::from_number(y, self.dmy),
            Date::from_number(x, self.dmy),
        ) {
            (Some(a), Some(b)) => {
                self.last_x = x;
                self.stack = [
                    dates::actual_days(a, b) as f64,
                    dates::days_360(a, b) as f64,
                    z,
                    z,
                ];
                self.lift = true;
            }
            _ => self.error = Some(8),
        }
    }

    /// Σ+ (sign 1) and Σ− (−1): x and y into R1–R6.
    fn sigma(&mut self, sign: f64) {
        self.finish();
        let [x, y, ..] = self.stack;
        for (r, v) in [(1, 1.0), (2, x), (3, x * x), (4, y), (5, y * y), (6, x * y)] {
            self.regs[r] += sign * v;
        }
        self.last_x = x;
        self.stack[0] = self.regs[1];
        self.lift = false;
    }

    fn sums(&self) -> (f64, f64, f64, f64, f64, f64) {
        let r = &self.regs;
        (r[1], r[2], r[3], r[4], r[5], r[6])
    }

    /// x̄ (g 0): mean of x in X, of y in Y.
    fn means(&mut self) {
        let (n, sx, _, sy, _, _) = self.sums();
        if n == 0.0 {
            self.error = Some(2);
            return;
        }
        self.put(sy / n);
        self.put(sx / n);
    }

    /// s (g .): sample standard deviations of x in X, of y in Y.
    fn deviations(&mut self) {
        let (n, sx, sxx, sy, syy, _) = self.sums();
        let var = |s: f64, ss: f64| (n * ss - s * s) / (n * (n - 1.0));
        let (vx, vy) = (var(sx, sxx), var(sy, syy));
        if n < 2.0 || vx < 0.0 || vy < 0.0 {
            self.error = Some(2);
            return;
        }
        self.put(vy.sqrt());
        self.put(vx.sqrt());
    }

    /// x̂,r (g 1) and ŷ,r (g 2): the linear estimate for the value in X,
    /// with the correlation coefficient in Y.
    fn estimate(&mut self, of_x: bool) {
        let (n, sx, sxx, sy, syy, sxy) = self.sums();
        let m = n * sxx - sx * sx;
        let p = n * syy - sy * sy;
        let q = n * sxy - sx * sy;
        if n < 2.0 || m <= 0.0 || p <= 0.0 {
            self.error = Some(2);
            return;
        }
        let b = q / m;
        let a = (sy - b * sx) / n;
        let r = q / (m * p).sqrt();
        let x = self.stack[0];
        let estimate = if of_x { (x - a) / b } else { a + b * x };
        self.last_x = x;
        self.stack[0] = r;
        self.lift = true;
        self.put(estimate);
    }

    /// x̄w (g 6): Σxy / Σx — values keyed as y, weights as x.
    fn weighted_mean(&mut self) {
        let (_, sx, _, _, _, sxy) = self.sums();
        if sx == 0.0 {
            self.error = Some(2);
            return;
        }
        self.put(sxy / sx);
    }

    fn round_to_display(&self, x: f64) -> f64 {
        if self.sci {
            let e = x.abs().log10().floor() as i32;
            let scale = 10f64.powi(6 - e);
            (x * scale).round() / scale
        } else {
            fin::round_to(x, self.fix)
        }
    }

    /// The display: the number being keyed, or X in the display format, or
    /// "Error n". `decimal` and `group` are the user's separators.
    pub fn display(&self, decimal: char, group: Option<char>) -> String {
        if let Some(e) = self.error {
            return format!("Error {e}");
        }
        let localize = |s: &str| -> String {
            let (sign, s) = match s.strip_prefix('-') {
                Some(rest) => ("−", rest),
                None => ("", s),
            };
            let (int, frac) = match s.split_once('.') {
                Some((i, f)) => (i, Some(f)),
                None => (s, None),
            };
            let mut out = String::from(sign);
            let len = int.len();
            for (k, c) in int.chars().enumerate() {
                if let Some(g) = group
                    && k > 0
                    && (len - k).is_multiple_of(3)
                {
                    out.push(g);
                }
                out.push(c);
            }
            if let Some(frac) = frac {
                out.push(decimal);
                out.push_str(frac);
            }
            out
        };
        if let Some(t) = &self.typing {
            let mantissa = if t.mantissa.is_empty() {
                "0".to_string()
            } else if t.mantissa.starts_with('.') {
                format!("0{}", t.mantissa)
            } else {
                t.mantissa.clone()
            };
            let mut s = localize(&format!("{}{mantissa}", if t.negative { "-" } else { "" }));
            if let Some((neg, digits)) = &t.exponent {
                s.push_str(&format!(" {}{digits:0>2}", if *neg { "−" } else { "" }));
            }
            return s;
        }
        let x = self.stack[0];
        let sci = |x: f64| {
            let s = format!("{x:.6e}");
            let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
            let e: i32 = e.parse().unwrap_or(0);
            format!(
                "{} {}{:02}",
                localize(m),
                if e < 0 { "−" } else { "" },
                e.abs()
            )
        };
        if x == 0.0 {
            return localize(&format!("{:.*}", usize::from(self.fix), 0.0));
        }
        if self.sci || x.abs() >= 1e10 {
            return sci(x);
        }
        // Ten digits in all: fewer decimals when the integer part is long.
        let int_digits = (x.abs().log10().floor() as i32 + 1).max(1);
        let decimals = (i32::from(self.fix)).min(10 - int_digits).max(0) as usize;
        // Rounded half away from zero first, as the 12c does (format! would
        // round 2.5 to even, 2).
        let text = format!("{:.decimals$}", fin::round_to(x, decimals as u8));
        if text
            .trim_start_matches('-')
            .trim_matches(['0', '.'])
            .is_empty()
            || text
                .trim_start_matches('-')
                .split('.')
                .next()
                .map_or(0, str::len)
                > 10
        {
            // Rounds to zero in this format, or grew past ten digits.
            return sci(x);
        }
        localize(&text)
    }
}

fn checked(x: f64) -> Result<f64, ErrorCode> {
    if x.is_nan() {
        Err(0)
    } else if x.abs() > MAX {
        // The 12c shows all nines instead of failing.
        Ok(MAX.copysign(x))
    } else {
        Ok(x)
    }
}

fn power(y: f64, x: f64) -> Result<f64, ErrorCode> {
    if y == 0.0 && x <= 0.0 || y < 0.0 && x.fract() != 0.0 {
        Err(0)
    } else {
        Ok(y.powf(x))
    }
}

fn factorial(x: f64) -> Result<f64, ErrorCode> {
    if x < 0.0 || x.fract() != 0.0 {
        return Err(0);
    }
    if x > 69.0 {
        return Ok(MAX);
    }
    Ok((1..=x as u32).fold(1.0, |acc, k| acc * f64::from(k)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Presses keys written as in the manual: digits and `.`, `E` ENTER,
    /// `+ - * /`, `f` `g`, and named keys in brackets: `[n]`, `[PV]`,
    /// `[CHS]`, `[STO]`, `[RCL]`, `[x<>y]`, `[R]` (R↓), `[CLx]`, `[%]`,
    /// `[%T]`, `[D%]`, `[yx]`, `[1/x]`, `[EEX]`, `[S+]` (Σ+).
    fn keys(c: &mut Hp12c, text: &str) {
        let mut rest = text;
        while let Some(ch) = rest.chars().next() {
            if ch == '[' {
                let end = rest.find(']').expect("closing ]");
                let key = match &rest[1..end] {
                    "n" => Pad::N,
                    "i" => Pad::I,
                    "PV" => Pad::Pv,
                    "PMT" => Pad::Pmt,
                    "FV" => Pad::Fv,
                    "CHS" => Pad::Chs,
                    "STO" => Pad::Sto,
                    "RCL" => Pad::Rcl,
                    "x<>y" => Pad::Swap,
                    "R" => Pad::RollDown,
                    "CLx" => Pad::Clx,
                    "%" => Pad::Pct,
                    "%T" => Pad::PctT,
                    "D%" => Pad::DeltaPct,
                    "yx" => Pad::Yx,
                    "1/x" => Pad::Recip,
                    "EEX" => Pad::Eex,
                    "S+" => Pad::SigmaPlus,
                    other => panic!("unknown key [{other}]"),
                };
                c.press(key);
                rest = &rest[end + 1..];
                continue;
            }
            let key = match ch {
                '0'..='9' => Pad::Digit(ch as u8 - b'0'),
                '.' => Pad::Point,
                'E' => Pad::Enter,
                '+' => Pad::Add,
                '-' => Pad::Sub,
                '*' => Pad::Mul,
                '/' => Pad::Div,
                'f' => Pad::F,
                'g' => Pad::G,
                ' ' => {
                    rest = &rest[1..];
                    continue;
                }
                other => panic!("unknown key {other}"),
            };
            c.press(key);
            rest = &rest[ch.len_utf8()..];
        }
    }

    fn run(text: &str) -> Hp12c {
        let mut c = Hp12c::default();
        keys(&mut c, text);
        c
    }

    fn near(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn rpn_and_stack_lift() {
        assert_eq!(run("3E4+").x(), 7.0);
        assert_eq!(run("3E4+5*").x(), 35.0);
        assert_eq!(run("10E2/").x(), 5.0);
        assert_eq!(run("2E3[yx]").x(), 8.0);
        // ENTER copies X up; the next number replaces the copy.
        assert_eq!(run("5EE*").x(), 25.0);
        // After an operation the next number pushes.
        let c = run("1E2+ 5");
        assert_eq!((c.stack[1], c.x()), (3.0, 5.0));
        // CLx disables lift.
        assert_eq!(run("7E8[CLx]9+").x(), 16.0);
        assert_eq!(run("1E2[x<>y]-").x(), 1.0);
        let c = run("1E2E3E4[R]");
        assert_eq!(c.stack, [3.0, 2.0, 1.0, 4.0]);
        // T is copied down when the stack drops.
        let c = run("1E2E3E4+");
        assert_eq!(c.stack, [7.0, 2.0, 1.0, 1.0]);
        assert_eq!(run("5[CHS]E3+").x(), -2.0);
        assert_eq!(run("1.5[EEX]3").x(), 1500.0);
        assert_eq!(run("2[EEX]3[CHS]").x(), 0.002);
        assert_eq!(run("6E3/ g+").x(), 3.0, "LSTx");
    }

    #[test]
    fn percentages() {
        let c = run("300E15[%]");
        assert_eq!((c.x(), c.stack[1]), (45.0, 300.0), "Y stays");
        assert_eq!(run("300E15[%]+").x(), 345.0);
        assert_eq!(run("50E60[D%]").x(), 20.0);
        assert_eq!(run("200E50[%T]").x(), 25.0);
    }

    #[test]
    fn blue_functions() {
        assert_eq!(run("16 g[yx]").x(), 4.0);
        assert!(near(run("1 g[1/x]").x(), std::f64::consts::E, 1e-12));
        assert_eq!(run("5 g3").x(), 120.0);
        assert_eq!(run("3 g*").x(), 9.0);
        assert!(near(run("3.75 g[D%]").x(), 0.75, 1e-12));
        assert_eq!(run("3.75 g[%]").x(), 3.0);
        assert_eq!(run("123E g-").x(), 0.0, "← with nothing typed clears X");
        assert_eq!(run("123 g-").x(), 12.0);
        assert_eq!(run("123 g- 4").x(), 124.0, "← erases a digit");
    }

    #[test]
    fn registers() {
        assert_eq!(run("12[STO]3 [CLx] [RCL]3").x(), 12.0);
        assert_eq!(run("12[STO]3 5[STO]+3 [RCL]3").x(), 17.0);
        assert_eq!(run("7[STO].4 [CLx] [RCL].4").x(), 7.0);
        assert_eq!(run("10[STO]1 0[STO]/1").error(), Some(0));
    }

    #[test]
    fn errors_clear_on_the_next_key() {
        let mut c = run("1E0/");
        assert_eq!(c.error(), Some(0));
        assert_eq!(c.display(',', Some('.')), "Error 0");
        keys(&mut c, "5");
        assert_eq!(c.error(), None);
        assert_eq!(c.display(',', Some('.')), "0,00", "the key only cleared");
    }

    #[test]
    fn financial_registers() {
        // Store n, i, PV, then compute PMT.
        let c = run("360[n] .5[i] 100000[PV] [PMT]");
        assert!(near(c.x(), -599.55, 0.005), "{}", c.x());
        // The same with 12× and 12÷.
        let c = run("30 g[n] 6 g[i] 100000[PV] 0[FV] [PMT]");
        assert!(near(c.x(), -599.55, 0.005), "{}", c.x());
        // Solve i.
        let c = run("36[n] 10000[PV] 332.14[CHS][PMT] 0[FV] [i]");
        assert!(near(c.x(), 1.0, 0.001), "{}", c.x());
        // A value keyed is stored, not computed.
        let c = run("36[n] 48[n]");
        assert_eq!(c.n, 48.0);
        // BEGIN.
        let c = run("g7 36[n] 1[i] 10000[CHS][PV] 0[FV] [PMT]");
        assert!(c.annunciators().begin);
        assert!(near(c.x(), 328.85, 0.005), "{}", c.x());
        // No solution.
        assert_eq!(
            run("10[n] 0[i] 100[PV] 10[PMT] 100[FV] [i]").error(),
            Some(5)
        );
        // f CLEAR FIN.
        let c = run("36[n] f[x<>y] [RCL][n]");
        assert_eq!(c.x(), 0.0);
    }

    #[test]
    fn amortization_and_interest() {
        let mut c = run("13.25 g[i] 50000[PV] 562.89[CHS][PMT] 12 f[n]");
        assert!(near(c.x(), -6616.82, 0.005), "{}", c.x());
        keys(&mut c, "[x<>y]");
        assert!(near(c.x(), -137.86, 0.005), "{}", c.x());
        assert_eq!(c.n, 12.0);
        // f INT: 60 days, 7%, 450.
        let c = run("60[n] 7[i] 450[CHS][PV] f[i]");
        assert!(near(c.x(), 5.25, 1e-9));
    }

    #[test]
    fn cash_flows() {
        let c = run("1000[CHS]g[PV] 1100g[PMT] f[FV]");
        assert!(near(c.x(), 10.0, 1e-9), "{}", c.x());
        assert!(near(c.i, 10.0, 1e-9));
        let c = run("10[i] 1000[CHS]g[PV] 500g[PMT] 2g[FV] f[PV]");
        assert!(near(c.x(), -132.23, 0.005), "{}", c.x());
        assert_eq!(run("1g[PV] 5g[PMT] 100g[FV]").error(), Some(6));
    }

    #[test]
    fn statistics() {
        let c = run("2E1[S+] 4E3[S+] 6E5[S+] g0");
        // x = 1, 3, 5; y = 2, 4, 6.
        assert_eq!((c.x(), c.stack[1]), (3.0, 4.0));
        let c = run("2E1[S+] 4E3[S+] 6E5[S+] g.");
        assert_eq!((c.x(), c.stack[1]), (2.0, 2.0));
        let c = run("2E1[S+] 4E3[S+] 6E5[S+] 7 g2");
        assert!(near(c.x(), 8.0, 1e-9), "ŷ for x = 7");
        assert!(near(c.stack[1], 1.0, 1e-9), "r");
        assert_eq!(run("g0").error(), Some(2));
    }

    #[test]
    fn dates() {
        // D.MY: 14 May 2004 plus 120 days is 11 September 2004, a Saturday.
        let c = run("g4 14.052004E 120 g[CHS]");
        assert!(near(c.x(), 11.092004, 1e-9), "{}", c.x());
        assert_eq!(c.annunciators().weekday, Some(6));
        // M.DY (the default): ΔDYS from 3 June 2003 to 14 October 2004.
        let c = run("6.032003E 10.142004 g[EEX]");
        assert_eq!((c.x(), c.stack[1]), (499.0, 491.0));
        assert_eq!(run("13.012004E 1 g[CHS]").error(), Some(8));
    }

    #[test]
    fn display() {
        let d = |text: &str| run(text).display(',', Some('.'));
        assert_eq!(d(""), "0,00");
        assert_eq!(d("1234.567E"), "1.234,57");
        assert_eq!(d("1234.5"), "1.234,5", "as keyed");
        assert_eq!(d("f4 2E3/"), "0,6667");
        assert_eq!(d("f9 2E3/"), "0,666666667");
        assert_eq!(d("123456789E"), "123.456.789,0", "ten digits in all");
        assert_eq!(d("99999E 999999*"), "9,999890 10");
        assert_eq!(d("0.001E"), "1,000000 −03", "too small for FIX 2");
        assert_eq!(d("f. 1234E"), "1,234000 03");
        assert_eq!(d("2[EEX]5"), "2 05");
        assert_eq!(d("5[CHS]"), "−5");
        assert_eq!(d("f0 2.5E"), "3");
    }
}
