//! The 12c's financial functions, as pure arithmetic.
//!
//! Time value of money uses the 12c's cash-flow sign convention — money
//! received positive, money paid negative — and its equation:
//!
//! ```text
//! PV·(1+i)^n + PMT·(1+i·S)·((1+i)^n − 1)/i + FV = 0
//! ```
//!
//! with `i` per period as a fraction and S = 1 for payments at the beginning
//! of each period (BEGIN), 0 at the end (END).

use super::dates::{self, Date};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tvm {
    pub n: f64,
    /// Percent per period, as keyed: 1 means 1%.
    pub i: f64,
    pub pv: f64,
    pub pmt: f64,
    pub fv: f64,
    pub begin: bool,
}

/// No solution (the 12c's "Error 5").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoSolution;

impl Tvm {
    fn rate(&self) -> f64 {
        self.i / 100.0
    }

    /// (1+i)^n and the annuity factor (1+i·S)·((1+i)^n − 1)/i.
    fn factors(&self, i: f64) -> (f64, f64) {
        let growth = (1.0 + i).powf(self.n);
        let s = if self.begin { 1.0 } else { 0.0 };
        let annuity = if i.abs() < 1e-12 {
            self.n
        } else {
            (1.0 + i * s) * (growth - 1.0) / i
        };
        (growth, annuity)
    }

    fn residual(&self, i: f64) -> f64 {
        let (growth, annuity) = self.factors(i);
        self.pv * growth + self.pmt * annuity + self.fv
    }

    pub fn solve_pv(&self) -> Result<f64, NoSolution> {
        let (growth, annuity) = self.factors(self.rate());
        finite(-(self.pmt * annuity + self.fv) / growth)
    }

    pub fn solve_pmt(&self) -> Result<f64, NoSolution> {
        let (growth, annuity) = self.factors(self.rate());
        if annuity == 0.0 {
            return Err(NoSolution);
        }
        finite(-(self.pv * growth + self.fv) / annuity)
    }

    pub fn solve_fv(&self) -> Result<f64, NoSolution> {
        let (growth, annuity) = self.factors(self.rate());
        finite(-(self.pv * growth + self.pmt * annuity))
    }

    /// Rounded up to a whole number of periods, as the 12c does (unless
    /// within 0.005 of the integer below).
    pub fn solve_n(&self) -> Result<f64, NoSolution> {
        let i = self.rate();
        let n = if i.abs() < 1e-12 {
            if self.pmt == 0.0 {
                return Err(NoSolution);
            }
            -(self.pv + self.fv) / self.pmt
        } else {
            let s = if self.begin { 1.0 } else { 0.0 };
            let a = self.pmt * (1.0 + i * s) / i;
            let ratio = (a - self.fv) / (self.pv + a);
            if ratio <= 0.0 || !ratio.is_finite() {
                return Err(NoSolution);
            }
            ratio.ln() / (1.0 + i).ln()
        };
        if !n.is_finite() || n <= 0.0 {
            return Err(NoSolution);
        }
        Ok(if n - n.floor() < 0.005 {
            n.floor()
        } else {
            n.ceil()
        })
    }

    /// The rate, in percent per period.
    pub fn solve_i(&self) -> Result<f64, NoSolution> {
        if self.n <= 0.0 {
            return Err(NoSolution);
        }
        root(|i| self.residual(i)).map(|i| i * 100.0)
    }
}

fn finite(x: f64) -> Result<f64, NoSolution> {
    if x.is_finite() {
        Ok(x)
    } else {
        Err(NoSolution)
    }
}

