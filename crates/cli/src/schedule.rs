//! When a routine runs: an interval (`every 30m`, `every 2h`, `every 1d`, `every 2w`), a cron
//! macro (`@weekly`, `@monthly`, `@yearly`, `@midnight`), or a five-field cron expression
//! (`0 9 * * 1-5`) read in the Runner's local time. `describe` says it in words the way Grok
//! Bot's routine panel does; `next_after` finds the next firing.
//!
//! What `canonical` stores is read by older Runners too, so it only uses their grammar:
//! `every Nm`, `every Nh`, `every Nd`, and five cron fields. Weeks are stored as days and macros
//! as the five fields they stand for. `@hourly` and `@daily` have always meant intervals here
//! (`every 1h` and `every 1d`, counted from the last run), and they still do.
//!
//! `parse` reads new text strictly. `parse_stored` reads a saved schedule the way older
//! Runners did, so a schedule that is already saved keeps its meaning and keeps running.

use crate::memory::{local_time, local_tm, start_of_local_day};

/// The shortest gap between two runs of one routine.
pub const MIN_INTERVAL_SECS: i64 = 5 * 60;

/// The longest interval. Anything rarer is a date, and a cron expression says dates.
const MAX_INTERVAL_SECS: i64 = 366 * 86_400;

/// How far ahead `next_after` looks. The rarest date a valid expression can ask for is Feb 29,
/// and leap years can be eight years apart: 2096, then 2104, because 2100 is not a leap year. The
/// extra days cover the time zone's offset and clock changes.
const SEARCH_HORIZON_SECS: i64 = (8 * 366 + 2) * 86_400;

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

/// Full English names. Any prefix of three letters or more names the month or day: `sep`,
/// `sept`, `september`.
const MONTH_NAMES: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
const DAY_NAMES: [&str; 7] = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
const DAY_LABELS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
/// The most days each month can have, Feb 29 included.
const MONTH_DAYS: [u32; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

const TOO_OFTEN: &str = "That runs too often. A routine runs at most every five minutes.";

/// How a request for something regular is written as a schedule. People ask in their own
/// language ("run this daily", "rode isso diariamente", "毎日実行して"); the bot reads what they
/// mean and writes it in this one form, so the same request is the same schedule in every
/// language. The routines tool shows the bot this list, and the tests parse each entry.
pub const REQUESTS: &[(&str, &str)] = &[
    ("every half hour", "every 30m"),
    ("every 2 hours", "every 2h"),
    ("daily, with no time of day", "every 1d"),
    ("every day at 9", "0 9 * * *"),
    ("weekdays at 8:30", "30 8 * * 1-5"),
    ("weekly, with no day or time", "every 1w"),
    ("every Monday at 9", "0 9 * * 1"),
    ("every other week", "every 2w"),
    ("on the 1st of every month at 9", "0 9 1 * *"),
];

/// Whether the text is new from a person or a bot, or was saved earlier, maybe by an older Runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reading {
    New,
    Stored,
}

/// Reads a new schedule, or says what is wrong with it.
pub fn parse(text: &str) -> Result<Schedule, String> {
    read(text, Reading::New)
}

/// Reads a saved schedule the way older Runners do. This accepts anything they accepted, such
/// as `0,1 * * * *` and a day written `monkey`, so a saved routine does not stop or change when
/// `parse` gets stricter. Use `parse` for text that is about to be saved.
pub fn parse_stored(text: &str) -> Result<Schedule, String> {
    read(text, Reading::Stored)
}

fn read(text: &str, reading: Reading) -> Result<Schedule, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Give a schedule: every 30m, every 2h, every 1d, every 2w, @weekly, or a cron expression like 0 9 * * 1-5.".into());
    }
    let lowered = text.to_lowercase();
    if let Some(name) = lowered.strip_prefix('@') {
        let name = name.trim();
        if let Some(fields) = macro_fields(name) {
            return parse_cron(fields, reading);
        }
        return parse_every(name).map_err(|error| {
            if name.chars().all(|c| c.is_ascii_alphabetic()) {
                format!("There is no @{name}. Use @hourly, @daily, @weekly, @monthly, @yearly, or @midnight.")
            } else {
                error
            }
        });
    }
    if let Some(rest) = lowered.strip_prefix("every").filter(|rest| rest.starts_with(char::is_whitespace)) {
        return parse_every(rest.trim());
    }
    parse_cron(text, reading)
}

/// Macros as the five fields older Runners read. `@hourly` and `@daily` are not listed here:
/// they have always been intervals, and they stay intervals.
fn macro_fields(name: &str) -> Option<&'static str> {
    Some(match name {
        "weekly" => "0 0 * * 0",
        "monthly" => "0 0 1 * *",
        "yearly" | "annually" => "0 0 1 1 *",
        "midnight" => "0 0 * * *",
        _ => return None,
    })
}

/// `30m`, `2h`, `1d`, `2w`, `30 minutes`, `2 weeks`, `day`, `week`, `hour`, `hourly`, `daily`.
fn parse_every(rest: &str) -> Result<Schedule, String> {
    let unreadable = || format!("Could not read the interval {rest:?}. Use every 30m, every 2h, every 1d, or every 2w.");
    let compact: String = rest.chars().filter(|c| !c.is_whitespace()).collect();
    let (digits, unit) = match compact.as_str() {
        "hour" | "hourly" => ("1", "h"),
        "day" | "daily" => ("1", "d"),
        "week" => ("1", "w"),
        "minute" => ("1", "m"),
        other => other.split_at(other.find(|c: char| !c.is_ascii_digit()).unwrap_or(other.len())),
    };
    if digits.is_empty() {
        return Err(unreadable());
    }
    let unit_secs: i64 = match unit {
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        // Stored as days, since older Runners have no weeks.
        "w" | "wk" | "wks" | "week" | "weeks" => 7 * 86_400,
        _ => return Err(unreadable()),
    };
    // A number too big for i64, or one that overflows once multiplied, is longer than a year.
    let secs = digits.parse::<i64>().ok().and_then(|amount| amount.checked_mul(unit_secs)).unwrap_or(i64::MAX);
    if secs < MIN_INTERVAL_SECS {
        return Err(TOO_OFTEN.into());
    }
    if secs > MAX_INTERVAL_SECS {
        return Err("That interval is longer than a year. Use a cron expression for a yearly date.".into());
    }
    Ok(Schedule::Every(secs))
}

