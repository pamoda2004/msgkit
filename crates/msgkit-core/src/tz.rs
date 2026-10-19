//! IANA time zones for imports whose timestamps carry no offset.
//!
//! The time-zone database is **bundled in the binary** (`jiff-tzdb`, pinned to an exact version), so a given
//! name resolves to the same UTC instant in every binding and on every platform. Nothing here reads the
//! operating system's time-zone files or the system clock; `tests/purity.rs` enforces both.
//!
//! A wall-clock time can be ambiguous or missing around a daylight-saving change:
//!
//! * **repeated** (the clocks went back): the first occurrence is used;
//! * **skipped** (the clocks went forward): the time is read with the offset in force before the change, which
//!   moves it forward by the length of the gap.

use chrono::{DateTime, Datelike, NaiveDateTime, Timelike, Utc};
use jiff::tz::{AmbiguousOffset, TimeZone};

/// The version of the bundled IANA time-zone database (for example `2026e`).
#[must_use]
pub fn tzdata_version() -> &'static str {
    jiff_tzdb::VERSION.unwrap_or("unknown")
}

/// How a wall-clock time relates to the zone's daylight-saving changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolved {
    /// Exactly one instant.
    Exact(DateTime<Utc>),
    /// The clocks went back: the first of two instants was used.
    Repeated(DateTime<Utc>),
    /// The clocks went forward: the time did not exist and was shifted forward by the gap.
    Skipped(DateTime<Utc>),
}

impl Resolved {
    pub(crate) fn instant(self) -> DateTime<Utc> {
        match self {
            Resolved::Exact(t) | Resolved::Repeated(t) | Resolved::Skipped(t) => t,
        }
    }
}

/// A time zone from the bundled database.
pub(crate) struct Zone(TimeZone);

impl Zone {
    /// Looks a zone up by its exact IANA name (`Asia/Colombo`, `America/New_York`, `UTC`, ...).
    ///
    /// The match is case-sensitive: `asia/colombo` is rejected so that every caller spells a zone one way.
    /// Offsets (`+05:30`), abbreviations (`IST`) and empty names are not zone names and are rejected too.
    pub(crate) fn lookup(name: &str) -> Option<Zone> {
        let (canonical, _) = jiff_tzdb::get(name)?;
        if canonical != name {
            return None;
        }
        TimeZone::get(name).ok().map(Zone)
    }

    /// The UTC instant of a local wall-clock time, and whether it fell in a daylight-saving change.
    pub(crate) fn resolve(&self, local: &NaiveDateTime) -> Option<Resolved> {
        let datetime = jiff::civil::DateTime::new(
            i16::try_from(local.year()).ok()?,
            i8::try_from(local.month()).ok()?,
            i8::try_from(local.day()).ok()?,
            i8::try_from(local.hour()).ok()?,
            i8::try_from(local.minute()).ok()?,
            i8::try_from(local.second()).ok()?,
            0,
        )
        .ok()?;
        let ambiguous = self.0.to_ambiguous_zoned(datetime);
        let kind = ambiguous.offset();
        let instant = DateTime::from_timestamp(ambiguous.compatible().ok()?.timestamp().as_second(), 0)?;
        Some(match kind {
            AmbiguousOffset::Unambiguous { .. } => Resolved::Exact(instant),
            AmbiguousOffset::Gap { .. } => Resolved::Skipped(instant),
            AmbiguousOffset::Fold { .. } => Resolved::Repeated(instant),
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    fn utc(zone: &str, local: NaiveDateTime) -> (&'static str, String) {
        let r = Zone::lookup(zone).unwrap().resolve(&local).unwrap();
        let kind = match r {
            Resolved::Exact(_) => "exact",
            Resolved::Repeated(_) => "repeated",
            Resolved::Skipped(_) => "skipped",
        };
        (kind, r.instant().format("%Y-%m-%dT%H:%MZ").to_string())
    }

    #[test]
    fn ordinary_times_are_exact() {
        assert_eq!(utc("Asia/Colombo", at(2024, 6, 1, 12, 0)), ("exact", "2024-06-01T06:30Z".into()));
        assert_eq!(utc("UTC", at(2024, 6, 1, 12, 0)), ("exact", "2024-06-01T12:00Z".into()));
    }

    #[test]
    fn repeated_hours_use_the_first_occurrence() {
        assert_eq!(utc("America/New_York", at(2024, 11, 3, 1, 30)), ("repeated", "2024-11-03T05:30Z".into()));
        assert_eq!(utc("Europe/London", at(2024, 10, 27, 1, 30)), ("repeated", "2024-10-27T00:30Z".into()));
        assert_eq!(
            utc("Australia/Lord_Howe", at(2024, 4, 7, 1, 45)),
            ("repeated", "2024-04-06T14:45Z".into()),
            "a 30-minute change"
        );
    }

    #[test]
    fn skipped_times_shift_forward_by_the_gap() {
        assert_eq!(utc("America/New_York", at(2024, 3, 10, 2, 30)), ("skipped", "2024-03-10T07:30Z".into()));
        assert_eq!(utc("Europe/London", at(2024, 3, 31, 1, 30)), ("skipped", "2024-03-31T01:30Z".into()));
        assert_eq!(
            utc("Australia/Lord_Howe", at(2024, 10, 6, 2, 15)),
            ("skipped", "2024-10-05T15:45Z".into()),
            "a 30-minute gap"
        );
        assert_eq!(utc("Asia/Colombo", at(1996, 5, 25, 0, 15)), ("skipped", "1996-05-24T18:45Z".into()));
    }

    #[test]
    fn the_rules_continue_after_2037() {
        assert_eq!(utc("America/New_York", at(2040, 7, 1, 12, 0)).1, "2040-07-01T16:00Z");
        assert_eq!(utc("Europe/London", at(2040, 7, 1, 12, 0)).1, "2040-07-01T11:00Z");
    }

    #[test]
    fn only_exact_iana_names_are_zones() {
        for good in ["Asia/Colombo", "Asia/Calcutta", "Asia/Kolkata", "America/New_York", "UTC", "Etc/GMT+5"]
        {
            assert!(Zone::lookup(good).is_some(), "{good}");
        }
        for bad in [
            "asia/colombo",
            "ASIA/COLOMBO",
            "utc",
            "IST",
            "+05:30",
            "Z",
            "",
            " ",
            "Asia/Colombo ",
            ":Asia/Colombo",
            "../etc/passwd",
            "Mars/Phobos",
            "Asia/",
        ] {
            assert!(Zone::lookup(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn the_bundled_database_has_a_version() {
        assert_ne!(tzdata_version(), "unknown");
    }
}