/// A root of `f` on (−100%, +∞): the sign changes of `f` over a grid of
/// rates are bracketed and the one nearest zero is refined by bisection.
fn root(f: impl Fn(f64) -> f64) -> Result<f64, NoSolution> {
    // Rates from −99% to 1000% per period, denser near 0.
    let mut grid: Vec<f64> = Vec::new();
    let mut r = 1e-6;
    while r < 10.0 {
        grid.push(r);
        grid.push(-r);
        r *= 1.15;
    }
    grid.push(0.0);
    grid.push(-0.99);
    grid.retain(|&r| r > -0.99999);
    grid.sort_by(f64::total_cmp);
    let values: Vec<f64> = grid.iter().map(|&r| f(r)).collect();
    let mut best: Option<(f64, f64)> = None;
    for k in 0..grid.len() - 1 {
        let (a, b) = (values[k], values[k + 1]);
        if a == 0.0 {
            return Ok(grid[k]);
        }
        if a.is_finite() && b.is_finite() && (a < 0.0) != (b < 0.0) {
            let pair = (grid[k], grid[k + 1]);
            let nearer = best.is_none_or(|(lo, _)| pair.0.abs() < lo.abs());
            if nearer {
                best = Some(pair);
            }
        }
    }
    let (mut lo, mut hi) = best.ok_or(NoSolution)?;
    let mut f_lo = f(lo);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let f_mid = f(mid);
        if f_mid == 0.0 {
            return Ok(mid);
        }
        if (f_mid < 0.0) == (f_lo < 0.0) {
            lo = mid;
            f_lo = f_mid;
        } else {
            hi = mid;
        }
    }
    Ok(0.5 * (lo + hi))
}

/// Cash flows: CF0, then each CFj with how many times it repeats (Nj).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Flows {
    pub cf0: f64,
    pub groups: Vec<(f64, u32)>,
}

impl Flows {
    /// Net present value at `i` percent per period.
    pub fn npv(&self, i: f64) -> f64 {
        let r = 1.0 + i / 100.0;
        let mut total = self.cf0;
        let mut t = 0;
        for &(cf, times) in &self.groups {
            for _ in 0..times {
                t += 1;
                total += cf / r.powi(t);
            }
        }
        total
    }

    /// Internal rate of return, percent per period.
    pub fn irr(&self) -> Result<f64, NoSolution> {
        if self.groups.is_empty() {
            return Err(NoSolution);
        }
        root(|r| self.npv(r * 100.0)).map(|r| r * 100.0)
    }
}

/// `x` rounded to `decimals` places, as the display shows it (f RND).
pub fn round_to(x: f64, decimals: u8) -> f64 {
    let scale = 10f64.powi(i32::from(decimals));
    (x * scale).round() / scale
}

/// AMORT: the next `payments` payments of the loan in `tvm`. Each period's
/// interest is rounded to `decimals`, as the 12c rounds it to the display.
/// Returns (interest, principal) — both with the payment's sign — and the
/// balance left.
pub fn amortize(tvm: &Tvm, payments: u32, decimals: u8) -> (f64, f64, f64) {
    let i = tvm.i / 100.0;
    let mut balance = tvm.pv;
    let (mut interest, mut principal) = (0.0, 0.0);
    for k in 0..payments {
        // In BEGIN mode the first payment comes before any interest.
        let first = tvm.n == 0.0 && k == 0;
        let int = if tvm.begin && first {
            0.0
        } else {
            round_to(-balance * i, decimals)
        };
        let prin = tvm.pmt - int;
        interest += int;
        principal += prin;
        balance += prin;
    }
    (interest, principal, balance)
}

