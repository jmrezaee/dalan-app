//! Schedules and retention for saved jobs (D15, E2; replica/databases/restore-copy.md §9).
//!
//! Every schedule is a 5-field cron underneath ("every day at 02:00" is `0 2 * * *`), read in the computer's local
//! time, so daylight saving is handled by chrono's `Local`: a time that doesn't exist on the day the clocks go
//! forward is skipped, and one that happens twice runs at the first. Backups are named
//! `<database>-<job>-<yyyy-mm-dd-hhmm>.<ext>`; retention only ever removes files whose name has exactly that shape for
//! the job, so nothing else in the folder is touched.

use chrono::{DateTime, Datelike, Duration, Local, LocalResult, NaiveDateTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult};

/// How often a job runs. Weekdays count from Sunday = 0, like cron.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "every", rename_all = "snake_case")]
pub enum Every {
    Hour { minute: u8 },
    Day { hour: u8, minute: u8 },
    Week { weekday: u8, hour: u8, minute: u8 },
    Cron { expr: String },
}

/// A job's schedule as the window sets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Schedule {
    #[serde(flatten)]
    pub every: Every,
    /// A notification after a run that went well too (failures always notify).
    pub notify_ok: bool,
}

/// What `db_jobs.schedule` holds: the schedule and the time up to which its slots were handled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stored {
    #[serde(flatten)]
    pub schedule: Schedule,
    pub done_until: i64,
}

/// Keep the last N files, files younger than N days, or both (a file goes only when every rule set lets it go).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Retention {
    pub keep_last: Option<u32>,
    pub keep_days: Option<u32>,
}

/// A parsed cron expression: one bit per allowed value.
#[derive(Debug, Clone)]
pub struct Cron {
    minute: u64,
    hour: u32,
    dom: u32,
    month: u16,
    dow: u8,
    dom_any: bool,
    dow_any: bool,
}

fn field(text: &str, lo: u32, hi: u32, what: &str) -> AppResult<(u64, bool)> {
    let bad = || {
        AppError::validation(format!(
            "The {what} field “{text}” isn't valid: use numbers {lo}–{hi}, *, ranges (1-5), lists (1,15) and steps (*/10)."
        ))
    };
    let mut bits = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().map_err(|_| bad())?),
            None => (part, 1),
        };
        if step == 0 {
            return Err(bad());
        }
        let (a, b) = if range == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?)
        } else {
            let a: u32 = range.parse().map_err(|_| bad())?;
            // `5/15` means from 5 to the end, every 15.
            (a, if part.contains('/') { hi } else { a })
        };
        if a < lo || b > hi || a > b {
            return Err(bad());
        }
        let mut v = a;
        while v <= b {
            bits |= 1 << v;
            v += step;
        }
    }
    Ok((bits, text == "*"))
}

impl Cron {
    pub fn parse(expr: &str) -> AppResult<Cron> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(AppError::validation(
                "A cron schedule has five fields: minute hour day-of-month month weekday (for example 30 2 * * 1-5).",
            ));
        }
        let (minute, _) = field(parts[0], 0, 59, "minute")?;
        let (hour, _) = field(parts[1], 0, 23, "hour")?;
        let (dom, dom_any) = field(parts[2], 1, 31, "day-of-month")?;
        let (month, _) = field(parts[3], 1, 12, "month")?;
        let (mut dow, dow_any) = field(parts[4], 0, 7, "weekday")?;
        // 7 is Sunday too.
        if dow & (1 << 7) != 0 {
            dow |= 1;
        }
        Ok(Cron {
            minute,
            hour: hour as u32,
            dom: dom as u32,
            month: month as u16,
            dow: (dow & 0x7f) as u8,
            dom_any,
            dow_any,
        })
    }

    fn day_matches(&self, t: &NaiveDateTime) -> bool {
        let dom = self.dom & (1 << t.day()) != 0;
        let dow = self.dow & (1 << t.weekday().num_days_from_sunday()) != 0;
        // Cron's rule: when both day fields are set, either one is enough.
        match (self.dom_any, self.dow_any) {
            (true, true) => true,
            (true, false) => dow,
            (false, true) => dom,
            (false, false) => dom || dow,
        }
    }

    /// The first minute after `t` (local wall clock) that the expression allows, within a year and a day.
    pub fn next_naive(&self, t: NaiveDateTime) -> Option<NaiveDateTime> {
        let mut c = t.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        let end = t + Duration::days(367);
        while c <= end {
            if self.month & (1 << c.month()) == 0 || !self.day_matches(&c) {
                c = (c.date() + Duration::days(1)).and_hms_opt(0, 0, 0)?;
                continue;
            }
            if self.hour & (1 << c.hour()) == 0 {
                c = c.with_minute(0)? + Duration::hours(1);
                continue;
            }
            if self.minute & (1 << c.minute()) != 0 {
                return Some(c);
            }
            c += Duration::minutes(1);
        }
        None
    }
}

