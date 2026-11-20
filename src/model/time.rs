//! Time axis and timestamp handling.
//!
//! This module provides utilities for working with time information
//! in MF4 files, including absolute timestamps and relative time axes.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Represents the recording start time.
#[derive(Debug, Clone, Copy, Default)]
pub struct RecordingTime {
    /// Nanoseconds since Unix epoch (January 1, 1970 UTC).
    pub timestamp_ns: i64,
    /// Time zone offset in minutes from UTC.
    pub tz_offset_min: i16,
    /// Daylight saving time offset in minutes.
    pub dst_offset_min: i16,
}

impl RecordingTime {
    /// Creates a new RecordingTime.
    pub fn new(timestamp_ns: i64, tz_offset_min: i16, dst_offset_min: i16) -> Self {
        RecordingTime {
            timestamp_ns,
            tz_offset_min,
            dst_offset_min,
        }
    }

    /// Returns the timestamp as seconds since Unix epoch.
    pub fn as_unix_seconds(&self) -> f64 {
        self.timestamp_ns as f64 / 1_000_000_000.0
    }

    /// Returns the timestamp as a SystemTime (if positive).
    pub fn as_system_time(&self) -> Option<SystemTime> {
        if self.timestamp_ns >= 0 {
            let duration = Duration::from_nanos(self.timestamp_ns as u64);
            Some(UNIX_EPOCH + duration)
        } else {
            None
        }
    }

    /// Returns the total UTC offset in minutes (timezone + DST).
    pub fn total_utc_offset_min(&self) -> i16 {
        self.tz_offset_min.saturating_add(self.dst_offset_min)
    }

    /// Converts a relative time (in seconds) to an absolute timestamp.
    pub fn relative_to_absolute(&self, relative_seconds: f64) -> f64 {
        self.as_unix_seconds() + relative_seconds
    }

    /// Formats the timestamp as an ISO 8601 string.
    pub fn to_iso8601(&self) -> String {
        let secs = self.timestamp_ns.div_euclid(1_000_000_000);
        let nanos = self.timestamp_ns.rem_euclid(1_000_000_000) as u32;

        let days_since_epoch = secs.div_euclid(86_400);
        let time_of_day = secs.rem_euclid(86_400) as u32;

        let hours = time_of_day / 3600;
        let minutes = (time_of_day % 3600) / 60;
        let seconds = time_of_day % 60;
        let millis = nanos / 1_000_000;

        let (year, month, day) = civil_from_days(days_since_epoch);

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            year, month, day, hours, minutes, seconds, millis
        )
    }
}

/// Howard Hinnant's civil-from-days algorithm (exact over the proleptic Gregorian calendar,
/// including negative `days_since_unix_epoch` for pre-1970 timestamps).
fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}

#[cfg(test)]
fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Time axis type for a channel group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeAxisType {
    /// Time in seconds.
    #[default]
    Time,
    /// Angle in radians.
    Angle,
    /// Distance in meters.
    Distance,
    /// Sample index (no time axis).
    Index,
}

/// Information about a time axis.
#[derive(Debug, Clone, Default)]
pub struct TimeAxis {
    /// Type of time axis.
    pub axis_type: TimeAxisType,
    /// Physical unit string.
    pub unit: String,
    /// Name of the master channel (if any).
    pub master_channel_name: Option<String>,
}

impl TimeAxis {
    /// Creates a new time axis.
    pub fn new(axis_type: TimeAxisType, unit: String) -> Self {
        TimeAxis {
            axis_type,
            unit,
            master_channel_name: None,
        }
    }

    /// Creates a time axis for seconds.
    pub fn seconds() -> Self {
        TimeAxis::new(TimeAxisType::Time, "s".to_string())
    }

    /// Creates a time axis for angles.
    pub fn angle() -> Self {
        TimeAxis::new(TimeAxisType::Angle, "rad".to_string())
    }

    /// Creates a time axis for distance.
    pub fn distance() -> Self {
        TimeAxis::new(TimeAxisType::Distance, "m".to_string())
    }

    /// Creates an index-based time axis (no physical time).
    pub fn index() -> Self {
        TimeAxis::new(TimeAxisType::Index, "".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recording_time_basic() {
        let time = RecordingTime::new(1_640_000_000_000_000_000, 60, 0);

        assert!((time.as_unix_seconds() - 1640000000.0).abs() < 0.001);
        assert_eq!(time.total_utc_offset_min(), 60);
    }

    #[test]
    fn test_recording_time_system_time() {
        let time = RecordingTime::new(1000000000, 0, 0);
        let sys_time = time.as_system_time();
        assert!(sys_time.is_some());
    }

    #[test]
    fn test_recording_time_relative_to_absolute() {
        let time = RecordingTime::new(1_000_000_000_000, 0, 0); // 1000 seconds
        let absolute = time.relative_to_absolute(5.0);
        assert!((absolute - 1005.0).abs() < 0.001);
    }

    #[test]
    fn test_time_axis() {
        let axis = TimeAxis::seconds();
        assert_eq!(axis.axis_type, TimeAxisType::Time);
        assert_eq!(axis.unit, "s");

        let axis = TimeAxis::angle();
        assert_eq!(axis.axis_type, TimeAxisType::Angle);
    }

    #[test]
    fn test_leap_year() {
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2004));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(2001));
    }

    #[test]
    fn negative_timestamp_iso8601_formats_subsecond_and_date_correctly() {
        let mut t = RecordingTime::new(-1, 0, 0);
        assert_eq!(t.to_iso8601(), "1969-12-31T23:59:59.999Z");

        t.timestamp_ns = -999_999_999;
        assert_eq!(t.to_iso8601(), "1969-12-31T23:59:59.000Z");

        t.timestamp_ns = -86_400_000_000_000;
        assert_eq!(t.to_iso8601(), "1969-12-31T00:00:00.000Z");

        t.tz_offset_min = i16::MIN;
        t.dst_offset_min = -100;
        assert_eq!(t.total_utc_offset_min(), i16::MIN);
    }
}
