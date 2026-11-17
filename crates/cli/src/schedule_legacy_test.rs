//! The schedule reader from before weeks, macros, and strict parsing (workspace 0.1.10), kept
//! unchanged. Runners that have not updated read canonical strings with this code, so the tests
//! check new canonical strings against it. Do not edit it to match `schedule.rs`: a difference
//! between the two is a difference on an older Runner.
#![allow(dead_code)]

use crate::memory::{local_tm, start_of_local_day};

/// The shortest gap between two runs of one routine.
pub const MIN_INTERVAL_SECS: i64 = 5 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    /// Every so many seconds, counted from the last run (or from when the routine was armed).
    Every(i64),
    Cron(Cron),
}

/// A five-field cron expression as bit sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minutes: u64,
    hours: u32,
    /// Bits 1..=31.
    days_of_month: u32,
    /// Bits 1..=12.
    months: u16,
    /// Bits 0..=6, Sunday is 0.
    days_of_week: u8,
    dom_restricted: bool,
    dow_restricted: bool,
    normalized: String,
}

const MONTH_NAMES: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const DAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const DAY_LABELS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

/// Reads a schedule, or says what is wrong with it.
pub fn parse(text: &str) -> Result<Schedule, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Give a schedule: every 30m, every 2h, every 1d, or a cron expression like 0 9 * * 1-5.".into());
    }
    let lowered = text.to_lowercase();
    if let Some(rest) = lowered.strip_prefix("every ").or_else(|| lowered.strip_prefix('@')) {
        return parse_every(rest.trim());
    }
    let fields: Vec<&str> = text.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "Could not read the schedule {text:?}. Use every 30m, every 2h, every 1d, or five cron fields (minute hour day-of-month month day-of-week) like 0 9 * * 1-5."
        ));
    }
    let minutes = parse_field(fields[0], 0, 59, &[])?;
    let hours = parse_field(fields[1], 0, 23, &[])?;
    let days_of_month = parse_field(fields[2], 1, 31, &[])?;
    let months = parse_field(fields[3], 1, 12, &MONTH_NAMES)?;
    let mut days_of_week = parse_field(fields[4], 0, 7, &DAY_NAMES)?;
    // 7 is Sunday too.
    if days_of_week & (1 << 7) != 0 {
        days_of_week = (days_of_week | 1) & !(1 << 7);
    }
    // More firings within an hour than one every five minutes.
    if minutes.count_ones() as i64 > 3600 / MIN_INTERVAL_SECS {
        return Err("That runs too often. A routine runs at most every five minutes.".into());
    }
    Ok(Schedule::Cron(Cron {
        minutes,
        hours: hours as u32,
        days_of_month: days_of_month as u32,
        months: months as u16,
        days_of_week: days_of_week as u8,
        dom_restricted: fields[2] != "*",
        dow_restricted: fields[4] != "*",
        normalized: fields.join(" "),
    }))
}

/// `30m`, `2h`, `1d`, `30 minutes`, `2 hours`, `day`, `hour`, `hourly`, `daily`.
fn parse_every(rest: &str) -> Result<Schedule, String> {
    let compact: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
    let (amount, unit) = match compact.as_str() {
        "hour" | "hourly" => (1, "h"),
        "day" | "daily" => (1, "d"),
        "minute" => (1, "m"),
        _ => {
            let digits: String = compact.chars().take_while(|c| c.is_ascii_digit()).collect();
            let unit = &compact[digits.len()..];
            let amount: i64 = digits.parse().map_err(|_| format!("Could not read the interval {rest:?}. Use every 30m, every 2h, or every 1d."))?;
            (amount, unit)
        }
    };
    let secs = match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => amount * 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => amount * 3600,
        "d" | "day" | "days" => amount * 86_400,
        _ => return Err(format!("Could not read the interval {rest:?}. Use every 30m, every 2h, or every 1d.")),
    };
    if secs < MIN_INTERVAL_SECS {
        return Err("That runs too often. A routine runs at most every five minutes.".into());
    }
    if secs > 366 * 86_400 {
        return Err("That interval is longer than a year. Use a cron expression for a yearly date.".into());
    }
    Ok(Schedule::Every(secs))
}