impl Every {
    pub fn cron(&self) -> AppResult<Cron> {
        let check = |ok: bool| {
            if ok {
                Ok(())
            } else {
                Err(AppError::validation(
                    "Pick a time between 00:00 and 23:59 (and a weekday).",
                ))
            }
        };
        match self {
            Every::Hour { minute } => {
                check(*minute < 60)?;
                Cron::parse(&format!("{minute} * * * *"))
            }
            Every::Day { hour, minute } => {
                check(*hour < 24 && *minute < 60)?;
                Cron::parse(&format!("{minute} {hour} * * *"))
            }
            Every::Week {
                weekday,
                hour,
                minute,
            } => {
                check(*weekday < 7 && *hour < 24 && *minute < 60)?;
                Cron::parse(&format!("{minute} {hour} * * {weekday}"))
            }
            Every::Cron { expr } => Cron::parse(expr),
        }
    }

    /// In words, for the Jobs list.
    pub fn describe(&self) -> String {
        const DAYS: [&str; 7] = [
            "Sunday",
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
        ];
        match self {
            Every::Hour { minute } => format!("Every hour at :{minute:02}"),
            Every::Day { hour, minute } => format!("Every day at {hour:02}:{minute:02}"),
            Every::Week {
                weekday,
                hour,
                minute,
            } => format!(
                "Every {} at {hour:02}:{minute:02}",
                DAYS.get(*weekday as usize).unwrap_or(&"week")
            ),
            Every::Cron { expr } => format!("Cron {}", expr.trim()),
        }
    }
}

/// The next run after `after_ms` in time zone `tz` (epoch ms), skipping wall-clock times that don't exist there.
pub fn next_in<Tz: TimeZone>(cron: &Cron, tz: &Tz, after_ms: i64) -> Option<i64> {
    let mut t = tz.timestamp_millis_opt(after_ms).single()?.naive_local();
    for _ in 0..1000 {
        let n = cron.next_naive(t)?;
        match tz.from_local_datetime(&n) {
            LocalResult::Single(d) => return Some(d.timestamp_millis()),
            LocalResult::Ambiguous(d, _) => return Some(d.timestamp_millis()),
            LocalResult::None => t = n,
        }
    }
    None
}

pub fn next(cron: &Cron, after_ms: i64) -> Option<i64> {
    next_in(cron, &Local, after_ms)
}

/// How many slots fell in `(from_ms, to_ms]` (counted up to `cap`), and the last of them.
pub fn slots_in<Tz: TimeZone>(
    cron: &Cron,
    tz: &Tz,
    from_ms: i64,
    to_ms: i64,
    cap: u32,
) -> (u32, Option<i64>) {
    let (mut n, mut last, mut t) = (0, None, from_ms);
    while n < cap {
        match next_in(cron, tz, t) {
            Some(x) if x <= to_ms => {
                n += 1;
                last = Some(x);
                t = x;
            }
            _ => break,
        }
    }
    (n, last)
}

