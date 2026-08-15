//! The shared temporal translation predicate (ADR 0036 decision 6).
//!
//! Lives in the frontend crate because both sides of the boundary consume
//! it: the resolver checks a `lateness` bound at compile time (ADR 0037
//! decision 4) and the runtime checks torsor arithmetic at evaluation time,
//! and the two must agree on the tolerance to the digit.  The runtime
//! re-exports it (`mensura_runtime::temporal`).

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
