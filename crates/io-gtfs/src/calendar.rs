//! The service day: which services run on which date, and the busiest weekday.
//!
//! A **service** (`service_id`) runs on a date when `calendar.txt` gives it that weekday within
//! its `start_date` and `end_date` (both included), unless `calendar_dates.txt` removes the
//! date (`exception_type` 2); or when `calendar_dates.txt` adds the date (`exception_type` 1).
//! A feed may have only one of the two files (GTFS reference, "calendar.txt" and
//! "calendar_dates.txt").
//!
//! The **busiest weekday** is the Monday to Friday with the most trips in service (the rows of
//! `trips.txt` whose service runs that date), the earliest on a tie, over the feed's whole
//! range of dates: from the earliest `start_date` or exception date to the latest `end_date`
//! or exception date. Nothing bounds the range but the feed.
//!
//! Dates are counted in **days since 1970-01-01** (a Thursday), proleptic Gregorian, by the
//! civil-calendar formulas of Hinnant ("chrono-Compatible Low-Level Date Algorithms", 2013).

use std::collections::HashMap;
use std::io::Read;

use crate::csv::Csv;
use crate::source::Feed;
use crate::{GtfsError, required};

/// The day number (days since 1970-01-01) of a `YYYYMMDD` date, if it is one.
pub(crate) fn day_of(text: &str) -> Option<i64> {
    if text.len() != 8 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i64 = text.parse().ok()?;
    let (y, m, d) = (n / 10_000, n / 100 % 100, n % 100);
    let day = days_from_civil(y, m, d);
    (civil_from_days(day) == (y, m, d)).then_some(day)
}

/// A day number as the `YYYYMMDD` number GTFS writes.
pub(crate) fn date_of(day: i64) -> u32 {
    let (y, m, d) = civil_from_days(day);
    u32::try_from(y * 10_000 + m * 100 + d).expect("a date of a GTFS feed")
}

/// The weekday of a day number, Monday 0 to Sunday 6.
fn weekday(day: i64) -> usize {
    usize::try_from((day + 3).rem_euclid(7)).expect("0 to 6")
}

/// Days since 1970-01-01 of year `y`, month `m` (1 to 12), day `d` (Hinnant 2013).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The year, month and day of a day number (Hinnant 2013).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// One row of `calendar.txt`: the weekdays (Monday first) and the dates, both included.
#[derive(Clone, Copy)]
struct Calendar {
    days: [bool; 7],
    start: i64,
    end: i64,
}

/// The feed's services: how many trips each has, its calendar and its exceptions.
#[derive(Default)]
pub(crate) struct Services {
    /// Service id to its index.
    pub index: HashMap<String, u32>,
    /// Per service, its rows in `trips.txt`.
    trips: Vec<u64>,
    calendar: Vec<Option<Calendar>>,
    /// `(service, day) -> added (true) or removed (false)`.
    exceptions: HashMap<(u32, i64), bool>,
}