/// One cron field into a bit set: `*`, `*/n`, `a`, `a-b`, `a-b/n`, names, and lists of those.
fn parse_field(field: &str, min: u32, max: u32, names: &[&str]) -> Result<u64, String> {
    let mut bits: u64 = 0;
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!("Empty entry in the cron field {field:?}."));
        }
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, step.parse::<u32>().ok().filter(|s| *s > 0).ok_or_else(|| format!("Bad step in {part:?}."))?),
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (parse_value(a, min, max, names)?, parse_value(b, min, max, names)?)
        } else {
            let value = parse_value(range, min, max, names)?;
            if step > 1 {
                (value, max)
            } else {
                (value, value)
            }
        };
        if lo > hi {
            return Err(format!("The range {range:?} runs backwards."));
        }
        let mut value = lo;
        while value <= hi {
            bits |= 1 << value;
            value += step;
        }
    }
    Ok(bits)
}

fn parse_value(text: &str, min: u32, max: u32, names: &[&str]) -> Result<u32, String> {
    let text = text.trim().to_lowercase();
    if let Some(index) = names.iter().position(|n| text.starts_with(n) && text.len() <= 9) {
        return Ok(index as u32 + min);
    }
    let value: u32 = text.parse().map_err(|_| format!("Could not read {text:?} in the cron expression."))?;
    if value < min || value > max {
        return Err(format!("{value} is out of range ({min}-{max}) in the cron expression."));
    }
    Ok(value)
}

impl Schedule {
    /// The first time this schedule fires strictly after `after` (unix seconds), in the local
    /// time zone. `None` when nothing matches in the next two years.
    pub fn next_after(&self, after: i64) -> Option<i64> {
        match self {
            Schedule::Every(secs) => Some(after + secs),
            Schedule::Cron(cron) => cron.next_after(after),
        }
    }

    /// The schedule in words: "Every 15 minutes", "Weekdays at 9:00 AM".
    pub fn describe(&self) -> String {
        match self {
            Schedule::Every(secs) => describe_interval(*secs),
            Schedule::Cron(cron) => cron.describe(),
        }
    }

    /// The canonical spelling, for storage.
    pub fn canonical(&self) -> String {
        match self {
            Schedule::Every(secs) => {
                if secs % 86_400 == 0 {
                    format!("every {}d", secs / 86_400)
                } else if secs % 3600 == 0 {
                    format!("every {}h", secs / 3600)
                } else {
                    format!("every {}m", secs / 60)
                }
            }
            Schedule::Cron(cron) => cron.normalized.clone(),
        }
    }
}

fn describe_interval(secs: i64) -> String {
    let plural = |n: i64, unit: &str| if n == 1 { format!("Every {unit}") } else { format!("Every {n} {unit}s") };
    if secs % 86_400 == 0 {
        plural(secs / 86_400, "day")
    } else if secs % 3600 == 0 {
        plural(secs / 3600, "hour")
    } else {
        plural(secs / 60, "minute")
    }
}

impl Cron {
    fn next_after(&self, after: i64) -> Option<i64> {
        let horizon = after + 2 * 366 * 86_400;
        let mut t = after - after.rem_euclid(60) + 60;
        while t <= horizon {
            let tm = local_tm(t);
            let month_ok = self.months & (1 << (tm.tm_mon + 1)) != 0;
            let dom_ok = self.days_of_month & (1 << tm.tm_mday) != 0;
            let dow_ok = self.days_of_week & (1 << tm.tm_wday) != 0;
            let day_ok = match (self.dom_restricted, self.dow_restricted) {
                (true, true) => dom_ok || dow_ok,
                (true, false) => dom_ok,
                (false, true) => dow_ok,
                (false, false) => true,
            };
            if !month_ok || !day_ok {
                // The next local midnight, whatever the day's length.
                t = start_of_local_day(start_of_local_day(t) + 36 * 3600);
                continue;
            }
            if self.hours & (1 << tm.tm_hour) == 0 {
                t = t - (tm.tm_min as i64) * 60 + 3600;
                continue;
            }
            if self.minutes & (1 << tm.tm_min) == 0 {
                t += 60;
                continue;
            }
            return Some(t);
        }
        None
    }

