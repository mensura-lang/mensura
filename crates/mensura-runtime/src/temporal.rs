//! Temporal text validation and normalization (ADR 0036 decisions 6 and 7).
//!
//! An `instant` arrives as RFC 3339 text with an explicit UTC offset, is
//! validated on the millisecond grid, normalized to UTC, and re-encoded in
//! the fixed-width form `YYYY-MM-DDTHH:MM:SS.sssZ`; with one zone and one
//! width, lexicographic order is chronological order, so the backend's
//! string comparison stays correct unchanged.  A `date` is exactly
//! `YYYY-MM-DD`.  Both decode or reject (ADR 0034 decision 3), never
//! repair: sub-millisecond fractions, leap-second labels, naive
//! timestamps, and out-of-range years are ingestion errors.

/// Normalize RFC 3339 text to the canonical `YYYY-MM-DDTHH:MM:SS.sssZ`
/// form, or say why it cannot be.
pub fn normalize_instant(s: &str) -> Result<String, String> {
    const STRUCTURE: &str = "not an RFC 3339 timestamp: expected \
         `YYYY-MM-DDTHH:MM:SS[.sss]` followed by `Z` or `+HH:MM`/`-HH:MM`";
    let b = s.as_bytes();
    let structure = |()| STRUCTURE.to_string();

    let year = num(b, 0, 4).map_err(structure)?;
    lit(b, 4, b'-').map_err(structure)?;
    let month = num(b, 5, 2).map_err(structure)?;
    lit(b, 7, b'-').map_err(structure)?;
    let day = num(b, 8, 2).map_err(structure)?;
    if !matches!(b.get(10), Some(b'T' | b't')) {
        return Err(STRUCTURE.into());
    }
    let hour = num(b, 11, 2).map_err(structure)?;
    lit(b, 13, b':').map_err(structure)?;
    let minute = num(b, 14, 2).map_err(structure)?;
    lit(b, 16, b':').map_err(structure)?;
    let second = num(b, 17, 2).map_err(structure)?;

    // Optional fraction: at most three digits (an `instant` carries exactly
    // millisecond resolution; finer input is rejected, not truncated).
    let mut at = 19;
    let mut milli = 0;
    if b.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while matches!(b.get(at), Some(c) if c.is_ascii_digit()) {
            at += 1;
        }
        let digits = at - start;
        if digits == 0 {
            return Err(STRUCTURE.into());
        }
        if digits > 3 {
            return Err(
                "sub-millisecond precision is rejected, not truncated: an `instant` \
                 carries exactly millisecond resolution (ADR 0036)"
                    .into(),
            );
        }
        // Scale a short fraction to milliseconds: `.5` is 500 ms.
        milli = num(b, start, digits).map_err(structure)? * 10i64.pow(3 - digits as u32);
    }

    // Offset: `Z` for UTC or an explicit `+HH:MM`/`-HH:MM`.  A timestamp
    // with no offset is a civil wall-clock reading, not an instant, and is
    // rejected rather than assigned a zone.
    let offset_min = match b.get(at) {
        None => {
            return Err(
                "missing UTC offset: an `instant` needs `Z` or an explicit `+HH:MM`/`-HH:MM`; \
                 a zone-naive timestamp is a civil reading, not a moment (ADR 0036)"
                    .into(),
            );
        }
        Some(b'Z' | b'z') => {
            at += 1;
            0
        }
        Some(sign @ (b'+' | b'-')) => {
            let oh = num(b, at + 1, 2).map_err(structure)?;
            lit(b, at + 3, b':').map_err(structure)?;
            let om = num(b, at + 4, 2).map_err(structure)?;
            if oh > 23 || om > 59 {
                return Err("UTC offset out of range: hours run 00-23, minutes 00-59".into());
            }
            if *sign == b'-' && oh == 0 && om == 0 {
                return Err(
                    "`-00:00` denotes an unknown offset (RFC 3339) and is rejected: \
                     write `Z` or `+00:00` for UTC"
                        .into(),
                );
            }
            at += 6;
            let minutes = oh * 60 + om;
            if *sign == b'-' { -minutes } else { minutes }
        }
        Some(_) => return Err(STRUCTURE.into()),
    };
    if at != b.len() {
        return Err(STRUCTURE.into());
    }

    check_civil_date(year, month, day)?;
    if hour > 23 || minute > 59 {
        return Err("time out of range: hours run 00-23, minutes 00-59".into());
    }
    if second == 60 {
        return Err(
            "leap-second labels are rejected: the seconds field runs 00-59, and \
             `23:59:60` has no slot on the millisecond grid (ADR 0036)"
                .into(),
        );
    }
    if second > 59 {
        return Err("time out of range: seconds run 00-59".into());
    }

    // Normalize to UTC: an exact integer count of minutes moves, then the
    // civil fields are recovered.  Whole-day and in-day parts are split with
    // euclidean arithmetic so a shift across midnight lands on the previous
    // or next calendar day.
    let total_min = days_from_civil(year, month, day) * 24 * 60 + hour * 60 + minute - offset_min;
    let day_min = total_min.rem_euclid(24 * 60);
    let (uy, um, ud) = civil_from_days(total_min.div_euclid(24 * 60));
    if !(1..=9999).contains(&uy) {
        return Err("out of range once normalized to UTC: years run 0001-9999 (ADR 0036)".into());
    }
    Ok(format!(
        "{uy:04}-{um:02}-{ud:02}T{:02}:{:02}:{second:02}.{milli:03}Z",
        day_min / 60,
        day_min % 60,
    ))
}

