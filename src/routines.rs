//! When a routine is due (`docs/resident-agent-design.md` §8).
//!
//! A schedule is written in words by people and by agents, parsed strictly into a value that
//! is stored beside the text, and asked one question: what is the next time after this one.
//! Times are the machine's local time. The date arithmetic is pure; only turning a local
//! date and clock into an instant (and back) asks the system, so a test can run in UTC and a
//! daylight-saving day can be run through the binary with `TZ` set.
pub mod api;
pub mod cli;

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

/// Where a clock reads from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// The machine's own, through the C library. On a platform without it, UTC.
    Local,
    Utc,
}

/// A local date and clock, with Monday as weekday 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub weekday: u32,
}

impl Zone {
    pub fn parts(self, at: i64) -> Parts {
        match self {
            Zone::Utc => utc_parts(at),
            #[cfg(unix)]
            Zone::Local => {
                let time = at as libc::time_t;
                let mut tm: libc::tm = unsafe { std::mem::zeroed() };
                if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
                    return utc_parts(at);
                }
                Parts {
                    year: tm.tm_year + 1900,
                    month: (tm.tm_mon + 1) as u32,
                    day: tm.tm_mday as u32,
                    hour: tm.tm_hour as u32,
                    minute: tm.tm_min as u32,
                    weekday: ((tm.tm_wday + 6) % 7) as u32,
                }
            }
            #[cfg(not(unix))]
            Zone::Local => utc_parts(at),
        }
    }

    /// The instant a local date and clock fall on. A clock the day skips (spring forward) is
    /// read as the next valid minute, which is what the system does with it.
    pub fn at(self, year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Option<i64> {
        match self {
            Zone::Utc => Some(utc_at(year, month, day, hour, minute)),
            #[cfg(unix)]
            Zone::Local => {
                let mut tm: libc::tm = unsafe { std::mem::zeroed() };
                tm.tm_year = year - 1900;
                tm.tm_mon = month as i32 - 1;
                tm.tm_mday = day as i32;
                tm.tm_hour = hour as i32;
                tm.tm_min = minute as i32;
                tm.tm_isdst = -1;
                let at = unsafe { libc::mktime(&mut tm) } as i64;
                (at != -1).then_some(at)
            }
            #[cfg(not(unix))]
            Zone::Local => Some(utc_at(year, month, day, hour, minute)),
        }
    }
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((y + i64::from(month <= 2)) as i32, month, day)
}

fn utc_parts(at: i64) -> Parts {
    let days = at.div_euclid(86_400);
    let secs = at.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Parts {
        year,
        month,
        day,
        hour: (secs / 3600) as u32,
        minute: (secs % 3600 / 60) as u32,
        weekday: (days + 3).rem_euclid(7) as u32,
    }
}

fn utc_at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
    days_from_civil(year, month, day) * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60
}

/// The civil date `offset` days after a local date.
fn add_days(parts: Parts, offset: i64) -> (i32, u32, u32, u32) {
    let days = days_from_civil(parts.year, parts.month, parts.day) + offset;
    let (year, month, day) = civil_from_days(days);
    (year, month, day, (days + 3).rem_euclid(7) as u32)
}

const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const WEEK: u8 = 0b111_1111;
const WEEKDAYS: u8 = 0b001_1111;
const WEEKENDS: u8 = 0b110_0000;

/// What a routine's words mean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    /// Once, at an instant fixed when it was written.
    Once { at: i64 },
    /// On these days (bit 0 is Monday) at these minutes of the day.
    Clock { days: u8, times: Vec<u16> },
    /// Every `secs`, counted from `anchor`; or, within a window of the day, from its start.
    Every {
        secs: i64,
        anchor: i64,
        window: Option<(u16, u16)>,
    },
    /// Five-field cron, as sets of what matches.
    Cron {
        minutes: u64,
        hours: u32,
        days: u32,
        months: u16,
        weekdays: u8,
        any_day: bool,
        any_weekday: bool,
    },
}

impl Schedule {
    pub fn repeats(&self) -> bool {
        !matches!(self, Schedule::Once { .. })
    }

