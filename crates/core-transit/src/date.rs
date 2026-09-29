//! Service days: the calendar dates a GTFS feed runs on.
//!
//! A [`ServiceDate`] is a day number (days since 1970-01-01), so a range of days
//! is a range of integers and a weekday is a remainder. The conversion is
//! Howard Hinnant's `days_from_civil`, exact for the proleptic Gregorian
//! calendar; no time zone is involved, because a GTFS time is a count of
//! seconds from the service day's midnight (noon minus twelve hours, strictly;
//! the two differ only on the two days a year the clocks change).

use core::fmt;

/// A day of the week.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Weekday {
    /// Monday.
    Monday = 0,
    /// Tuesday.
    Tuesday = 1,
    /// Wednesday.
    Wednesday = 2,
    /// Thursday.
    Thursday = 3,
    /// Friday.
    Friday = 4,
    /// Saturday.
    Saturday = 5,
    /// Sunday.
    Sunday = 6,
}

impl Weekday {
    /// Every day, Monday first (the order of GTFS `calendar.txt`'s columns).
    pub const ALL: [Weekday; 7] = [
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ];

    /// Monday to Friday.
    #[must_use]
    pub const fn is_weekday(self) -> bool {
        (self as u8) < 5
    }

    /// The lower-case English name, as `calendar.txt` names its column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Weekday::Monday => "monday",
            Weekday::Tuesday => "tuesday",
            Weekday::Wednesday => "wednesday",
            Weekday::Thursday => "thursday",
            Weekday::Friday => "friday",
            Weekday::Saturday => "saturday",
            Weekday::Sunday => "sunday",
        }
    }
}

/// A calendar date, as a day number.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceDate(i32);

impl ServiceDate {
    /// The date `year-month-day`, or `None` if there is no such day.
    #[must_use]
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Option<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self(days_from_civil(year, month, day)))
    }

    /// Parse GTFS's `YYYYMMDD`, or `None` if it is not a date.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.len() != 8 || !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let year: i32 = text[0..4].parse().ok()?;
        let month: u32 = text[4..6].parse().ok()?;
        let day: u32 = text[6..8].parse().ok()?;
        Self::from_ymd(year, month, day)
    }

    /// The day number: days since 1970-01-01.
    #[must_use]
    pub const fn day_number(self) -> i32 {
        self.0
    }

    /// The date with this day number.
    #[must_use]
    pub const fn from_day_number(day: i32) -> Self {
        Self(day)
    }

    /// The day of the week.
    #[must_use]
    pub fn weekday(self) -> Weekday {
        // 1970-01-01 was a Thursday.
        Weekday::ALL[(self.0 + 3).rem_euclid(7) as usize]
    }

    /// `(year, month, day)`.
    #[must_use]
    pub fn ymd(self) -> (i32, u32, u32) {
        civil_from_days(self.0)
    }

    /// The date `days` later (earlier if negative).
    #[must_use]
    pub const fn plus_days(self, days: i32) -> Self {
        Self(self.0 + days)
    }
}

impl fmt::Display for ServiceDate {
    /// ISO 8601: `2026-10-09`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.ymd();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

impl fmt::Debug for ServiceDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServiceDate({self})")
    }
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

#[allow(
    clippy::cast_possible_wrap,
    reason = "month and day are at most 31, far inside i32; the year is an i32 already"
)]
fn days_from_civil(year: i32, month: u32, day: u32) -> i32 {
    let (m, d) = (month as i32, day as i32);
    let y = if m <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[allow(
    clippy::cast_sign_loss,
    reason = "month and day are computed in 1..=12 and 1..=31, both positive"
)]
fn civil_from_days(z: i32) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_dates_and_weekdays() {
        let epoch = ServiceDate::parse("19700101").unwrap();
        assert_eq!(epoch.day_number(), 0);
        assert_eq!(epoch.weekday(), Weekday::Thursday);
        let d = ServiceDate::parse("20261009").unwrap();
        assert_eq!(d.weekday(), Weekday::Friday, "9 October 2026 is a Friday");
        assert_eq!(d.to_string(), "2026-10-09");
        assert_eq!(ServiceDate::parse("20240229").unwrap().ymd(), (2024, 2, 29));
        assert_eq!(ServiceDate::parse("19691231").unwrap().weekday(), Weekday::Wednesday);
    }

    #[test]
    fn not_dates_are_refused() {
        for bad in ["20230229", "20261301", "2026100", "2026-10-09", "", "20261000"] {
            assert!(ServiceDate::parse(bad).is_none(), "{bad:?} is not a date");
        }
    }

    #[test]
    fn every_day_round_trips_over_four_centuries() {
        let start = ServiceDate::parse("18000101").unwrap().day_number();
        let mut previous = ServiceDate::from_day_number(start - 1).weekday() as u8;
        for n in start..start + 146_097 {
            let date = ServiceDate::from_day_number(n);
            let (y, m, d) = date.ymd();
            assert_eq!(ServiceDate::from_ymd(y, m, d), Some(date));
            let w = date.weekday() as u8;
            assert_eq!(w, (previous + 1) % 7, "weekdays follow one another");
            previous = w;
        }
    }
}