fn parse_cron(text: &str, reading: Reading) -> Result<Schedule, String> {
    let fields: Vec<&str> = text.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "Could not read the schedule {text:?}. Use every 30m, every 2h, every 1d, every 2w, @weekly, or five cron fields (minute hour day-of-month month day-of-week) like 0 9 * * 1-5."
        ));
    }
    let minutes = parse_field(fields[0], 0, 59, &[], reading)?;
    let hours = parse_field(fields[1], 0, 23, &[], reading)?;
    let days_of_month = parse_field(fields[2], 1, 31, &[], reading)?;
    let months = parse_field(fields[3], 1, 12, &MONTH_NAMES, reading)?;
    let mut days_of_week = parse_field(fields[4], 0, 7, &DAY_NAMES, reading)?;
    // 7 is Sunday too.
    if days_of_week & (1 << 7) != 0 {
        days_of_week = (days_of_week | 1) & !(1 << 7);
    }
    let cron = Cron {
        minutes,
        hours: hours as u32,
        days_of_month: days_of_month as u32,
        months: months as u16,
        days_of_week: days_of_week as u8,
        dom_restricted: fields[2] != "*",
        dow_restricted: fields[4] != "*",
        normalized: fields.join(" "),
    };
    match reading {
        Reading::New => {
            if !cron.is_possible() {
                return Err(format!("The schedule {:?} never runs: none of its months has that day of the month.", cron.normalized));
            }
            let gap = cron.shortest_gap_minutes();
            if gap < MIN_INTERVAL_SECS / 60 {
                let minutes = if gap == 1 { "1 minute".to_string() } else { format!("{gap} minutes") };
                return Err(format!("That runs too often: two runs can be {minutes} apart. A routine runs at most every five minutes."));
            }
        }
        // What older Runners checked: no more than twelve minutes of the hour.
        Reading::Stored => {
            if minutes.count_ones() as i64 > 3600 / MIN_INTERVAL_SECS {
                return Err(TOO_OFTEN.into());
            }
        }
    }
    Ok(Schedule::Cron(cron))
}

/// One cron field into a bit set: `*`, `*/n`, `a`, `a-b`, `a-b/n`, names, and lists of those.
fn parse_field(field: &str, min: u32, max: u32, names: &[&str], reading: Reading) -> Result<u64, String> {
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
            (parse_value(a, min, max, names, reading)?, parse_value(b, min, max, names, reading)?)
        } else {
            let value = parse_value(range, min, max, names, reading)?;
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
            // A step past u32::MAX ends the range instead of wrapping round.
            match value.checked_add(step) {
                Some(next) => value = next,
                None => break,
            }
        }
    }
    Ok(bits)
}

fn parse_value(text: &str, min: u32, max: u32, names: &[&str], reading: Reading) -> Result<u32, String> {
    let text = text.trim().to_lowercase();
    let named = match reading {
        // Three letters or more of the English name: `mon`, `tues`, `thursday`, `sept`.
        Reading::New => names.iter().position(|name| text.len() >= 3 && name.starts_with(text.as_str())),
        // Older Runners took anything that starts with the three-letter name, `monkey` included.
        Reading::Stored => names.iter().position(|name| text.starts_with(&name[..3]) && text.len() <= 9),
    };
    if let Some(index) = named {
        return Ok(index as u32 + min);
    }
    if !names.is_empty() && !text.is_empty() && text.chars().all(|c| c.is_ascii_alphabetic()) {
        let (kind, first, last) = if names.len() == 7 { ("day", "sun", "sat") } else { ("month", "jan", "dec") };
        return Err(format!("{text:?} is not a {kind} name. Use {first} to {last}, or the full name."));
    }
    let value: u32 = text.parse().map_err(|_| format!("Could not read {text:?} in the cron expression."))?;
    if value < min || value > max {
        return Err(format!("{value} is out of range ({min}-{max}) in the cron expression."));
    }
    Ok(value)
}

impl Schedule {
    /// The first time this schedule fires strictly after `after` (unix seconds), in the local
    /// time zone. `None` when an interval's next firing exceeds the timestamp range, when no
    /// cron matches in the next eight years, or past what the clock can say.
    pub fn next_after(&self, after: i64) -> Option<i64> {
        self.next_in(after, &LocalZone)
    }

    fn next_in(&self, after: i64, zone: &impl Zone) -> Option<i64> {
        match self {
            Schedule::Every(secs) => after.checked_add(*secs),
            Schedule::Cron(cron) => cron.next_in(after, zone),
        }
    }

    /// The schedule in words: "Every 15 minutes", "Weekdays at 9:00 AM".
    pub fn describe(&self) -> String {
        match self {
            Schedule::Every(secs) => describe_interval(*secs),
            Schedule::Cron(cron) => cron.describe(),
        }
    }

    /// The canonical spelling, for storage. Older Runners read it too: intervals in minutes,
    /// hours, or days, and cron as five fields.
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

// MARK: - Local time

/// A local date and clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Civil {
    year: i64,
    /// 1..=12.
    month: u32,
    /// 1..=31.
    day: u32,
    hour: u32,
    minute: u32,
    /// 0..=6, Sunday is 0.
    weekday: u32,
}

/// How a time zone reads an instant. The Runner uses the machine's zone; tests use fixed ones,
/// so clock changes can be tested on any machine.
trait Zone {
    /// The local date and time at `unix`, or `None` when the zone cannot say.
    fn civil(&self, unix: i64) -> Option<Civil>;
    /// When a local date starts: its midnight, or the first instant of the day when a clock
    /// change skips midnight.
    fn day_start(&self, year: i64, month: u32, day: u32) -> Option<i64>;
}