/// Check that `date` text is exactly `YYYY-MM-DD` and a real calendar day.
pub fn validate_date(s: &str) -> Result<(), String> {
    const STRUCTURE: &str = "not a calendar date: expected exactly `YYYY-MM-DD`";
    let b = s.as_bytes();
    let structure = |()| STRUCTURE.to_string();
    let year = num(b, 0, 4).map_err(structure)?;
    lit(b, 4, b'-').map_err(structure)?;
    let month = num(b, 5, 2).map_err(structure)?;
    lit(b, 7, b'-').map_err(structure)?;
    let day = num(b, 8, 2).map_err(structure)?;
    if b.len() != 10 {
        return Err(STRUCTURE.into());
    }
    check_civil_date(year, month, day)
}

/// Milliseconds since the epoch of a *canonical* instant: the fixed-width
/// UTC form `normalize_instant` produces and storage holds.  Anything else
/// errs; the storage contract is the normal form, so a mismatch here is
/// data corruption rather than input to repair.
pub fn instant_to_ms(s: &str) -> Result<i64, String> {
    let b = s.as_bytes();
    let bad = || format!("`{s}` is not a normalized instant (`YYYY-MM-DDTHH:MM:SS.sssZ`)");
    let field = |at: usize, n: usize| num(b, at, n).map_err(|()| bad());
    let year = field(0, 4)?;
    let month = field(5, 2)?;
    let day = field(8, 2)?;
    let hour = field(11, 2)?;
    let minute = field(14, 2)?;
    let second = field(17, 2)?;
    let milli = field(20, 3)?;
    let seps = lit(b, 4, b'-')
        .and_then(|()| lit(b, 7, b'-'))
        .and_then(|()| lit(b, 10, b'T'))
        .and_then(|()| lit(b, 13, b':'))
        .and_then(|()| lit(b, 16, b':'))
        .and_then(|()| lit(b, 19, b'.'))
        .and_then(|()| lit(b, 23, b'Z'));
    if seps.is_err() || b.len() != 24 {
        return Err(bad());
    }
    if check_civil_date(year, month, day).is_err() || hour > 23 || minute > 59 || second > 59 {
        return Err(bad());
    }
    Ok(days_from_civil(year, month, day) * 86_400_000
        + hour * 3_600_000
        + minute * 60_000
        + second * 1_000
        + milli)
}

/// The canonical text of a millisecond count.  Errs when the point leaves
/// the representable range (years 0001-9999, ADR 0036 decision 6), which is
/// how a translation past either end fails.
pub fn ms_to_instant(ms: i64) -> Result<String, String> {
    let (year, month, day) = civil_from_days(ms.div_euclid(86_400_000));
    if !(1..=9999).contains(&year) {
        return Err("the result leaves the representable range (years 0001-9999, ADR 0036)".into());
    }
    let in_day = ms.rem_euclid(86_400_000);
    let (hour, minute, second, milli) = (
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1_000 % 60,
        in_day % 1_000,
    );
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z"
    ))
}

/// The whole-millisecond predicate of ADR 0036 decision 6: a `time[real]`
/// magnitude (a count of seconds) converts to an exact integer millisecond
/// count, or the operation is rejected.  It never rounds: silent rounding
/// can break key identity and drifts a window grid invisibly.
///
/// The tolerance is one ULP of the converted magnitude (approximated as
/// `|ms| * EPSILON`, within a factor of two of the true ULP for normal
/// values).  This is the ADR's recommended reading; the exact predicate is
/// its open question, owned by the deferred `precision` library, so this is
/// deliberately the simplest faithful implementation.
pub fn whole_milliseconds(seconds: f64) -> Result<i64, String> {
    if !seconds.is_finite() {
        return Err(format!("{seconds} is not a finite duration"));
    }
    let ms = seconds * 1000.0;
    let nearest = ms.round();
    if (ms - nearest).abs() > ms.abs() * f64::EPSILON {
        return Err(format!(
            "{seconds} s is not a whole number of milliseconds: translation is \
             exact-or-error (ADR 0036 decision 6), so the duration is rejected, \
             not rounded"
        ));
    }
    // Past 2^53 adjacent integer counts are no longer exactly representable,
    // and no in-range translation needs one.
    if nearest.abs() >= 9.007_199_254_740_992e15 {
        return Err(format!(
            "duration out of range: {seconds} s exceeds the exactly representable \
             millisecond counts"
        ));
    }
    Ok(nearest as i64)
}

