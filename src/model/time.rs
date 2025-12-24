//! Time axis and timestamp handling.
//!
//! This module provides utilities for working with time information
//! in MF4 files, including absolute timestamps and relative time axes.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Represents the recording start time.
#[derive(Debug, Clone, Copy)]
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
        self.tz_offset_min + self.dst_offset_min
    }

    /// Converts a relative time (in seconds) to an absolute timestamp.
    pub fn relative_to_absolute(&self, relative_seconds: f64) -> f64 {
        self.as_unix_seconds() + relative_seconds
    }

    /// Formats the timestamp as an ISO 8601 string.
    ///
    /// Note: This is a simple implementation. For production use,
    /// consider using chrono or time crate for proper formatting.
    pub fn to_iso8601(&self) -> String {
        let secs = self.timestamp_ns / 1_000_000_000;
        let nanos = (self.timestamp_ns % 1_000_000_000) as u32;
        
        // Simple date/time calculation (not handling leap years perfectly)
        let days_since_epoch = secs / 86400;
        let time_of_day = secs % 86400;
        
        let hours = time_of_day / 3600;
        let minutes = (time_of_day % 3600) / 60;
        let seconds = time_of_day % 60;
        let millis = nanos / 1_000_000;

        // Approximate year/month/day (simplified, not fully accurate)
        let mut year = 1970i64;
        let mut remaining_days = days_since_epoch;
        
        loop {
            let days_in_year = if is_leap_year(year) { 366 } else { 365 };
            if remaining_days < days_in_year {
                break;
            }
            remaining_days -= days_in_year;
            year += 1;
        }

        let (month, day) = day_of_year_to_month_day(remaining_days as u32 + 1, is_leap_year(year));

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            year, month, day, hours, minutes, seconds, millis
        )
    }
}

impl Default for RecordingTime {
    fn default() -> Self {
        RecordingTime {
            timestamp_ns: 0,
            tz_offset_min: 0,
            dst_offset_min: 0,
        }
    }
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn day_of_year_to_month_day(day_of_year: u32, leap: bool) -> (u32, u32) {
    let days_in_months: [u32; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut remaining = day_of_year;
    for (month_idx, &days) in days_in_months.iter().enumerate() {
        if remaining <= days {
            return (month_idx as u32 + 1, remaining);
        }
        remaining -= days;
    }
    (12, 31) // Fallback
}

/// Time axis type for a channel group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeAxisType {
    /// Time in seconds.
    Time,
    /// Angle in radians.
    Angle,
    /// Distance in meters.
    Distance,
    /// Sample index (no time axis).
    Index,
}

impl Default for TimeAxisType {
    fn default() -> Self {
        TimeAxisType::Time
    }
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
        let time = RecordingTime::new(1640000000_000_000_000, 60, 0);
        
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
        let time = RecordingTime::new(1000_000_000_000, 0, 0); // 1000 seconds
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
}