/// The machine's zone, the one the rest of the Runner reads.
struct LocalZone;

impl Zone for LocalZone {
    fn civil(&self, unix: i64) -> Option<Civil> {
        let tm = local_tm(unix);
        // `local_tm` leaves the fields zero when the C library cannot convert the time.
        (tm.tm_mday >= 1).then(|| Civil {
            year: tm.tm_year as i64 + 1900,
            month: tm.tm_mon as u32 + 1,
            day: tm.tm_mday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
            weekday: tm.tm_wday as u32,
        })
    }

    fn day_start(&self, year: i64, month: u32, day: u32) -> Option<i64> {
        // Noon UTC is on the date itself in nearly every zone. Zones more than twelve hours
        // from UTC need one step back or forward.
        let wanted = days_from_civil(year, month, day);
        let mut probe = wanted.checked_mul(86_400)?.checked_add(43_200)?;
        for _ in 0..3 {
            let local = self.civil(probe)?;
            let have = days_from_civil(local.year, local.month, local.day);
            if have == wanted {
                return Some(start_of_local_day(probe));
            }
            probe += (wanted - have).signum() * 43_200;
        }
        None
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let (month, day) = (month as i64, day as i64);
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 } as u32;
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// 0..=6 with Sunday 0, for days since 1970-01-01, a Thursday.
fn weekday_of(days: i64) -> u32 {
    (days + 4).rem_euclid(7) as u32
}

impl Cron {
    fn month_matches(&self, month: u32) -> bool {
        self.months & (1 << month) != 0
    }

    /// Both day fields restricted: either one matches, as cron has it.
    fn day_matches(&self, day: u32, weekday: u32) -> bool {
        let dom_ok = self.days_of_month & (1 << day) != 0;
        let dow_ok = self.days_of_week & (1 << weekday) != 0;
        match (self.dom_restricted, self.dow_restricted) {
            (true, true) => dom_ok || dow_ok,
            (true, false) => dom_ok,
            (false, true) => dow_ok,
            (false, false) => true,
        }
    }

    /// Whether any date matches. Days of the week come round in every month, so only a day of
    /// the month on its own can ask for a date that never comes, like Feb 30 or Apr 31.
    fn is_possible(&self) -> bool {
        if !self.dom_restricted || self.dow_restricted {
            return true;
        }
        (1..=12u32).any(|month| self.month_matches(month) && (1..=MONTH_DAYS[month as usize - 1]).any(|day| self.days_of_month & (1 << day) != 0))
    }

    /// Whether two dates in a row can both match, across a month's end and Feb 29 too.
    /// 2001 to 2028 has every date on every weekday and no century year.
    fn fires_on_consecutive_days(&self) -> bool {
        let first = days_from_civil(2001, 1, 1);
        let last = days_from_civil(2029, 1, 1);
        let mut previous = false;
        for days in first..=last {
            let (_, month, day) = civil_from_days(days);
            let fires = self.month_matches(month) && self.day_matches(day, weekday_of(days));
            if fires && previous {
                return true;
            }
            previous = fires;
        }
        false
    }

    /// The shortest gap between two runs, in minutes of the local clock: within an hour, from
    /// one hour to the next, and across midnight when two days in a row can both run. A clock
    /// change can shorten it on the day it happens.
    fn shortest_gap_minutes(&self) -> i64 {
        let mut times = Vec::new();
        for hour in (0..24).filter(|h| self.hours & (1 << h) != 0) {
            for minute in (0..60).filter(|m| self.minutes & (1u64 << m) != 0) {
                times.push(hour as i64 * 60 + minute as i64);
            }
        }
        let (Some(first), Some(last)) = (times.first(), times.last()) else {
            return i64::MAX;
        };
        let within_days = times.windows(2).map(|pair| pair[1] - pair[0]).min().unwrap_or(i64::MAX);
        let across_midnight = first + 24 * 60 - last;
        if across_midnight < within_days && self.fires_on_consecutive_days() {
            across_midnight
        } else {
            within_days
        }
    }

