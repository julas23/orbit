//! Calendar arithmetic of the 12c: dates written as numbers, DATE and ΔDYS.
//!
//! A date is keyed as `dd.mmyyyy` (D.MY mode) or `mm.ddyyyy` (M.DY, the
//! default), from 15 October 1582 to 25 November 4046, the 12c's range.

/// A calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

const FIRST: Date = Date {
    year: 1582,
    month: 10,
    day: 15,
};
const LAST: Date = Date {
    year: 4046,
    month: 11,
    day: 25,
};

pub fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

impl Date {
    /// The date in `x`, written in D.MY (`dmy`) or M.DY order.
    pub fn from_number(x: f64, dmy: bool) -> Option<Date> {
        if !(0.0..100.0).contains(&x) {
            return None;
        }
        // Six decimals: dd.mmyyyy -> "dd.mmyyyy".
        let text = format!("{x:.6}");
        let (a, rest) = text.split_once('.')?;
        let a: u32 = a.parse().ok()?;
        let b: u32 = rest[..2].parse().ok()?;
        let year: i64 = rest[2..6].parse().ok()?;
        let (day, month) = if dmy { (a, b) } else { (b, a) };
        let date = Date { year, month, day };
        let valid = (1..=12).contains(&month)
            && (1..=days_in_month(year, month)).contains(&day)
            && (FIRST..=LAST).contains(&date);
        valid.then_some(date)
    }

    /// The date as the 12c shows it, in D.MY or M.DY order.
    pub fn to_number(self, dmy: bool) -> f64 {
        let (a, b) = if dmy {
            (self.day, self.month)
        } else {
            (self.month, self.day)
        };
        f64::from(a) + f64::from(b) / 100.0 + self.year as f64 / 1_000_000.0
    }

    /// Days since 1970-01-01 (proleptic Gregorian; H. Hinnant's algorithm).
    pub fn ordinal(self) -> i64 {
        let y = if self.month <= 2 {
            self.year - 1
        } else {
            self.year
        };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_ordinal(z: i64) -> Date {
        let z = z + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = yoe + era * 400 + i64::from(month <= 2);
        Date { year, month, day }
    }

    /// 1 = Monday … 7 = Sunday, as the 12c shows after DATE.
    pub fn weekday(self) -> u8 {
        // 1970-01-01 was a Thursday.
        ((self.ordinal() + 3).rem_euclid(7) + 1) as u8
    }

    /// `days` later (or earlier, when negative), within the 12c's range.
    pub fn add_days(self, days: i64) -> Option<Date> {
        let d = Date::from_ordinal(self.ordinal() + days);
        (FIRST..=LAST).contains(&d).then_some(d)
    }

    /// The same day `months` later, clamped to the end of shorter months.
    pub fn add_months(self, months: i64) -> Date {
        let index = self.year * 12 + i64::from(self.month) - 1 + months;
        let year = index.div_euclid(12);
        let month = (index.rem_euclid(12) + 1) as u32;
        let day = self.day.min(days_in_month(year, month));
        Date { year, month, day }
    }
}

/// Actual days from `a` to `b`.
pub fn actual_days(a: Date, b: Date) -> i64 {
    b.ordinal() - a.ordinal()
}

/// Days from `a` to `b` on the 30/360 basis, as the 12c counts them.
pub fn days_360(a: Date, b: Date) -> i64 {
    let d1 = if a.day == 31 { 30 } else { a.day };
    let d2 = if b.day == 31 && d1 >= 30 { 30 } else { b.day };
    (b.year - a.year) * 360 + (i64::from(b.month) - i64::from(a.month)) * 30 + i64::from(d2)
        - i64::from(d1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i64, month: u32, day: u32) -> Date {
        Date { year, month, day }
    }

    #[test]
    fn numbers() {
        assert_eq!(Date::from_number(14.052004, true), Some(d(2004, 5, 14)));
        assert_eq!(Date::from_number(5.142004, false), Some(d(2004, 5, 14)));
        assert_eq!(Date::from_number(31.022004, true), None);
        assert_eq!(Date::from_number(1.011500, true), None, "before 1582");
        assert!((d(2004, 5, 14).to_number(true) - 14.052004).abs() < 1e-9);
        assert!((d(2004, 5, 14).to_number(false) - 5.142004).abs() < 1e-9);
    }

    #[test]
    fn arithmetic() {
        assert_eq!(d(1970, 1, 1).ordinal(), 0);
        assert_eq!(
            Date::from_ordinal(d(2026, 10, 19).ordinal()),
            d(2026, 10, 19)
        );
        assert_eq!(d(2026, 10, 19).weekday(), 1, "a Monday");
        assert_eq!(d(2004, 5, 14).add_days(120), Some(d(2004, 9, 11)));
        assert_eq!(d(2004, 9, 11).weekday(), 6, "a Saturday");
        assert_eq!(d(2024, 1, 31).add_months(1), d(2024, 2, 29));
        assert_eq!(d(2024, 3, 15).add_months(-6), d(2023, 9, 15));
        // The 12c manual's example: 3 June 2003 to 14 October 2004.
        assert_eq!(actual_days(d(2003, 6, 3), d(2004, 10, 14)), 499);
        assert_eq!(days_360(d(2003, 6, 3), d(2004, 10, 14)), 491);
        assert_eq!(days_360(d(2004, 1, 31), d(2004, 3, 31)), 60);
    }
}