/// Read `n` ASCII digits at `at` as a number; `Err` on anything else.
fn num(b: &[u8], at: usize, n: usize) -> Result<i64, ()> {
    let slice = b.get(at..at + n).ok_or(())?;
    if !slice.iter().all(u8::is_ascii_digit) {
        return Err(());
    }
    Ok(slice
        .iter()
        .fold(0, |acc, d| acc * 10 + i64::from(d - b'0')))
}

fn lit(b: &[u8], at: usize, expect: u8) -> Result<(), ()> {
    if b.get(at) == Some(&expect) {
        Ok(())
    } else {
        Err(())
    }
}

fn check_civil_date(year: i64, month: i64, day: i64) -> Result<(), String> {
    if !(1..=9999).contains(&year) {
        return Err("year out of range: years run 0001-9999 (ADR 0036)".into());
    }
    if !(1..=12).contains(&month) {
        return Err("month out of range: months run 01-12".into());
    }
    if !(1..=days_in_month(year, month)).contains(&day) {
        return Err(format!(
            "day out of range: {year:04}-{month:02} has {} days",
            days_in_month(year, month)
        ));
    }
    Ok(())
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        _ => 28,
    }
}

/// Days since the Unix epoch of a proleptic-Gregorian civil date
/// (Howard Hinnant's `days_from_civil`, exact over the whole range).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
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

    #[test]
    fn utc_input_normalizes_to_fixed_width() {
        // Fraction padding and the lowercase `t`/`z` forms RFC 3339 allows
        // all land on one canonical width.
        for (input, want) in [
            ("2026-08-10T10:07:31.221Z", "2026-08-10T10:07:31.221Z"),
            ("2026-08-10T10:07:31Z", "2026-08-10T10:07:31.000Z"),
            ("2026-08-10T10:07:31.5Z", "2026-08-10T10:07:31.500Z"),
            ("2026-08-10t10:07:31.22z", "2026-08-10T10:07:31.220Z"),
            ("2026-08-10T10:07:31+00:00", "2026-08-10T10:07:31.000Z"),
        ] {
            assert_eq!(normalize_instant(input).as_deref(), Ok(want), "{input}");
        }
    }

    #[test]
    fn offsets_normalize_across_midnight_and_year_boundaries() {
        for (input, want) in [
            ("2026-08-10T10:07:31.221+02:00", "2026-08-10T08:07:31.221Z"),
            ("2026-08-10T01:30:00-05:00", "2026-08-10T06:30:00.000Z"),
            ("2026-08-10T00:10:00+02:00", "2026-08-09T22:10:00.000Z"),
            ("2026-12-31T23:30:00-01:00", "2027-01-01T00:30:00.000Z"),
            ("2024-03-01T01:00:00+02:00", "2024-02-29T23:00:00.000Z"),
        ] {
            assert_eq!(normalize_instant(input).as_deref(), Ok(want), "{input}");
        }
    }

    #[test]
    fn rejections_name_their_rule() {
        for (input, needle) in [
            // Naive timestamps are civil readings, not instants.
            ("2026-08-10T10:07:31.221", "missing UTC offset"),
            // Finer than the millisecond grid is rejected, not truncated.
            ("2026-08-10T10:07:31.2214Z", "sub-millisecond"),
            // A leap-second label has no slot on the grid.
            ("2016-12-31T23:59:60Z", "leap-second"),
            // RFC 3339's unknown-offset form.
            ("2026-08-10T10:07:31-00:00", "unknown offset"),
            // Calendar and clock validity.
            ("2026-02-29T00:00:00Z", "day out of range"),
            ("2026-13-01T00:00:00Z", "month out of range"),
            ("2026-08-10T24:00:00Z", "time out of range"),
            // Normalization must not leave the representable range.
            ("9999-12-31T23:30:00-01:00", "once normalized"),
            // Structure: not a timestamp at all, or trailing garbage.
            ("not-a-time", "RFC 3339"),
            ("2026-08-10 10:07:31Z", "RFC 3339"),
            ("2026-08-10T10:07:31Zx", "RFC 3339"),
            ("2026-08-10T10:07:31.Z", "RFC 3339"),
        ] {
            let e = normalize_instant(input).expect_err(input);
            assert!(e.contains(needle), "{input}: {e}");
        }
    }

    #[test]
    fn round_trip_is_identity_on_the_canonical_form() {
        // The canonical form is a fixed point: normalizing it again changes
        // nothing, so re-ingesting a dump is safe.
        for s in [
            "0001-01-01T00:00:00.000Z",
            "1970-01-01T00:00:00.000Z",
            "2026-08-10T08:07:31.221Z",
            "9999-12-31T23:59:59.999Z",
        ] {
            assert_eq!(normalize_instant(s).as_deref(), Ok(s));
        }
    }

    #[test]
    fn dates_are_exactly_iso_calendar_days() {
        assert_eq!(validate_date("2026-07-31"), Ok(()));
        assert_eq!(validate_date("2024-02-29"), Ok(()));
        for (input, needle) in [
            ("2026-2-9", "YYYY-MM-DD"),
            ("2026/02/09", "YYYY-MM-DD"),
            ("2026-07-31T00:00:00Z", "YYYY-MM-DD"),
            ("2026-02-30", "day out of range"),
            ("2023-02-29", "day out of range"),
            ("2026-00-01", "month out of range"),
            ("0000-01-01", "year out of range"),
            ("July 31, 2026", "YYYY-MM-DD"),
        ] {
            let e = validate_date(input).expect_err(input);
            assert!(e.contains(needle), "{input}: {e}");
        }
    }

    #[test]
    fn instant_ms_round_trips_the_canonical_form() {
        assert_eq!(instant_to_ms("1970-01-01T00:00:00.000Z"), Ok(0));
        assert_eq!(instant_to_ms("1970-01-01T00:00:01.500Z"), Ok(1500));
        assert_eq!(instant_to_ms("1969-12-31T23:59:59.999Z"), Ok(-1));
        for s in [
            "0001-01-01T00:00:00.000Z",
            "1969-12-31T23:59:59.999Z",
            "2026-08-12T08:07:31.221Z",
            "9999-12-31T23:59:59.999Z",
        ] {
            let ms = instant_to_ms(s).expect(s);
            assert_eq!(ms_to_instant(ms).as_deref(), Ok(s), "{s}");
        }
        // Only the canonical form converts: this path reads storage, where
        // anything else is corruption, not input.
        assert!(instant_to_ms("2026-08-12T08:07:31Z").is_err());
        assert!(instant_to_ms("2026-08-12t08:07:31.221z").is_err());
        // Translation past either end of the range errs (ADR 0036).
        let last = instant_to_ms("9999-12-31T23:59:59.999Z").unwrap();
        assert!(ms_to_instant(last + 1).is_err());
        let first = instant_to_ms("0001-01-01T00:00:00.000Z").unwrap();
        assert!(ms_to_instant(first - 1).is_err());
    }

    #[test]
    fn whole_milliseconds_is_exact_or_error() {
        // Whole counts pass, including the zero and negative ones.
        assert_eq!(whole_milliseconds(900.0), Ok(900_000));
        assert_eq!(whole_milliseconds(0.0), Ok(0));
        assert_eq!(whole_milliseconds(-1.5), Ok(-1500));
        assert_eq!(whole_milliseconds(0.001), Ok(1));
        // A fraction of a millisecond is rejected, not rounded (ADR 0036
        // decision 6): rounding would drift a window grid invisibly.
        assert!(
            whole_milliseconds(0.0001)
                .unwrap_err()
                .contains("not rounded")
        );
        assert!(whole_milliseconds(1.0005).is_err());
        assert!(whole_milliseconds(f64::NAN).is_err());
        assert!(whole_milliseconds(f64::INFINITY).is_err());
        assert!(whole_milliseconds(1.0e16).is_err());
    }

    #[test]
    fn civil_conversions_invert_each_other() {
        // Exhaustive round-trip over every day in the representable range
        // would be slow; the era boundaries and leap rules are the risk, so
        // sweep a century around each plus the range ends.
        let mut day = days_from_civil(1, 1, 1);
        assert_eq!(civil_from_days(day), (1, 1, 1));
        for &(y, m, d) in &[
            (1600, 2, 29),
            (1900, 2, 28),
            (1970, 1, 1),
            (2000, 2, 29),
            (2024, 2, 29),
            (9999, 12, 31),
        ] {
            day = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(day), (y, m, d), "{y}-{m}-{d}");
        }
        // A contiguous sweep across the 2000-02-29 era boundary.
        let start = days_from_civil(1999, 12, 1);
        for offset in 0..500 {
            let (y, m, d) = civil_from_days(start + offset);
            assert_eq!(days_from_civil(y, m, d), start + offset);
        }
    }
}