    /// Reads a schedule written in words; `now` fixes the instants that are relative to it.
    pub fn parse(text: &str, now: i64, zone: Zone) -> Result<Schedule> {
        let lowered = text.trim().to_ascii_lowercase();
        let words: Vec<&str> = lowered.split_whitespace().collect();
        let Some(&first) = words.first() else {
            bail!("a schedule needs words, such as `daily at 08:00`");
        };
        match first {
            "at" => {
                ensure!(words.len() == 2, "`at` takes one time: `at 15:00`");
                let minute = clock(words[1])?;
                let next = Schedule::Clock {
                    days: WEEK,
                    times: vec![minute],
                }
                .next_after(now, zone)
                .ok_or_else(|| anyhow::anyhow!("no such time"))?;
                Ok(Schedule::Once { at: next })
            }
            "on" => {
                ensure!(
                    words.len() == 4 && words[2] == "at",
                    "`on` reads `on 2026-11-24 at 09:30`"
                );
                let (year, month, day) = date(words[1])?;
                let minute = clock(words[3])?;
                let at = zone
                    .at(
                        year,
                        month,
                        day,
                        u32::from(minute / 60),
                        u32::from(minute % 60),
                    )
                    .ok_or_else(|| anyhow::anyhow!("no such date and time"))?;
                ensure!(at > now, "{text} is already past");
                Ok(Schedule::Once { at })
            }
            "in" => {
                ensure!(words.len() == 2, "`in` takes one length: `in 90m`");
                let secs = length(words[1])?;
                ensure!(secs <= 366 * 86_400, "`in` reaches a year ahead at most");
                Ok(Schedule::Once { at: now + secs })
            }
            "daily" | "weekdays" | "weekends" => {
                ensure!(
                    words.get(1) == Some(&"at") && words.len() > 2,
                    "`{first}` reads `{first} at 08:00`"
                );
                let days = match first {
                    "daily" => WEEK,
                    "weekdays" => WEEKDAYS,
                    _ => WEEKENDS,
                };
                Ok(Schedule::Clock {
                    days,
                    times: clocks(&words[2..])?,
                })
            }
            "every" => {
                ensure!(words.len() > 1, "`every` needs what to repeat on");
                if words[1].starts_with(|c: char| c.is_ascii_digit()) {
                    let secs = length(words[1])?;
                    ensure!(secs >= 60, "an interval is a minute at least");
                    let window = match &words[2..] {
                        [] => None,
                        ["from", start, "to", end] => {
                            let (start, end) = (clock(start)?, clock(end)?);
                            ensure!(start < end, "a window ends later in the day than it starts");
                            Some((start, end))
                        }
                        _ => bail!("`every 2h` may end with `from 09:00 to 19:00`"),
                    };
                    return Ok(Schedule::Every {
                        secs,
                        anchor: now,
                        window,
                    });
                }
                ensure!(
                    words.len() >= 4 && words[2] == "at",
                    "`every` reads `every mon,thu at 10:00` or `every 2h`"
                );
                let mut days = 0u8;
                for name in words[1].split(',') {
                    let index = DAYS
                        .iter()
                        .position(|d| name.get(..3).is_some_and(|short| d.starts_with(short)))
                        .ok_or_else(|| anyhow::anyhow!("`{name}` is not a day of the week"))?;
                    days |= 1 << index;
                }
                Ok(Schedule::Clock {
                    days,
                    times: clocks(&words[3..])?,
                })
            }
            "cron" => {
                let fields = &words[1..];
                ensure!(fields.len() == 5, "`cron` takes five fields");
                let minutes = field(fields[0], 0, 59)?;
                let hours = field(fields[1], 0, 23)? as u32;
                // Days and months count from 1; the sets count from bit 0.
                let days = (field(fields[2], 1, 31)? >> 1) as u32;
                let months = (field(fields[3], 1, 12)? >> 1) as u16;
                // Cron's 0 and 7 are both Sunday and 1 is Monday; here Monday is bit 0.
                let named = field(fields[4], 0, 7)?;
                let weekdays = (0..8u32)
                    .filter(|d| named >> d & 1 == 1)
                    .fold(0u8, |set, d| set | 1 << ((d + 6) % 7));
                Ok(Schedule::Cron {
                    minutes,
                    hours,
                    days,
                    months,
                    weekdays,
                    any_day: fields[2].starts_with('*'),
                    any_weekday: fields[4].starts_with('*'),
                })
            }
            _ => {
                bail!("a schedule starts with at, on, in, daily, weekdays, weekends, every or cron")
            }
        }
    }