/// Letters, digits, `-` and `_` stay; anything else becomes `-` (file names on every system).
pub fn safe(x: &str) -> String {
    x.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

const STAMP: &str = "%Y-%m-%d-%H%M";

/// `<database>-<job>-<yyyy-mm-dd-hhmm>.<ext>` at local time `at`.
pub fn backup_name(db: &str, job: &str, at: DateTime<Local>, ext: &str) -> String {
    format!("{}-{}-{}.{ext}", safe(db), safe(job), at.format(STAMP))
}

/// The time in a file name when it is one of this job's backups, `None` for any other file.
pub fn stamp_of(name: &str, db: &str, job: &str, ext: &str) -> Option<NaiveDateTime> {
    let prefix = format!("{}-{}-", safe(db), safe(job));
    let rest = name.strip_prefix(&prefix)?;
    let stamp = rest.strip_suffix(&format!(".{ext}"))?;
    if stamp.len() != 15 {
        return None;
    }
    NaiveDateTime::parse_from_str(stamp, STAMP).ok()
}

/// Which of this job's files retention removes. `files` are (name, stamp); the newest always stays.
pub fn to_remove(
    mut files: Vec<(String, NaiveDateTime)>,
    r: &Retention,
    now: NaiveDateTime,
) -> Vec<String> {
    if r.keep_last.is_none() && r.keep_days.is_none() {
        return Vec::new();
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    files
        .into_iter()
        .enumerate()
        .filter(|(i, (_, t))| {
            let beyond_count = r.keep_last.is_none_or(|n| *i >= n.max(1) as usize);
            let too_old = r
                .keep_days
                .is_none_or(|d| now - *t > Duration::days(i64::from(d)));
            *i > 0 && beyond_count && too_old
        })
        .map(|(_, (n, _))| n)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate, Utc};

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn schedules_find_their_next_run_in_the_given_zone() {
        // 2026-11-27 is a Friday.
        let daily = Every::Day { hour: 2, minute: 0 }.cron().unwrap();
        assert_eq!(
            next_in(&daily, &Utc, at(2026, 11, 27, 1, 59)),
            Some(at(2026, 11, 27, 2, 0))
        );
        assert_eq!(
            next_in(&daily, &Utc, at(2026, 11, 27, 2, 0)),
            Some(at(2026, 11, 28, 2, 0))
        );
        // Local 02:00 at +03:30 is 22:30 UTC the day before.
        let tehran = FixedOffset::east_opt(3 * 3600 + 1800).unwrap();
        assert_eq!(
            next_in(&daily, &tehran, at(2026, 11, 27, 0, 0)),
            Some(at(2026, 11, 27, 22, 30))
        );
        let monday = Every::Week {
            weekday: 1,
            hour: 9,
            minute: 30,
        }
        .cron()
        .unwrap();
        assert_eq!(
            next_in(&monday, &Utc, at(2026, 11, 27, 12, 0)),
            Some(at(2026, 11, 30, 9, 30))
        );
        let hourly = Every::Hour { minute: 15 }.cron().unwrap();
        assert_eq!(
            next_in(&hourly, &Utc, at(2026, 11, 27, 12, 15)),
            Some(at(2026, 11, 27, 13, 15))
        );
        let cron = Cron::parse("*/20 8-9 1,15 * *").unwrap();
        assert_eq!(
            next_in(&cron, &Utc, at(2026, 11, 27, 0, 0)),
            Some(at(2026, 12, 1, 8, 0))
        );
        // Both day fields set: either matches (the 1st, or any Sunday).
        let either = Cron::parse("0 0 1 * 0").unwrap();
        assert_eq!(
            next_in(&either, &Utc, at(2026, 11, 27, 0, 0)),
            Some(at(2026, 11, 29, 0, 0))
        );
        let feb30 = Cron::parse("0 0 30 2 *").unwrap();
        assert_eq!(next_in(&feb30, &Utc, at(2026, 11, 27, 0, 0)), None);
        for bad in [
            "* * * *",
            "60 * * * *",
            "*/0 * * * *",
            "a * * * *",
            "5-1 * * * *",
        ] {
            assert!(Cron::parse(bad).is_err(), "{bad}");
        }
        assert!(Every::Day {
            hour: 24,
            minute: 0
        }
        .cron()
        .is_err());
    }

    #[test]
    fn missed_slots_are_counted_and_capped() {
        let hourly = Every::Hour { minute: 0 }.cron().unwrap();
        let (n, last) = slots_in(
            &hourly,
            &Utc,
            at(2026, 11, 27, 6, 30),
            at(2026, 11, 27, 9, 12),
            1000,
        );
        assert_eq!((n, last), (3, Some(at(2026, 11, 27, 9, 0))));
        assert_eq!(
            slots_in(
                &hourly,
                &Utc,
                at(2026, 11, 27, 9, 0),
                at(2026, 11, 27, 9, 59),
                1000
            )
            .0,
            0
        );
        assert_eq!(
            slots_in(
                &hourly,
                &Utc,
                at(2026, 1, 1, 0, 0),
                at(2026, 11, 27, 0, 0),
                50
            )
            .0,
            50
        );
    }

    #[test]
    fn retention_only_touches_this_jobs_files_and_keeps_the_newest() {
        let t = |d: u32| {
            NaiveDate::from_ymd_opt(2026, 11, d)
                .unwrap()
                .and_hms_opt(2, 0, 0)
                .unwrap()
        };
        let name = |d: u32| format!("shop-Nightly-2026-11-{d:02}-0200.dump");
        assert_eq!(stamp_of(&name(20), "shop", "Nightly", "dump"), Some(t(20)));
        for other in [
            "shop-Nightly-2026-11-20-0200.sql",
            "shop-Nightly-2-2026-11-20-0200.dump",
            "shop-Nightly-2026-11-20-0200.dump.part",
            "shop-Nightly-latest.dump",
            "notes.txt",
        ] {
            assert_eq!(stamp_of(other, "shop", "Nightly", "dump"), None, "{other}");
        }
        assert_eq!(
            stamp_of(
                "my-db-k4-backup-2026-11-20-0200.archive.gz",
                "my db",
                "k4 backup",
                "archive.gz"
            ),
            Some(t(20))
        );
        let files: Vec<_> = (20..=27).map(|d| (name(d), t(d))).collect();
        let now = t(27);
        let keep3 = Retention {
            keep_last: Some(3),
            keep_days: None,
        };
        let mut gone = to_remove(files.clone(), &keep3, now);
        gone.sort();
        assert_eq!(gone, (20..=24).map(name).collect::<Vec<_>>());
        let days2 = Retention {
            keep_last: None,
            keep_days: Some(2),
        };
        assert_eq!(to_remove(files.clone(), &days2, now).len(), 5);
        // Both: a file goes only when it is beyond the count and older than the days.
        let both = Retention {
            keep_last: Some(2),
            keep_days: Some(5),
        };
        let mut gone = to_remove(files.clone(), &both, now);
        gone.sort();
        assert_eq!(gone, vec![name(20), name(21)]);
        assert!(to_remove(files.clone(), &Retention::default(), now).is_empty());
        // The newest stays even when every rule would remove it.
        let zero = Retention {
            keep_last: Some(0),
            keep_days: Some(0),
        };
        assert_eq!(
            to_remove(vec![(name(1), t(1))], &zero, now),
            Vec::<String>::new()
        );
    }
}