/// The value at `column` as a day number, or the error naming the row's service and the value.
fn date_at<R: Read>(
    csv: &Csv<R>,
    file: &'static str,
    (service, column, name): (usize, usize, &'static str),
) -> Result<i64, GtfsError> {
    day_of(csv.field(column)).ok_or_else(|| GtfsError::BadValue {
        file,
        id: csv.field(service).to_owned(),
        column: name,
        value: csv.field(column).to_owned(),
    })
}

impl Services {
    /// Count the trips of each service (`trips.txt`) and read `calendar.txt` and
    /// `calendar_dates.txt`, whichever the feed has. A date that is not one, a weekday flag
    /// or an exception type the reference does not define, a service's calendar given twice
    /// or its exception on one date given twice is an error naming the service and the value.
    pub fn read(feed: &mut Feed) -> Result<Self, GtfsError> {
        let mut services = Self::default();
        let mut csv = feed.csv("trips.txt")?;
        let service = required(&csv, "trips.txt", "service_id")?;
        while csv.next()? {
            let s = services.id(csv.field(service));
            services.trips[s as usize] += 1;
        }
        drop(csv);
        if let Some(csv) = feed.optional_csv("calendar.txt")? {
            services.read_calendar(csv)?;
        }
        if let Some(csv) = feed.optional_csv("calendar_dates.txt")? {
            services.read_exceptions(csv)?;
        }
        Ok(services)
    }

    fn id(&mut self, name: &str) -> u32 {
        if let Some(&s) = self.index.get(name) {
            return s;
        }
        let s = u32::try_from(self.trips.len()).expect("services fit u32");
        self.index.insert(name.to_owned(), s);
        self.trips.push(0);
        self.calendar.push(None);
        s
    }

    fn read_calendar<R: Read>(&mut self, mut csv: Csv<R>) -> Result<(), GtfsError> {
        let f = "calendar.txt";
        let service = required(&csv, f, "service_id")?;
        let (start, end) = (required(&csv, f, "start_date")?, required(&csv, f, "end_date")?);
        let names = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
        let mut columns = [0; 7];
        for (c, name) in columns.iter_mut().zip(names) {
            *c = required(&csv, f, name)?;
        }
        while csv.next()? {
            let a = date_at(&csv, f, (service, start, "start_date"))?;
            let b = date_at(&csv, f, (service, end, "end_date"))?;
            let mut days = [false; 7];
            for ((day, &c), name) in days.iter_mut().zip(&columns).zip(names) {
                *day = match csv.field(c) {
                    "1" => true,
                    "0" => false,
                    v => {
                        let id = csv.field(service).to_owned();
                        return Err(GtfsError::BadValue {
                            file: f,
                            id,
                            column: name,
                            value: v.to_owned(),
                        });
                    }
                };
            }
            let s = self.id(csv.field(service));
            let slot = &mut self.calendar[s as usize];
            if slot.is_some() {
                return Err(GtfsError::RepeatedId { file: f, id: csv.field(service).to_owned() });
            }
            *slot = Some(Calendar { days, start: a, end: b });
        }
        Ok(())
    }

    fn read_exceptions<R: Read>(&mut self, mut csv: Csv<R>) -> Result<(), GtfsError> {
        let f = "calendar_dates.txt";
        let service = required(&csv, f, "service_id")?;
        let (date, kind) = (required(&csv, f, "date")?, required(&csv, f, "exception_type")?);
        while csv.next()? {
            let added = match csv.field(kind) {
                "1" => true,
                "2" => false,
                v => {
                    let (id, value) = (csv.field(service).to_owned(), v.to_owned());
                    return Err(GtfsError::BadValue {
                        file: f,
                        id,
                        column: "exception_type",
                        value,
                    });
                }
            };
            let day = date_at(&csv, f, (service, date, "date"))?;
            let s = self.id(csv.field(service));
            if self.exceptions.insert((s, day), added).is_some() {
                let id = format!("{} on {}", csv.field(service), csv.field(date));
                return Err(GtfsError::RepeatedId { file: f, id });
            }
        }
        Ok(())
    }

    /// Whether service `s` runs on `day` by its calendar alone.
    fn by_calendar(&self, s: usize, day: i64) -> bool {
        self.calendar[s].is_some_and(|c| c.start <= day && day <= c.end && c.days[weekday(day)])
    }

    /// Per service, whether it runs on `day`.
    pub fn active_on(&self, day: i64) -> Vec<bool> {
        (0..self.trips.len())
            .map(|s| {
                let key = (u32::try_from(s).expect("services fit u32"), day);
                self.exceptions.get(&key).copied().unwrap_or_else(|| self.by_calendar(s, day))
            })
            .collect()
    }

    /// The busiest weekday over the feed's whole range of dates (see the module documentation),
    /// or `None` when no weekday has a trip.
    pub fn busiest_weekday(&self) -> Option<i64> {
        let calendars = self.calendar.iter().flatten();
        let exception_days = self.exceptions.keys().map(|k| k.1);
        let first = calendars.clone().map(|c| c.start).chain(exception_days.clone()).min()?;
        let last = calendars.map(|c| c.end).chain(exception_days).max()?;
        if last < first {
            return None;
        }
        let slot = |day: i64| usize::try_from(day - first).expect("in the range");
        let mut count = vec![0_i64; slot(last) + 1];
        for (s, c) in self.calendar.iter().enumerate() {
            let Some(c) = c else { continue };
            let n = i64::try_from(self.trips[s]).expect("trip counts fit i64");
            for day in c.start.max(first)..=c.end.min(last) {
                if c.days[weekday(day)] {
                    count[slot(day)] += n;
                }
            }
        }
        for (&(s, day), &added) in &self.exceptions {
            let n = i64::try_from(self.trips[s as usize]).expect("trip counts fit i64");
            match (added, self.by_calendar(s as usize, day)) {
                (true, false) => count[slot(day)] += n,
                (false, true) => count[slot(day)] -= n,
                _ => {}
            }
        }
        (first..=last)
            .filter(|&day| weekday(day) < 5 && count[slot(day)] > 0)
            // The most trips; on a tie, the earliest date.
            .max_by(|&a, &b| count[slot(a)].cmp(&count[slot(b)]).then(b.cmp(&a)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_against_the_calendar() {
        // 1970-01-01 is day 0, a Thursday; the leap days of 2000 and 2024; not 1900.
        assert_eq!(day_of("19700101"), Some(0));
        assert_eq!(weekday(0), 3);
        assert_eq!(day_of("20261005"), Some(20_731), "a Monday");
        assert_eq!(weekday(20_731), 0);
        assert!(day_of("20000229").is_some() && day_of("20240229").is_some());
        for bad in ["19000229", "20261301", "20260431", "2026101", "2026-10-1", "x"] {
            assert_eq!(day_of(bad), None, "{bad}");
        }
        assert_eq!(date_of(20_731), 20_261_005);
    }
}