    /// The first time strictly after `after` it is due, if it ever is again.
    pub fn next_after(&self, after: i64, zone: Zone) -> Option<i64> {
        match self {
            Schedule::Once { at } => (*at > after).then_some(*at),
            Schedule::Clock { days, times } => {
                let today = zone.parts(after);
                (0..=8)
                    .flat_map(|offset| {
                        let (year, month, day, weekday) = add_days(today, offset);
                        times.iter().filter_map(move |minute| {
                            (days >> weekday & 1 == 1)
                                .then(|| {
                                    zone.at(
                                        year,
                                        month,
                                        day,
                                        u32::from(minute / 60),
                                        u32::from(minute % 60),
                                    )
                                })
                                .flatten()
                        })
                    })
                    .filter(|at| *at > after)
                    .min()
            }
            Schedule::Every {
                secs,
                anchor,
                window: None,
            } => {
                let n = ((after - anchor).div_euclid(*secs) + 1).max(0);
                Some(anchor + n * secs)
            }
            Schedule::Every {
                secs,
                window: Some((start, end)),
                ..
            } => {
                let today = zone.parts(after);
                (0..=2)
                    .flat_map(|offset| {
                        let (year, month, day, _) = add_days(today, offset);
                        let from = zone.at(
                            year,
                            month,
                            day,
                            u32::from(start / 60),
                            u32::from(start % 60),
                        );
                        let to =
                            zone.at(year, month, day, u32::from(end / 60), u32::from(end % 60));
                        let span = from.zip(to);
                        (0..=(24 * 3600 / secs)).filter_map(move |k| {
                            let (from, to) = span?;
                            let at = from + k * secs;
                            (at <= to).then_some(at)
                        })
                    })
                    .filter(|at| *at > after)
                    .min()
            }
            Schedule::Cron {
                minutes,
                hours,
                days,
                months,
                weekdays,
                any_day,
                any_weekday,
            } => {
                let today = zone.parts(after);
                // Far enough for a leap day that only a given month names.
                for offset in 0..1500 {
                    let (year, month, day, weekday) = add_days(today, offset);
                    if months >> (month - 1) & 1 == 0 {
                        continue;
                    }
                    let in_month = days >> (day - 1) & 1 == 1;
                    let in_week = weekdays >> weekday & 1 == 1;
                    // Cron: when both are restricted, either one matches.
                    let day_ok = match (any_day, any_weekday) {
                        (true, true) => true,
                        (false, true) => in_month,
                        (true, false) => in_week,
                        (false, false) => in_month || in_week,
                    };
                    if !day_ok {
                        continue;
                    }
                    let found = (0..24u32)
                        .filter(|h| hours >> h & 1 == 1)
                        .flat_map(|h| {
                            (0..60u32)
                                .filter(|m| minutes >> m & 1 == 1)
                                .map(move |m| (h, m))
                        })
                        .filter_map(|(h, m)| zone.at(year, month, day, h, m))
                        .filter(|at| *at > after)
                        .min();
                    if found.is_some() {
                        return found;
                    }
                }
                None
            }
        }
    }

    /// The shortest time between two runs among the next few, for the floor on how often a
    /// routine may wake. `None` for one that runs once.
    pub fn shortest_gap(&self, now: i64, zone: Zone) -> Option<i64> {
        match self {
            Schedule::Once { .. } => None,
            Schedule::Every { secs, .. } => Some(*secs),
            _ => {
                let mut previous = self.next_after(now, zone)?;
                let mut shortest = i64::MAX;
                for _ in 0..24 {
                    let Some(next) = self.next_after(previous, zone) else {
                        break;
                    };
                    shortest = shortest.min(next - previous);
                    previous = next;
                }
                (shortest != i64::MAX).then_some(shortest)
            }
        }
    }