    /// Walks forward from `after`: to the next month when the month is wrong, the next day when
    /// the day is, then by hours and minutes. Clock changes follow the local clock: a time a
    /// change skips does not run that day, and a time it repeats runs each time it comes.
    fn next_in(&self, after: i64, zone: &impl Zone) -> Option<i64> {
        let horizon = after.checked_add(SEARCH_HORIZON_SECS)?;
        let mut t = after.checked_sub(after.rem_euclid(60))?.checked_add(60)?;
        while t <= horizon {
            let now = zone.civil(t)?;
            let next = if !self.month_matches(now.month) {
                let (year, month) = if now.month == 12 { (now.year + 1, 1) } else { (now.year, now.month + 1) };
                zone.day_start(year, month, 1)?
            } else if !self.day_matches(now.day, now.weekday) {
                let (year, month, day) = civil_from_days(days_from_civil(now.year, now.month, now.day) + 1);
                zone.day_start(year, month, day)?
            } else if self.hours & (1 << now.hour) == 0 {
                t.saturating_sub(now.minute as i64 * 60).saturating_add(3600)
            } else if self.minutes & (1 << now.minute) == 0 {
                t.saturating_add(60)
            } else {
                return Some(t);
            };
            // Always forward, whatever a zone hands back.
            t = next.max(t.saturating_add(60));
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

/// `today 9:00 AM`, `tomorrow 9:00 AM`, `Mon 9:00 AM`, `2026-10-01 9:00 AM`: when a run is due.
pub fn when_label(at: i64, now: i64) -> String {
    let tm = local_tm(at);
    let time = clock(tm.tm_hour as u32, tm.tm_min as u32);
    let today = start_of_local_day(now);
    let day = start_of_local_day(at);
    if day == today {
        format!("today {time}")
    } else if day > today && day - today <= 36 * 3600 {
        format!("tomorrow {time}")
    } else if day > today && day - today < 7 * 86_400 {
        format!("{} {time}", DAY_LABELS[tm.tm_wday as usize])
    } else {
        format!("{} {time}", local_time(at).date)
    }
}

#[cfg(test)]
#[path = "schedule_legacy_test.rs"]
mod legacy;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::local_unix;

    fn at(date: &str, clock: &str) -> i64 {
        local_unix(date, Some(clock)).unwrap()
    }

    #[test]
    fn oversized_intervals_reject_without_overflow_or_wraparound() {
        for input in [
            "every 9223372036854775807m", "every 9223372036854775807h",
            "every 9223372036854775807d", "every 144115188075855873d",
        ] {
            assert!(parse(input).is_err(), "accepted {input}");
        }
        assert_eq!(parse("every 366d").unwrap(), Schedule::Every(366 * 86_400));
        assert!(parse("every 367d").is_err());
    }

    #[test]
    fn interval_next_firing_does_not_wrap_timestamp() {
        let schedule = parse("every 5m").unwrap();
        assert_eq!(schedule.next_after(i64::MAX - 300), Some(i64::MAX));
        assert_eq!(schedule.next_after(i64::MAX - 299), None);
        assert_eq!(schedule.next_after(i64::MIN), Some(i64::MIN + 300));
    }

    #[test]
    fn intervals_parse_and_read_back() {
        assert_eq!(parse("every 30m").unwrap(), Schedule::Every(1800));
        assert_eq!(parse("every 2 hours").unwrap(), Schedule::Every(7200));
        assert_eq!(parse("Every day").unwrap(), Schedule::Every(86_400));
        assert_eq!(parse("@hourly").unwrap(), Schedule::Every(3600));
        assert_eq!(parse("every 30m").unwrap().describe(), "Every 30 minutes");
        assert_eq!(parse("every 1h").unwrap().describe(), "Every hour");
        assert_eq!(parse("every 3d").unwrap().describe(), "Every 3 days");
        assert_eq!(parse("every 90m").unwrap().canonical(), "every 90m");
        assert_eq!(parse("every 120m").unwrap().canonical(), "every 2h");
        assert!(parse("every 2m").unwrap_err().contains("too often"));
        assert!(parse("every fortnight").is_err());
        for unit in ["m", "h", "d"] {
            assert!(parse(&format!("every {}{unit}", i64::MAX)).unwrap_err().contains("longer than a year"));
        }
        assert_eq!(parse("every 30m").unwrap().next_after(1000), Some(2800));
    }

    #[test]
    fn cron_reads_in_words() {
        let words = |text: &str| parse(text).unwrap().describe();
        assert_eq!(words("0 9 * * *"), "Every day at 9:00 AM");
        assert_eq!(words("30 17 * * 1-5"), "Weekdays at 5:30 PM");
        assert_eq!(words("0 9 * * mon-fri"), "Weekdays at 9:00 AM");
        assert_eq!(words("0 10 * * 0,6"), "Weekends at 10:00 AM");
        assert_eq!(words("0 9 * * 1"), "Every Monday at 9:00 AM");
        assert_eq!(words("0 9 * * 1,3,5"), "Monday, Wednesday, and Friday at 9:00 AM");
        assert_eq!(words("0 9 1 * *"), "On the 1st of every month at 9:00 AM");
        assert_eq!(words("0 9 1,15 * *"), "On the 1st and 15th of every month at 9:00 AM");
        assert_eq!(words("0 9,17 * * *"), "Every day at 9:00 AM and 5:00 PM");
        assert_eq!(words("0 * * * *"), "Every hour");
        assert_eq!(words("15 * * * *"), "Every hour at :15");
        assert_eq!(words("*/15 * * * *"), "Every 15 minutes");
        assert_eq!(words("0 */6 * * *"), "Every 6 hours");
        assert_eq!(words("0 0 * * *"), "Every day at 12:00 AM");
        assert_eq!(words("5 4 * * 1-3"), "Monday, Tuesday, and Wednesday at 4:05 AM");
        assert_eq!(words("0 9 1 jan *"), "Cron 0 9 1 jan *");
        assert!(parse("* * * * *").unwrap_err().contains("too often"));
        assert!(parse("0 25 * * *").is_err());
        assert!(parse("0 9 * *").is_err());
        assert_eq!(parse("0  9 * * 1-5").unwrap().canonical(), "0 9 * * 1-5");
    }

    #[test]
    fn cron_finds_the_next_local_firing() {
        // 2026-09-17 is a Thursday.
        let thursday_noon = at("2026-09-17", "12:00");
        let weekdays = parse("0 9 * * 1-5").unwrap();
        assert_eq!(weekdays.next_after(thursday_noon), Some(at("2026-09-18", "09:00")));
        let friday_ten = at("2026-09-18", "10:00");
        assert_eq!(weekdays.next_after(friday_ten), Some(at("2026-09-21", "09:00")), "skips the weekend");
        assert_eq!(weekdays.next_after(at("2026-09-18", "09:00")), Some(at("2026-09-21", "09:00")), "strictly after");
        assert_eq!(weekdays.next_after(at("2026-09-18", "08:59")), Some(at("2026-09-18", "09:00")));

        let quarter = parse("*/15 * * * *").unwrap();
        assert_eq!(quarter.next_after(at("2026-09-17", "12:07")), Some(at("2026-09-17", "12:15")));
        assert_eq!(quarter.next_after(at("2026-09-17", "12:45")), Some(at("2026-09-17", "13:00")));

        let first = parse("30 8 1 * *").unwrap();
        assert_eq!(first.next_after(thursday_noon), Some(at("2026-10-01", "08:30")));
        let leap = parse("0 0 29 2 *").unwrap();
        assert_eq!(leap.next_after(thursday_noon), Some(at("2028-02-29", "00:00")));
        // Both day fields set: either matches, as cron has it.
        let either = parse("0 9 15 * 1").unwrap();
        assert_eq!(either.next_after(at("2026-09-17", "12:00")), Some(at("2026-09-21", "09:00")));
        assert_eq!(either.next_after(at("2026-10-13", "12:00")), Some(at("2026-10-15", "09:00")));
    }

    #[test]
    fn due_labels_read_like_a_person() {
        let now = at("2026-09-17", "12:00");
        assert_eq!(when_label(at("2026-09-17", "17:30"), now), "today 5:30 PM");
        assert_eq!(when_label(at("2026-09-18", "09:00"), now), "tomorrow 9:00 AM");
        assert_eq!(when_label(at("2026-09-21", "09:00"), now), "Monday 9:00 AM");
        assert_eq!(when_label(at("2026-10-01", "08:30"), now), "2026-10-01 8:30 AM");
    }

    // MARK: - Weeks and macros

    #[test]
    fn weeks_are_stored_as_days() {
        assert_eq!(parse("every 2w").unwrap(), Schedule::Every(14 * 86_400));
        assert_eq!(parse("every 2w").unwrap().canonical(), "every 14d");
        assert_eq!(parse("every 1 week").unwrap().canonical(), "every 7d");
        assert_eq!(parse("Every Week").unwrap().canonical(), "every 7d");
        assert_eq!(parse("every 3 wks").unwrap().canonical(), "every 21d");
        assert_eq!(parse("every\t2w").unwrap().canonical(), "every 14d");
        // In days, as older Runners word the same stored text, and as the apps translate it.
        assert_eq!(parse("every 2w").unwrap().describe(), "Every 14 days");
        assert_eq!(parse("every 52w").unwrap().canonical(), "every 364d");
        assert!(parse("every 53w").unwrap_err().contains("longer than a year"));
        assert!(parse("every 0w").unwrap_err().contains("too often"));
        assert!(parse("every 2 fortnights").is_err());
    }

    #[test]
    fn macros_are_stored_as_five_fields() {
        let stored = |text: &str| parse(text).unwrap().canonical();
        assert_eq!(stored("@weekly"), "0 0 * * 0");
        assert_eq!(stored("@monthly"), "0 0 1 * *");
        assert_eq!(stored("@yearly"), "0 0 1 1 *");
        assert_eq!(stored("@annually"), "0 0 1 1 *");
        assert_eq!(stored("@midnight"), "0 0 * * *");
        assert_eq!(stored(" @WEEKLY "), "0 0 * * 0");
        let words = |text: &str| parse(text).unwrap().describe();
        assert_eq!(words("@weekly"), "Every Sunday at 12:00 AM");
        assert_eq!(words("@monthly"), "On the 1st of every month at 12:00 AM");
        assert_eq!(words("@midnight"), "Every day at 12:00 AM");
        // Month-restricted expressions have always read as the raw line, which the apps show as is.
        assert_eq!(words("@yearly"), "Cron 0 0 1 1 *");
        assert!(parse("@fortnightly").unwrap_err().contains("There is no @fortnightly"));
        assert!(parse("@2m").unwrap_err().contains("too often"));
    }

    #[test]
    fn hourly_and_daily_stay_intervals() {
        // Counted from the last run, not on the hour or at midnight.
        assert_eq!(parse("@hourly").unwrap(), Schedule::Every(3600));
        assert_eq!(parse("@daily").unwrap(), Schedule::Every(86_400));
        assert_eq!(parse("@hourly").unwrap().canonical(), "every 1h");
        assert_eq!(parse("@daily").unwrap().canonical(), "every 1d");
        let armed = at("2026-09-17", "10:17");
        assert_eq!(parse("@daily").unwrap().next_after(armed), Some(at("2026-09-18", "10:17")));
        assert_eq!(parse("@30m").unwrap(), Schedule::Every(1800));
    }

    // MARK: - Requests in any language

    /// The bot writes what people ask for, in any language, from `REQUESTS`: each must read as
    /// meant, here and on older Runners, and in words the apps translate.
    #[test]
    fn each_request_has_one_schedule() {
        let words: Vec<String> = REQUESTS.iter().map(|(_, text)| parse(text).unwrap().describe()).collect();
        assert_eq!(
            words,
            [
                "Every 30 minutes",
                "Every 2 hours",
                "Every day",
                "Every day at 9:00 AM",
                "Weekdays at 8:30 AM",
                "Every 7 days",
                "Every Monday at 9:00 AM",
                "Every 14 days",
                "On the 1st of every month at 9:00 AM",
            ]
        );
        for (request, text) in REQUESTS {
            let canonical = parse(text).unwrap().canonical();
            let old = legacy::parse(&canonical).unwrap_or_else(|error| panic!("{request}: {canonical:?}: {error}"));
            assert_eq!(old.describe(), parse(text).unwrap().describe(), "{request}");
        }
    }

    /// Whichever spelling the bot writes for a request, it is stored the same way.
    #[test]
    fn every_spelling_of_a_request_is_stored_the_same() {
        let stored = |text: &str| parse(text).unwrap().canonical();
        for daily in ["every 1d", "every day", "Every Day", "every 1 day", "every 24h", "every 1440m", "@daily"] {
            assert_eq!(stored(daily), "every 1d", "{daily}");
        }
        for weekly in ["every 1w", "every week", "every 1 week", "every 7d", "every 7 days", "every 168h"] {
            assert_eq!(stored(weekly), "every 7d", "{weekly}");
        }
        for monday in ["0 9 * * 1", "0 9 * * mon", "0 9 * * Monday", "0 9 * * 1 "] {
            assert_eq!(parse(monday).unwrap().describe(), "Every Monday at 9:00 AM", "{monday}");
        }
    }

    /// People's own words are the bot's to read; the schedule it writes is never them. Words
    /// in any language are refused, with the forms to use instead.
    #[test]
    fn words_in_a_language_are_not_a_schedule() {
        for words in ["daily", "diariamente", "todos los días", "täglich", "毎日", "每天", "ежедневно", "rodar diariamente às 9"] {
            let error = parse(words).unwrap_err();
            assert!(error.contains("every 1d") && error.contains("0 9 * * 1-5"), "{words}: {error}");
        }
    }

    // MARK: - Strictness

    #[test]
    fn names_are_strict_in_new_schedules() {
        let words = |text: &str| parse(text).unwrap().describe();
        assert_eq!(words("0 9 * * Mon"), "Every Monday at 9:00 AM");
        assert_eq!(words("0 9 * * monday"), "Every Monday at 9:00 AM");
        assert_eq!(words("0 9 * * tues-thurs"), "Tuesday, Wednesday, and Thursday at 9:00 AM");
        assert_eq!(words("0 9 * * SUN,sat"), "Weekends at 9:00 AM");
        assert_eq!(parse("0 9 1 sept *").unwrap().next_after(at("2026-01-01", "00:00")), Some(at("2026-09-01", "09:00")));
        assert_eq!(parse("0 9 1 January *").unwrap().canonical(), "0 9 1 January *");
        assert!(parse("0 9 * * monkey").unwrap_err().contains("not a day name"));
        assert!(parse("0 9 * * sunflower").unwrap_err().contains("not a day name"));
        assert!(parse("0 9 * * mo").unwrap_err().contains("not a day name"));
        assert!(parse("0 9 1 junk *").unwrap_err().contains("not a month name"));
        assert!(parse("0 9 * mon *").unwrap_err().contains("not a month name"));
        assert!(parse("0 9 * * mon1").unwrap_err().contains("Could not read"));
        assert!(parse("0 mon * * *").unwrap_err().contains("Could not read"));
    }

    #[test]
    fn overflow_is_an_error_not_a_panic() {
        assert!(parse("every 999999999999999d").unwrap_err().contains("longer than a year"));
        assert!(parse("every 9223372036854775807w").unwrap_err().contains("longer than a year"));
        assert!(parse("every 99999999999999999999999999m").unwrap_err().contains("longer than a year"));
        // The step runs past u32::MAX: the range ends instead of wrapping round.
        assert_eq!(parse("59/4294967295 9 * * *").unwrap().describe(), "Every day at 9:59 AM");
        assert_eq!(parse_stored("59/4294967295 9 * * *").unwrap().describe(), "Every day at 9:59 AM");
        assert!(parse("*/99999999999 9 * * *").unwrap_err().contains("Bad step"));
        assert_eq!(parse("every 1d").unwrap().next_after(i64::MAX - 10), None);
        let nine = parse("0 9 * * *").unwrap();
        assert_eq!(nine.next_after(i64::MAX - 10), None);
        assert_eq!(nine.next_after(i64::MIN), None);
        // Past what the C library can convert to a date.
        assert_eq!(nine.next_after(i64::MAX / 2), None);
    }

    #[test]
    fn gaps_count_across_hours_and_midnight() {
        let gap = |text: &str| parse(text).unwrap_err();
        assert!(gap("0,1 * * * *").contains("1 minute apart"));
        assert!(gap("0,56 * * * *").contains("4 minutes apart"), "56 to the next hour's 0");
        assert!(gap("0,58 0,23 * * *").contains("2 minutes apart"), "23:58 to the next day's 0:00");
        assert!(gap("0,58 0,23 * * 1,2").contains("2 minutes apart"), "Monday 23:58 to Tuesday 0:00");
        assert!(gap("0,58 0,23 28,29 2 *").contains("2 minutes apart"), "Feb 28 to Feb 29");
        assert!(gap("0,58 0,23 31,1 * *").contains("2 minutes apart"), "the 31st to the next month's 1st");
        assert!(gap("* * * * *").contains("too often"));
        assert!(parse("*/5 * * * *").is_ok());
        assert!(parse("0,55 * * * *").is_ok());
        assert!(parse("0,58 0,23 * * 1").is_ok(), "Monday alone: no next day to run on");
        assert!(parse("0,58 0,23 28 2 *").is_ok(), "Feb 28 is never followed by Feb 28");
        assert!(parse("0,58 0,23 * * 1,3").is_ok());
    }

    #[test]
    fn dates_that_never_come_are_refused() {
        assert!(parse("0 0 30 2 *").unwrap_err().contains("never runs"));
        assert!(parse("0 0 31 4,6,9,11 *").unwrap_err().contains("never runs"));
        assert!(parse("0 0 31 apr *").unwrap_err().contains("never runs"));
        assert!(parse("0 0 31 2,3 *").is_ok());
        assert!(parse("0 0 29 2 *").is_ok());
        // With a weekday too, either day field matches, and February has Mondays.
        assert!(parse("0 0 30 2 1").is_ok());
    }

    // MARK: - Search

    #[test]
    fn leap_day_search_spans_a_skipped_century_leap_year() {
        let leap = parse("0 0 29 2 *").unwrap();
        let utc = RuleZone::fixed(0);
        assert_eq!(leap.next_in(utc_at(2028, 3, 1, 0, 0), &utc), Some(utc_at(2032, 2, 29, 0, 0)));
        // 2100 is not a leap year: eight years from one Feb 29 to the next.
        assert_eq!(leap.next_in(utc_at(2096, 2, 29, 0, 0), &utc), Some(utc_at(2104, 2, 29, 0, 0)));
        assert_eq!(leap.next_in(utc_at(2096, 3, 1, 0, 0), &RuleZone::fixed(14 * 3600)), Some(utc_at(2104, 2, 29, 0, 0) - 14 * 3600));
        // The machine's own zone.
        assert_eq!(leap.next_after(at("2028-03-01", "00:00")), Some(at("2032-02-29", "00:00")));
        // Before the change: two years on, with nothing found.
        assert_eq!(legacy::parse("0 0 29 2 *").unwrap().next_after(at("2028-03-01", "00:00")), None);
    }

    /// New York's 2026 clock changes: 2:00 AM becomes 3:00 AM on Mar 8, and 2:00 AM becomes 1:00
    /// AM again on Nov 1. These pin down the current rule, which follows the local clock.
    #[test]
    fn clock_changes_follow_the_local_clock() {
        let ny = RuleZone::new_york_2026();
        let next = |text: &str, after: i64| parse(text).unwrap().next_in(after, &ny).unwrap();

        // A daily time stays at that time, 23 or 25 hours apart.
        let spring_eve = ny.at(2026, 3, 7, 9, 0);
        assert_eq!(next("0 9 * * *", spring_eve), ny.at(2026, 3, 8, 9, 0));
        assert_eq!(ny.at(2026, 3, 8, 9, 0) - spring_eve, 23 * 3600);
        let fall_eve = ny.at(2026, 10, 31, 9, 0);
        assert_eq!(next("0 9 * * *", fall_eve), ny.at(2026, 11, 1, 9, 0));
        assert_eq!(ny.at(2026, 11, 1, 9, 0) - fall_eve, 25 * 3600);

        // A time the change skips does not run that day.
        assert!(ny.instants(2026, 3, 8, 2, 30).is_empty());
        assert_eq!(next("30 2 * * *", ny.at(2026, 3, 7, 12, 0)), ny.at(2026, 3, 9, 2, 30));

        // A time the change repeats runs each time, an hour apart.
        let twice = ny.instants(2026, 11, 1, 1, 30);
        assert_eq!(twice.len(), 2);
        assert_eq!(next("30 1 * * *", ny.at(2026, 10, 31, 12, 0)), twice[0]);
        assert_eq!(next("30 1 * * *", twice[0]), twice[1]);
        assert_eq!(next("30 1 * * *", twice[1]), ny.at(2026, 11, 2, 1, 30));

        // A rhythm keeps its real spacing through both changes.
        assert_eq!(next("*/15 * * * *", ny.at(2026, 3, 8, 1, 45)), ny.at(2026, 3, 8, 3, 0));
        assert_eq!(ny.at(2026, 3, 8, 3, 0) - ny.at(2026, 3, 8, 1, 45), 15 * 60);
        let first_145 = ny.instants(2026, 11, 1, 1, 45)[0];
        assert_eq!(next("*/15 * * * *", first_145), ny.instants(2026, 11, 1, 1, 0)[1]);
        assert_eq!(ny.instants(2026, 11, 1, 1, 0)[1] - first_145, 15 * 60);

        // The gap check reads the local clock, so a change can bring two runs closer on its day.
        assert_eq!(next("0,58 1,3 * * *", ny.at(2026, 3, 8, 1, 58)) - ny.at(2026, 3, 8, 1, 58), 2 * 60);
        assert_eq!(next("0,58 1 * * *", ny.instants(2026, 11, 1, 1, 58)[0]) - ny.instants(2026, 11, 1, 1, 58)[0], 2 * 60);
    }

    /// Chile's clocks go from midnight straight to 1:00 AM on 2026-09-06.
    #[test]
    fn a_day_without_a_midnight_starts_when_its_clock_does() {
        let zone = RuleZone::midnight_skip_2026();
        let next = |text: &str, after: i64| parse(text).unwrap().next_in(after, &zone).unwrap();
        assert!(zone.instants(2026, 9, 6, 0, 0).is_empty());
        assert_eq!(zone.day_start(2026, 9, 6), Some(zone.at(2026, 9, 6, 1, 0)));
        assert_eq!(next("0 9 * * *", zone.at(2026, 9, 5, 12, 0)), zone.at(2026, 9, 6, 9, 0));
        assert_eq!(next("0 9 6 9 *", zone.at(2026, 8, 1, 0, 0)), zone.at(2026, 9, 6, 9, 0));
        assert_eq!(next("0 0 * * *", zone.at(2026, 9, 5, 12, 0)), zone.at(2026, 9, 7, 0, 0));
    }

    // MARK: - Older Runners

    /// Older Runners read what this one stores, so each new spelling must mean the same there.
    #[test]
    fn new_canonical_strings_read_the_same_on_older_runners() {
        let from = at("2026-09-17", "12:00");
        let inputs = [
            "every 2w",
            "every 1 week",
            "every 52 weeks",
            "@weekly",
            "@monthly",
            "@yearly",
            "@annually",
            "@midnight",
            "@hourly",
            "@daily",
            "0 9 * * tues-thurs",
            "30 8 1 sept *",
            "0 9 * * Mon-Fri",
            "every 90m",
            "0,55 9 * * *",
            "0 0 31 2,3 *",
        ];
        for input in inputs {
            let new = parse(input).unwrap();
            let canonical = new.canonical();
            let old = legacy::parse(&canonical).unwrap_or_else(|error| panic!("{input} stores {canonical:?}, which an older Runner refuses: {error}"));
            assert_eq!(old.canonical(), canonical, "{input}");
            assert_eq!(old.describe(), new.describe(), "{input}");
            let (mut a, mut b) = (from, from);
            for _ in 0..4 {
                let (n, o) = (new.next_after(a), old.next_after(b));
                assert_eq!(n, o, "{input} after {a}");
                (a, b) = (n.unwrap(), o.unwrap());
            }
        }
    }

    /// The search skips whole months and finds each day's start its own way; it must still land
    /// where the old search did, across every clock change of the machine's zone in two years.
    #[test]
    fn search_lands_where_the_old_search_did() {
        let crons = [
            "0 9 * * 1-5",
            "30 2 * * *",
            "30 1 * * *",
            "*/15 * * * *",
            "0 0 * * *",
            "0 0 1 * *",
            "15 3 29 2 *",
            "0 9 15 * 1",
            "0 0 31 * *",
            "45 23 * 3,10,11 0",
            "0,30 0-3 * * *",
            "0 12 * jan,jul *",
            "5 */7 * * *",
        ];
        let start = at("2026-01-01", "00:00");
        for text in crons {
            let (new, old) = (parse(text).unwrap(), legacy::parse(text).unwrap());
            let mut after = start;
            while after < start + 2 * 365 * 86_400 {
                match old.next_after(after) {
                    Some(old) => assert_eq!(new.next_after(after), Some(old), "{text} after {after}"),
                    // The old search gave up two years on; Feb 29 can be further off than that.
                    None => assert!(new.next_after(after).is_some_and(|t| t > after + 2 * 366 * 86_400), "{text} after {after}"),
                }
                after += 61 * 3600 + 17 * 60 + 13;
            }
        }
    }

    /// What is already saved runs as it did, even where new text is now refused.
    #[test]
    fn saved_schedules_keep_their_meaning() {
        let from = at("2026-09-17", "12:00");
        let saved = [
            "0 9 * * monkey",
            "0,1 * * * *",
            "0,58 0,23 * * *",
            "0 0 30 2 *",
            "0 9 * * Mon-Fri",
            "every 1d",
            "every 2h",
            "*/5 * * * *",
            "0 9 */2 * 1",
        ];
        for text in saved {
            let old = legacy::parse(text).unwrap();
            let now = parse_stored(text).unwrap();
            assert_eq!(now.canonical(), old.canonical(), "{text}");
            assert_eq!(now.describe(), old.describe(), "{text}");
            let (mut a, mut b) = (from, from);
            for _ in 0..4 {
                let (n, o) = (now.next_after(a), old.next_after(b));
                assert_eq!(n, o, "{text} after {a}");
                match (n, o) {
                    (Some(n), Some(o)) => (a, b) = (n, o),
                    _ => break,
                }
            }
        }
        for refused in ["0 9 * * monkey", "0,1 * * * *", "0,58 0,23 * * *", "0 0 30 2 *"] {
            assert!(parse(refused).is_err(), "{refused}");
        }
        for bad in ["", "* * * * *", "0 25 * * *", "0 9 * *", "every 2m", "every fortnight"] {
            assert!(parse_stored(bad).is_err(), "{bad:?}");
            assert!(legacy::parse(bad).is_err(), "{bad:?}");
        }
    }

    /// Either day field matches when both are given, even as a step like `*/2`, as it always has.
    #[test]
    fn day_fields_keep_the_either_rule() {
        let thursday_noon = at("2026-09-17", "12:00");
        // An odd day or a Monday: Saturday the 19th comes first.
        assert_eq!(parse("0 9 */2 * 1").unwrap().next_after(thursday_noon), Some(at("2026-09-19", "09:00")));
        assert_eq!(parse("0 9 1-31 * 1").unwrap().next_after(thursday_noon), Some(at("2026-09-18", "09:00")));
    }

    // MARK: - A zone with fixed clock changes

    /// Unix seconds of a UTC date and time.
    fn utc_at(year: i64, month: u32, day: u32, hour: i64, minute: i64) -> i64 {
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60
    }

    /// A zone with fixed clock changes, so these cases read the same on every machine and need no
    /// time zone database.
    struct RuleZone {
        /// The offset from UTC before the first change, in seconds.
        before: i64,
        /// When each change happens, in UTC, and the offset from then on.
        changes: Vec<(i64, i64)>,
    }

    impl RuleZone {
        fn fixed(offset: i64) -> Self {
            RuleZone { before: offset, changes: Vec::new() }
        }

        fn new_york_2026() -> Self {
            RuleZone {
                before: -5 * 3600,
                changes: vec![(utc_at(2026, 3, 8, 7, 0), -4 * 3600), (utc_at(2026, 11, 1, 6, 0), -5 * 3600)],
            }
        }

        fn midnight_skip_2026() -> Self {
            RuleZone {
                before: -4 * 3600,
                changes: vec![(utc_at(2026, 9, 6, 4, 0), -3 * 3600)],
            }
        }

        fn offset_at(&self, unix: i64) -> i64 {
            self.changes.iter().rev().find(|(at, _)| unix >= *at).map_or(self.before, |(_, offset)| *offset)
        }

        /// The instants that read as a local time: none in a gap, two in an overlap.
        fn instants(&self, year: i64, month: u32, day: u32, hour: i64, minute: i64) -> Vec<i64> {
            let local = utc_at(year, month, day, hour, minute);
            let offsets = std::iter::once(self.before).chain(self.changes.iter().map(|(_, offset)| *offset));
            let mut found: Vec<i64> = offsets.map(|offset| local - offset).filter(|unix| unix + self.offset_at(*unix) == local).collect();
            found.sort();
            found.dedup();
            found
        }

        /// The one instant that reads as a local time.
        fn at(&self, year: i64, month: u32, day: u32, hour: i64, minute: i64) -> i64 {
            let found = self.instants(year, month, day, hour, minute);
            assert_eq!(found.len(), 1, "{year}-{month}-{day} {hour}:{minute} is not one instant");
            found[0]
        }
    }

    impl Zone for RuleZone {
        fn civil(&self, unix: i64) -> Option<Civil> {
            let local = unix.checked_add(self.offset_at(unix))?;
            let days = local.div_euclid(86_400);
            let secs = local.rem_euclid(86_400);
            let (year, month, day) = civil_from_days(days);
            Some(Civil {
                year,
                month,
                day,
                hour: (secs / 3600) as u32,
                minute: (secs % 3600 / 60) as u32,
                weekday: weekday_of(days),
            })
        }

        fn day_start(&self, year: i64, month: u32, day: u32) -> Option<i64> {
            self.instants(year, month, day, 0, 0).first().copied().or_else(|| {
                self.changes
                    .iter()
                    .map(|(at, _)| *at)
                    .find(|at| self.civil(*at).is_some_and(|c| (c.year, c.month, c.day) == (year, month, day)))
            })
        }
    }

    #[test]
    fn calendar_arithmetic_round_trips() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday_of(days_from_civil(2026, 9, 17)), 4, "a Thursday");
        assert_eq!(civil_from_days(days_from_civil(2100, 2, 28) + 1), (2100, 3, 1));
        assert_eq!(civil_from_days(days_from_civil(2000, 2, 28) + 1), (2000, 2, 29));
        for days in days_from_civil(1899, 12, 1)..days_from_civil(2105, 1, 31) {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days);
        }
    }
}