    fn describe(&self) -> String {
        let minutes: Vec<u32> = (0..60).filter(|m| self.minutes & (1 << m) != 0).collect();
        let hours: Vec<u32> = (0..24).filter(|h| self.hours & (1 << h) != 0).collect();
        let doms: Vec<u32> = (1..=31).filter(|d| self.days_of_month & (1 << d) != 0).collect();
        let dows: Vec<u32> = (0..7).filter(|d| self.days_of_week & (1 << d) != 0).collect();
        let all_months = self.months & 0b1_1111_1111_1110 == 0b1_1111_1111_1110;
        let fallback = || format!("Cron {}", self.normalized);
        if !all_months {
            return fallback();
        }
        let every_day = !self.dom_restricted && (!self.dow_restricted || dows.len() == 7);
        let days = if every_day {
            None
        } else if self.dow_restricted && !self.dom_restricted {
            Some(match dows.as_slice() {
                [1, 2, 3, 4, 5] => "Weekdays".to_string(),
                [0, 6] => "Weekends".to_string(),
                [one] => format!("Every {}", DAY_LABELS[*one as usize]),
                many => join_words(&many.iter().map(|d| DAY_LABELS[*d as usize].to_string()).collect::<Vec<_>>()),
            })
        } else if self.dom_restricted && !self.dow_restricted {
            Some(format!("On the {} of every month", join_words(&doms.iter().map(|d| ordinal(*d)).collect::<Vec<_>>())))
        } else {
            return fallback();
        };
        // Times of day, or a rhythm within the day.
        let times: Option<String> = match (minutes.as_slice(), hours.as_slice()) {
            ([minute], hours) if hours.len() == 24 => {
                if days.is_some() {
                    None
                } else if *minute == 0 {
                    return "Every hour".into();
                } else {
                    return format!("Every hour at :{minute:02}");
                }
            }
            (minutes, hours) if hours.len() == 24 && days.is_none() && minutes.len() > 1 && evenly_spaced(minutes, 60) => {
                return format!("Every {} minutes", 60 / minutes.len() as u32);
            }
            ([0], hours) if hours.len() > 1 && days.is_none() && evenly_spaced(hours, 24) => {
                return format!("Every {} hours", 24 / hours.len() as u32);
            }
            ([minute], hours) if hours.len() <= 3 => Some(join_words(&hours.iter().map(|h| clock(*h, *minute)).collect::<Vec<_>>())),
            _ => None,
        };
        match (days, times) {
            (None, Some(times)) => format!("Every day at {times}"),
            (Some(days), Some(times)) => format!("{days} at {times}"),
            _ => fallback(),
        }
    }
}

/// `9:00 AM`, `5:30 PM`.
fn clock(hour: u32, minute: u32) -> String {
    let (h, suffix) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!("{h}:{minute:02} {suffix}")
}

fn ordinal(n: u32) -> String {
    let suffix = match n % 100 {
        11..=13 => "th",
        _ => match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    };
    format!("{n}{suffix}")
}

/// "A", "A and B", "A, B, and C".
pub(crate) fn join_words(words: &[String]) -> String {
    match words {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        many => format!("{}, and {}", many[..many.len() - 1].join(", "), many[many.len() - 1]),
    }
}

/// `0, 15, 30, 45` within 60: a step that divides the cycle, starting at 0.
fn evenly_spaced(values: &[u32], cycle: u32) -> bool {
    if values.len() < 2 || values[0] != 0 || !cycle.is_multiple_of(values.len() as u32) {
        return false;
    }
    let step = cycle / values.len() as u32;
    values.iter().enumerate().all(|(i, v)| *v == i as u32 * step)
}
