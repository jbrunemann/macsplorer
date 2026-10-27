//! The ISO 8601 times in a file's `metadata` (`modified`, `accessed`).
//!
//! LocalSend writes them with Dart's `toIso8601String`, which leaves out
//! the time zone for a local time ("2024-05-01T14:03:27.123"), so a time
//! without one is read as local; this side writes UTC with a `Z`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `t` in UTC, to the millisecond: "2024-05-01T12:03:27.123Z".
pub fn format(t: SystemTime) -> Option<String> {
    let since = t.duration_since(UNIX_EPOCH).ok()?;
    let secs = since.as_secs() as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    Some(format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rest / 3600, rest / 60 % 60, rest % 60, since.subsec_millis()))
}

/// Reads "YYYY-MM-DDTHH:MM:SS", with an optional fraction and an optional
/// `Z` or `±HH:MM` (`±HHMM`, `±HH`); a space may stand for the `T`.
pub fn parse(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = s.get(from..from + len)?;
        part.bytes().all(|c| c.is_ascii_digit()).then(|| part.parse().ok())?
    };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut at = 19;
    let mut nanos = 0u32;
    if b.get(at) == Some(&b'.') || b.get(at) == Some(&b',') {
        at += 1;
        let digits = b[at..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        let frac = &s[at..at + digits.min(9)];
        nanos = frac.parse::<u32>().ok()? * 10u32.pow(9 - frac.len() as u32);
        at += digits;
    }
    let wall = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    let secs = match &s[at..] {
        "" => local_to_utc(year, month, day, hour, minute, second)?,
        "Z" | "z" => wall,
        zone => wall - offset(zone)?,
    };
    let secs = u64::try_from(secs).ok()?;
    Some(UNIX_EPOCH + Duration::new(secs, nanos))
}

/// "+02:00", "-0530" or "+02" in seconds east of UTC.
fn offset(zone: &str) -> Option<i64> {
    let sign = match zone.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits: String = zone[1..].chars().filter(|c| *c != ':').collect();
    if !digits.bytes().all(|c| c.is_ascii_digit()) || !matches!(digits.len(), 2 | 4) {
        return None;
    }
    let hours: i64 = digits[..2].parse().ok()?;
    let minutes: i64 = if digits.len() == 4 { digits[2..].parse().ok()? } else { 0 };
    Some(sign * (hours * 3600 + minutes * 60))
}

/// A wall clock time in this computer's time zone, in seconds since 1970.
#[cfg(unix)]
fn local_to_utc(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> Option<i64> {
    // SAFETY: mktime only reads and normalizes the struct it is given.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tm.tm_year = (year - 1900) as libc::c_int;
        tm.tm_mon = (month - 1) as libc::c_int;
        tm.tm_mday = day as libc::c_int;
        tm.tm_hour = hour as libc::c_int;
        tm.tm_min = minute as libc::c_int;
        tm.tm_sec = second as libc::c_int;
        tm.tm_isdst = -1;
        let t = libc::mktime(&mut tm);
        (t != -1).then_some(t as i64)
    }
}

#[cfg(not(unix))]
fn local_to_utc(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> Option<i64> {
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Days since 1970-01-01 of a date in the proleptic Gregorian calendar
/// (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64, millis: u32) -> SystemTime {
        UNIX_EPOCH + Duration::new(secs, millis * 1_000_000)
    }

    #[test]
    fn utc_times_are_written_with_milliseconds() {
        assert_eq!(format(at(0, 0)).unwrap(), "1970-01-01T00:00:00.000Z");
        // 2024-02-29 13:45:07.250 UTC, a leap day.
        assert_eq!(format(at(1_709_214_307, 250)).unwrap(), "2024-02-29T13:45:07.250Z");
    }

    #[test]
    fn times_with_a_zone_are_read() {
        assert_eq!(parse("2024-02-29T13:45:07.250Z"), Some(at(1_709_214_307, 250)));
        assert_eq!(parse("2024-02-29T15:45:07.25+02:00"), Some(at(1_709_214_307, 250)));
        assert_eq!(parse("2024-02-29T08:15:07-0530"), Some(at(1_709_214_307, 0)));
        assert_eq!(parse("2024-02-29 13:45:07Z"), Some(at(1_709_214_307, 0)));
        assert_eq!(parse(&format(at(1_000_000_000, 7)).unwrap()), Some(at(1_000_000_000, 7)));
    }

    #[test]
    fn a_time_without_a_zone_is_local() {
        let local = parse("2024-02-29T13:45:07").unwrap();
        // Whatever this computer's zone, it is within a day of UTC.
        let utc = at(1_709_214_307, 0);
        let apart = local.duration_since(utc).or_else(|_| utc.duration_since(local)).unwrap();
        assert!(apart <= Duration::from_secs(14 * 3600), "{apart:?}");
    }

    #[test]
    fn nonsense_is_not_a_time() {
        for s in ["", "yesterday", "2024-13-01T00:00:00Z", "2024-02-29T25:00:00Z", "2024-02-29T13:45:07+2", "2024-02-29T13:45:07.Z", "1969-12-31T23:59:59Z"] {
            assert_eq!(parse(s), None, "{s}");
        }
    }
}