/// INT: simple interest on PV for `n` days at `i` percent a year.
/// Returns the interest on 360-day and on 365-day years, with the sign
/// opposite to PV's.
pub fn simple_interest(n: f64, i: f64, pv: f64) -> (f64, f64) {
    let base = -pv * n * i / 100.0;
    (base / 360.0, base / 365.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depreciation {
    StraightLine,
    SumOfYearsDigits,
    /// i is the declining-balance factor, in percent (200 = double).
    DecliningBalance,
}

/// The depreciation for `year` of an asset costing `cost` (PV), worth
/// `salvage` (FV) at the end of `life` years (n), and the depreciable value
/// left after it. `factor` is i, for the declining balance.
pub fn depreciate(
    method: Depreciation,
    cost: f64,
    salvage: f64,
    life: f64,
    factor: f64,
    year: f64,
) -> Result<(f64, f64), NoSolution> {
    if life <= 0.0 || year <= 0.0 || year.fract() != 0.0 || life.fract() != 0.0 {
        return Err(NoSolution);
    }
    let (life, year) = (life as u32, year as u32);
    let base = cost - salvage;
    let mut book = cost;
    let mut dep = 0.0;
    for k in 1..=year {
        dep = if k > life {
            0.0
        } else {
            match method {
                Depreciation::StraightLine => base / f64::from(life),
                Depreciation::SumOfYearsDigits => {
                    let digits = f64::from(life * (life + 1) / 2);
                    base * f64::from(life - k + 1) / digits
                }
                Depreciation::DecliningBalance => {
                    let d = book * factor / 100.0 / f64::from(life);
                    d.min(book - salvage).max(0.0)
                }
            }
        };
        book -= dep;
    }
    Ok((dep, book - salvage))
}

/// A bond with semiannual coupons, priced on the actual/actual basis
/// (the 12c's PRICE and YTM).
pub struct Bond {
    /// Annual coupon rate, percent (PMT).
    pub coupon: f64,
    pub settlement: Date,
    pub maturity: Date,
}

impl Bond {
    /// The coupon period around settlement: (days from its start to
    /// settlement, its length in days, coupons left).
    fn period(&self) -> Option<(f64, f64, u32)> {
        if self.settlement >= self.maturity {
            return None;
        }
        let mut next = self.maturity;
        let mut count = 0u32;
        let mut months = 0i64;
        loop {
            let previous = self.maturity.add_months(months - 6);
            if previous <= self.settlement {
                let a = dates::actual_days(previous, self.settlement) as f64;
                let e = dates::actual_days(previous, next) as f64;
                return Some((a, e, count + 1));
            }
            next = previous;
            count += 1;
            months -= 6;
        }
    }

    /// Clean price per 100 at `yield_` percent a year, and the accrued
    /// interest.
    pub fn price(&self, yield_: f64) -> Result<(f64, f64), NoSolution> {
        let (a, e, n) = self.period().ok_or(NoSolution)?;
        let c = self.coupon / 2.0;
        let y = yield_ / 100.0 / 2.0;
        let dsc = e - a;
        let accrued = c * a / e;
        let dirty = if n == 1 {
            (100.0 + c) / (1.0 + dsc / e * y)
        } else {
            let fraction = dsc / e;
            let discount = |k: f64| (1.0 + y).powf(k - 1.0 + fraction);
            (1..=n).map(|k| c / discount(f64::from(k))).sum::<f64>()
                + 100.0 / discount(f64::from(n))
        };
        finite(dirty - accrued).map(|p| (p, accrued))
    }

    /// The yield, percent a year, at which the clean price is `price`.
    pub fn yield_to_maturity(&self, price: f64) -> Result<f64, NoSolution> {
        root(|y| self.price(y * 100.0).map_or(f64::NAN, |(p, _)| p - price)).map(|y| y * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tvm(n: f64, i: f64, pv: f64, pmt: f64, fv: f64) -> Tvm {
        Tvm {
            n,
            i,
            pv,
            pmt,
            fv,
            begin: false,
        }
    }

    fn near(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn time_value_of_money() {
        // A 30-year mortgage of 100 000 at 6% a year.
        let pmt = tvm(360.0, 0.5, 100_000.0, 0.0, 0.0).solve_pmt().unwrap();
        assert!(near(pmt, -599.55, 0.005), "{pmt}");
        let i = tvm(36.0, 0.0, 10_000.0, -332.14, 0.0).solve_i().unwrap();
        assert!(near(i, 1.0, 0.001), "{i}");
        let n = tvm(0.0, 1.0, 10_000.0, -332.14, 0.0).solve_n().unwrap();
        assert_eq!(n, 36.0);
        let fv = tvm(12.0, 0.5, -1000.0, 0.0, 0.0).solve_fv().unwrap();
        assert!(near(fv, 1061.68, 0.005), "{fv}");
        let pv = tvm(10.0, 8.0, 0.0, 0.0, 10_000.0).solve_pv().unwrap();
        assert!(near(pv, -4631.93, 0.005), "{pv}");
        // Leasing: 36 payments in advance, BEGIN.
        let mut lease = tvm(36.0, 1.0, -10_000.0, 0.0, 0.0);
        lease.begin = true;
        let pmt = lease.solve_pmt().unwrap();
        assert!(near(pmt, 328.85, 0.005), "{pmt}");
        // No interest.
        assert_eq!(tvm(10.0, 0.0, 1000.0, 0.0, 0.0).solve_fv(), Ok(-1000.0));
        assert_eq!(tvm(0.0, 0.0, 1000.0, -100.0, 0.0).solve_n(), Ok(10.0));
        // Same signs everywhere: no rate pays that.
        assert_eq!(
            tvm(10.0, 0.0, 100.0, 10.0, 100.0).solve_i(),
            Err(NoSolution)
        );
    }

    #[test]
    fn cash_flows() {
        let flows = Flows {
            cf0: -1000.0,
            groups: vec![(1100.0, 1)],
        };
        assert!(near(flows.irr().unwrap(), 10.0, 1e-9));
        let flows = Flows {
            cf0: -80_000.0,
            groups: vec![(10_000.0, 3), (12_000.0, 2), (100_000.0, 1)],
        };
        let npv = flows.npv(10.0);
        let irr = flows.irr().unwrap();
        assert!(near(flows.npv(irr), 0.0, 1e-6), "{irr}");
        assert!(npv > 0.0 && irr > 10.0);
    }

    #[test]
    fn amortization() {
        // 50 000 at 13.25%/12 over 30 years, payment 562.89: the first
        // year, each month's interest rounded to cents (checked by hand
        // with the same rounding).
        let loan = Tvm {
            n: 0.0,
            i: 13.25 / 12.0,
            pv: 50_000.0,
            pmt: -562.89,
            fv: 0.0,
            begin: false,
        };
        let (interest, principal, balance) = amortize(&loan, 12, 2);
        assert!(near(interest, -6_616.82, 0.005), "{interest}");
        assert!(near(principal, -137.86, 0.005), "{principal}");
        assert!(near(balance, 49_862.14, 0.005), "{balance}");
    }

    #[test]
    fn simple_and_depreciation() {
        let (i360, i365) = simple_interest(60.0, 7.0, 450.0);
        assert!(near(i360, -5.25, 1e-9));
        assert!(near(i365, -5.178, 0.001));
        let sl = depreciate(Depreciation::StraightLine, 50_000.0, 8_000.0, 6.0, 0.0, 1.0);
        assert_eq!(sl, Ok((7_000.0, 35_000.0)));
        let (soyd, _) = depreciate(
            Depreciation::SumOfYearsDigits,
            50_000.0,
            8_000.0,
            6.0,
            0.0,
            1.0,
        )
        .unwrap();
        assert!(near(soyd, 12_000.0, 1e-9));
        let (db, left) = depreciate(
            Depreciation::DecliningBalance,
            10_000.0,
            1_000.0,
            5.0,
            200.0,
            1.0,
        )
        .unwrap();
        assert_eq!((db, left), (4_000.0, 5_000.0));
        assert_eq!(
            depreciate(Depreciation::StraightLine, 1.0, 0.0, 5.0, 0.0, 1.5),
            Err(NoSolution)
        );
    }

    #[test]
    fn bonds() {
        // A 6.75% bond settled 28 April 2004, maturing 4 June 2018, at a
        // yield of 4.75%: 120.38 and 2.69 accrued, as the standard
        // (SIA, actual/actual) formula gives — checked independently.
        let bond = Bond {
            coupon: 6.75,
            settlement: Date {
                year: 2004,
                month: 4,
                day: 28,
            },
            maturity: Date {
                year: 2018,
                month: 6,
                day: 4,
            },
        };
        let (price, accrued) = bond.price(4.75).unwrap();
        assert!(near(price, 120.38, 0.005), "{price}");
        assert!(near(accrued, 2.69, 0.005), "{accrued}");
        let y = bond.yield_to_maturity(price).unwrap();
        assert!(near(y, 4.75, 1e-6), "{y}");
    }
}