    /// The schedule in words, for a list.
    pub fn describe(&self, zone: Zone) -> String {
        let at = |minute: &u16| format!("{:02}:{:02}", minute / 60, minute % 60);
        match self {
            Schedule::Once { at: instant } => {
                let p = zone.parts(*instant);
                format!(
                    "once, {:04}-{:02}-{:02} {:02}:{:02}",
                    p.year, p.month, p.day, p.hour, p.minute
                )
            }
            Schedule::Clock { days, times } => {
                let when = times.iter().map(at).collect::<Vec<_>>().join(", ");
                let who = match *days {
                    WEEK => "daily".to_owned(),
                    WEEKDAYS => "weekdays".to_owned(),
                    WEEKENDS => "weekends".to_owned(),
                    days => format!(
                        "every {}",
                        (0..7)
                            .filter(|d| days >> d & 1 == 1)
                            .map(|d| DAYS[d])
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                };
                format!("{who} at {when}")
            }
            Schedule::Every { secs, window, .. } => {
                let length = if secs % 86_400 == 0 {
                    format!("{}d", secs / 86_400)
                } else if secs % 3600 == 0 {
                    format!("{}h", secs / 3600)
                } else {
                    format!("{}m", secs / 60)
                };
                match window {
                    Some((from, to)) => format!("every {length} from {} to {}", at(from), at(to)),
                    None => format!("every {length}"),
                }
            }
            Schedule::Cron { .. } => "cron".to_owned(),
        }
    }
}

fn clock(word: &str) -> Result<u16> {
    let Some((hour, minute)) = word.split_once(':') else {
        bail!("`{word}` is not a time; write it as 08:00");
    };
    let (Ok(hour), Ok(minute)) = (hour.parse::<u16>(), minute.parse::<u16>()) else {
        bail!("`{word}` is not a time; write it as 08:00");
    };
    ensure!(hour < 24 && minute < 60, "`{word}` is not a time of day");
    Ok(hour * 60 + minute)
}

fn clocks(words: &[&str]) -> Result<Vec<u16>> {
    let mut times = vec![];
    for word in words
        .iter()
        .flat_map(|w| w.split(','))
        .filter(|w| !w.is_empty())
    {
        times.push(clock(word)?);
    }
    ensure!(!times.is_empty(), "a time is needed, such as 08:00");
    times.sort_unstable();
    times.dedup();
    Ok(times)
}

fn date(word: &str) -> Result<(i32, u32, u32)> {
    let parts: Vec<&str> = word.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        bail!("`{word}` is not a date; write it as 2026-11-24");
    };
    let (Ok(year), Ok(month), Ok(day)) = (year.parse(), month.parse(), day.parse()) else {
        bail!("`{word}` is not a date; write it as 2026-11-24");
    };
    ensure!(
        (1..=12).contains(&month) && (1..=31).contains(&day),
        "`{word}` is not a date"
    );
    Ok((year, month, day))
}

/// `90m`, `2h`, `1d` and their spelled-out forms, in seconds.
pub fn length(word: &str) -> Result<i64> {
    let split = word
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| anyhow::anyhow!("`{word}` needs a unit: m, h or d"))?;
    let amount: i64 = word[..split]
        .parse()
        .map_err(|_| anyhow::anyhow!("`{word}` is not a length"))?;
    ensure!(amount > 0, "a length is more than nothing");
    let unit = match &word[split..] {
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86_400,
        other => bail!("`{other}` is not a unit; use m, h or d"),
    };
    Ok(amount * unit)
}

/// One cron field as the set of values it names, from `min` to `max`.
fn field(text: &str, min: u32, max: u32) -> Result<u64> {
    let mut set = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (
                range,
                step.parse::<u32>()
                    .ok()
                    .filter(|s| *s > 0)
                    .ok_or_else(|| anyhow::anyhow!("`{part}` has no step"))?,
            ),
            None => (part, 1),
        };
        let (low, high) = if range == "*" {
            (min, max)
        } else if let Some((low, high)) = range.split_once('-') {
            (number(low)?, number(high)?)
        } else {
            let value = number(range)?;
            (value, if step > 1 { max } else { value })
        };
        ensure!(
            low >= min && high <= max && low <= high,
            "`{part}` is outside {min}-{max}"
        );
        let mut value = low;
        while value <= high {
            set |= 1 << value;
            value += step;
        }
    }
    Ok(set)
}

fn number(text: &str) -> Result<u32> {
    text.parse()
        .map_err(|_| anyhow::anyhow!("`{text}` is not a number"))
}

/// The routines' clock: the real one, or the unix seconds in the file `OROCHI_KEEPER_CLOCK`
/// names, which is how a test moves time (a schedule, the keeper and the lists all read it).
pub fn clock_file() -> Option<std::path::PathBuf> {
    std::env::var_os("OROCHI_KEEPER_CLOCK").map(std::path::PathBuf::from)
}

pub fn now() -> i64 {
    clock_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or_default()
        })
}
