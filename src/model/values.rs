//! Typed sample values.
//!
//! MF4 channels carry integers of any bit width from 1 to 64, floats, strings
//! and opaque byte blobs. Forcing all of those through `f64` loses information:
//! a `u64` above 2^53 stops being exact, and a byte array such as a CAN frame's
//! payload becomes a meaningless number. [`SignalValues`] preserves the channel's
//! own type instead, and [`SignalValues::to_f64`] remains available where a
//! uniform numeric view is genuinely what is wanted.

/// Days from 1970-01-01 to a civil date, for dates from 1901 onwards.
///
/// Howard Hinnant's `days_from_civil`, which is exact over the whole proleptic
/// Gregorian calendar and needs no dependency. Used to place a CANopen date on
/// the Unix epoch.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let d = day as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// A CANopen date sample — MF4 data type 12, seven bytes.
///
/// A broken-down local calendar time, not an instant: it carries no time zone,
/// and the day-of-week and summer-time fields cannot be recovered from a
/// timestamp. Kept as its own type for that reason; use
/// [`CanopenDate::to_unix_nanos`] where an instant is what is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanopenDate {
    /// Full year. The field on disk counts from 1984 and is seven bits, so the
    /// representable range is 1984 to 2111.
    pub year: u16,
    /// Month, 1 to 12.
    pub month: u8,
    /// Day of month, 1 to 31.
    pub day: u8,
    /// Hour, 0 to 23.
    pub hour: u8,
    /// Minute, 0 to 59.
    pub minute: u8,
    /// Milliseconds within the minute, 0 to 59,999 — seconds included.
    pub ms: u16,
    /// Day of week, 1 (Monday) to 7 (Sunday); 0 when the writer left it unset.
    ///
    /// Redundant with the date, and stored anyway, so it is preserved rather
    /// than recomputed: a file whose two disagree is saying something.
    pub day_of_week: u8,
    /// Whether the writer marked this time as summer time.
    pub summer_time: bool,
}

impl CanopenDate {
    /// Converts to nanoseconds since the Unix epoch, treating the fields as UTC.
    ///
    /// The format records no time zone, so a caller who knows the measurement's
    /// offset must apply it. `day_of_week` and `summer_time` are not
    /// representable in the result.
    pub fn to_unix_nanos(&self) -> i64 {
        let days = days_from_civil(self.year as i64, self.month as u32, self.day as u32);
        let secs = days * 86_400 + self.hour as i64 * 3_600 + self.minute as i64 * 60;
        secs * 1_000_000_000 + self.ms as i64 * 1_000_000
    }
}

/// A CANopen time sample — MF4 data type 13, six bytes.
///
/// An elapsed time from a fixed epoch, which is what makes it unlike
/// [`CanopenDate`]: both fields together are exactly an instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanopenTime {
    /// Milliseconds since midnight. The field on disk is 28 bits.
    pub ms_since_midnight: u32,
    /// Days since 1984-01-01, the CANopen epoch.
    pub days_since_1984: u16,
}

impl CanopenTime {
    /// Days from the Unix epoch to the CANopen epoch of 1984-01-01.
    const EPOCH_DAYS: i64 = 5_113;

    /// Converts to nanoseconds since the Unix epoch, treating the value as UTC.
    pub fn to_unix_nanos(&self) -> i64 {
        let days = Self::EPOCH_DAYS + self.days_since_1984 as i64;
        days * 86_400 * 1_000_000_000 + self.ms_since_midnight as i64 * 1_000_000
    }
}

/// The Rust type a channel's samples decode to.
///
/// Determined by the channel's raw data type, its bit width, and whether a
/// conversion applies. A channel with a non-identity conversion always decodes
/// to [`ValueKind::F64`], because conversions produce physical values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValueKind {
    /// Unsigned integer, up to 8 bits.
    U8,
    /// Unsigned integer, 9 to 16 bits.
    U16,
    /// Unsigned integer, 17 to 32 bits.
    U32,
    /// Unsigned integer, 33 to 64 bits.
    U64,
    /// Signed integer, up to 8 bits.
    I8,
    /// Signed integer, 9 to 16 bits.
    I16,
    /// Signed integer, 17 to 32 bits.
    I32,
    /// Signed integer, 33 to 64 bits.
    I64,
    /// 32-bit float.
    F32,
    /// 64-bit float, and the result of any non-identity conversion.
    F64,
    /// Fixed-width opaque bytes: byte arrays and MIME samples.
    Bytes,
    /// Text.
    Str,
    /// Complex numbers, as a real and an imaginary part per sample.
    Complex,
    /// CANopen broken-down calendar dates.
    CanopenDate,
    /// CANopen elapsed times.
    CanopenTime,
}

impl ValueKind {
    /// Returns true if samples of this kind are integers or floats.
    ///
    /// Complex and the CANopen types are not: none of them has a single
    /// meaningful `f64`, which is what this question is asked in order to decide.
    pub fn is_numeric(&self) -> bool {
        !matches!(
            self,
            ValueKind::Bytes
                | ValueKind::Str
                | ValueKind::Complex
                | ValueKind::CanopenDate
                | ValueKind::CanopenTime
        )
    }

    /// Returns the kind's short name, e.g. `"u32"`.
    pub fn name(&self) -> &'static str {
        match self {
            ValueKind::U8 => "u8",
            ValueKind::U16 => "u16",
            ValueKind::U32 => "u32",
            ValueKind::U64 => "u64",
            ValueKind::I8 => "i8",
            ValueKind::I16 => "i16",
            ValueKind::I32 => "i32",
            ValueKind::I64 => "i64",
            ValueKind::F32 => "f32",
            ValueKind::F64 => "f64",
            ValueKind::Bytes => "bytes",
            ValueKind::Str => "str",
            ValueKind::Complex => "complex",
            ValueKind::CanopenDate => "canopen_date",
            ValueKind::CanopenTime => "canopen_time",
        }
    }
}

impl std::fmt::Display for ValueKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Decoded samples, in the channel's own type.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SignalValues {
    /// Unsigned 8-bit samples.
    U8(Vec<u8>),
    /// Unsigned 16-bit samples.
    U16(Vec<u16>),
    /// Unsigned 32-bit samples.
    U32(Vec<u32>),
    /// Unsigned 64-bit samples.
    U64(Vec<u64>),
    /// Signed 8-bit samples.
    I8(Vec<i8>),
    /// Signed 16-bit samples.
    I16(Vec<i16>),
    /// Signed 32-bit samples.
    I32(Vec<i32>),
    /// Signed 64-bit samples.
    I64(Vec<i64>),
    /// 32-bit float samples.
    F32(Vec<f32>),
    /// 64-bit float samples, including all converted physical values.
    F64(Vec<f64>),
    /// Fixed-width byte samples, stored flat.
    ///
    /// Every sample occupies exactly `width` bytes; use [`SignalValues::bytes_at`]
    /// to address one. Storing flat avoids an allocation per sample, which
    /// matters for bus-logging channels with millions of frames.
    Bytes {
        /// All samples concatenated.
        data: Vec<u8>,
        /// Bytes per sample.
        width: usize,
    },
    /// Variable-width byte samples, stored flat with an index.
    ///
    /// Produced by variable-length channels whose payloads differ in size. When
    /// every payload happens to be the same size — a CAN log of full frames,
    /// say — [`SignalValues::Bytes`] is produced instead, since a fixed width is
    /// simpler to work with and is what other readers report.
    VarBytes {
        /// All samples concatenated.
        data: Vec<u8>,
        /// Start of each sample, with a final entry marking the end. Length is
        /// therefore one more than the sample count.
        ///
        /// `usize`, not `u32`: a channel's payloads can exceed four gigabytes,
        /// and a narrowing cast would silently point at the wrong bytes.
        starts: Vec<usize>,
    },
    /// Text samples.
    Str(Vec<String>),
    /// Complex samples, split into parallel real and imaginary parts.
    ///
    /// Both vectors hold one entry per sample. Split rather than interleaved so
    /// that taking the real part of a channel is a slice, not a stride.
    Complex {
        /// Real parts, one per sample.
        re: Vec<f64>,
        /// Imaginary parts, one per sample.
        im: Vec<f64>,
    },
    /// CANopen date samples — broken-down local calendar times.
    CanopenDate(Vec<CanopenDate>),
    /// CANopen time samples — elapsed time from the 1984 epoch.
    CanopenTime(Vec<CanopenTime>),
    /// Fixed-size array samples, decoded as flat f64 values.
    ///
    /// Each sample contributes `elements_per_sample` values to the flat
    /// `values` vector, so element `j` of sample `i` is at
    /// `values[i * elements_per_sample + j]`. Use
    /// [`Channel::array_shape`](crate::Channel::array_shape) to recover the
    /// per-dimension sizes.
    Array {
        /// Flat element values in row-major order, converted to f64.
        values: Vec<f64>,
        /// Total number of elements per sample (product of all dimensions).
        elements_per_sample: usize,
    },
}

impl SignalValues {
    /// Returns the number of samples.
    pub fn len(&self) -> usize {
        match self {
            SignalValues::U8(v) => v.len(),
            SignalValues::U16(v) => v.len(),
            SignalValues::U32(v) => v.len(),
            SignalValues::U64(v) => v.len(),
            SignalValues::I8(v) => v.len(),
            SignalValues::I16(v) => v.len(),
            SignalValues::I32(v) => v.len(),
            SignalValues::I64(v) => v.len(),
            SignalValues::F32(v) => v.len(),
            SignalValues::F64(v) => v.len(),
            SignalValues::Bytes { data, width } => {
                if *width == 0 {
                    0
                } else {
                    data.len() / width
                }
            }
            SignalValues::VarBytes { starts, .. } => starts.len().saturating_sub(1),
            SignalValues::Str(v) => v.len(),
            SignalValues::Complex { re, .. } => re.len(),
            SignalValues::CanopenDate(v) => v.len(),
            SignalValues::CanopenTime(v) => v.len(),
            SignalValues::Array {
                values,
                elements_per_sample,
            } => {
                if *elements_per_sample == 0 {
                    0
                } else {
                    values.len() / elements_per_sample
                }
            }
        }
    }

    /// Returns true if there are no samples.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the kind of these values.
    pub fn kind(&self) -> ValueKind {
        match self {
            SignalValues::U8(_) => ValueKind::U8,
            SignalValues::U16(_) => ValueKind::U16,
            SignalValues::U32(_) => ValueKind::U32,
            SignalValues::U64(_) => ValueKind::U64,
            SignalValues::I8(_) => ValueKind::I8,
            SignalValues::I16(_) => ValueKind::I16,
            SignalValues::I32(_) => ValueKind::I32,
            SignalValues::I64(_) => ValueKind::I64,
            SignalValues::F32(_) => ValueKind::F32,
            SignalValues::F64(_) => ValueKind::F64,
            SignalValues::Bytes { .. } | SignalValues::VarBytes { .. } => ValueKind::Bytes,
            SignalValues::Str(_) => ValueKind::Str,
            SignalValues::Complex { .. } => ValueKind::Complex,
            SignalValues::CanopenDate(_) => ValueKind::CanopenDate,
            SignalValues::CanopenTime(_) => ValueKind::CanopenTime,
            SignalValues::Array { .. } => ValueKind::F64,
        }
    }

    /// Returns the bytes of one sample, for [`SignalValues::Bytes`] values.
    ///
    /// Returns `None` for other variants, or if `index` is out of range.
    pub fn bytes_at(&self, index: usize) -> Option<&[u8]> {
        match self {
            SignalValues::Bytes { data, width } => {
                if *width == 0 {
                    return None;
                }
                data.get(index * width..(index + 1) * width)
            }
            SignalValues::VarBytes { data, starts } => {
                let from = *starts.get(index)?;
                let to = *starts.get(index + 1)?;
                data.get(from..to)
            }
            _ => None,
        }
    }

    /// Converts every sample to `f64`.
    ///
    /// Lossy in two ways worth knowing about: integers beyond 2^53 lose
    /// precision, and non-numeric samples (bytes, text) have no meaningful
    /// numeric value, so they become `NaN` rather than a misleading number.
    pub fn to_f64(&self) -> Vec<f64> {
        fn cast<T: Copy + Into<f64>>(v: &[T]) -> Vec<f64> {
            v.iter().map(|&x| x.into()).collect()
        }
        match self {
            SignalValues::U8(v) => cast(v),
            SignalValues::U16(v) => cast(v),
            SignalValues::U32(v) => cast(v),
            SignalValues::U64(v) => v.iter().map(|&x| x as f64).collect(),
            SignalValues::I8(v) => cast(v),
            SignalValues::I16(v) => cast(v),
            SignalValues::I32(v) => cast(v),
            SignalValues::I64(v) => v.iter().map(|&x| x as f64).collect(),
            SignalValues::F32(v) => cast(v),
            SignalValues::F64(v) => v.clone(),
            SignalValues::Bytes { .. } | SignalValues::VarBytes { .. } | SignalValues::Str(_) => {
                vec![f64::NAN; self.len()]
            }
            // A complex number has no single real value, and a date is a
            // calendar record rather than a scalar. NaN says so; picking the
            // real part, or an epoch offset, would be a silent choice made on
            // the caller's behalf. `to_unix_nanos` is the explicit route.
            SignalValues::Complex { .. }
            | SignalValues::CanopenDate(_)
            | SignalValues::CanopenTime(_) => vec![f64::NAN; self.len()],
            SignalValues::Array { values, .. } => values.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_length_per_variant() {
        assert_eq!(SignalValues::U8(vec![1, 2, 3]).len(), 3);
        assert_eq!(SignalValues::F64(vec![]).len(), 0);
        assert!(SignalValues::F64(vec![]).is_empty());
    }

    #[test]
    fn variable_width_sample_starts_are_wide_enough_for_large_files() {
        // Companion to the guard in `vlsd`: these offsets index the same
        // payload data, so narrowing them would reintroduce the same silent
        // mis-addressing above four gigabytes.
        fn starts_are_usize(_: &[usize]) {}
        let v = SignalValues::VarBytes {
            data: vec![1, 2, 3],
            starts: vec![0, 2, 3],
        };
        if let SignalValues::VarBytes { starts, .. } = &v {
            starts_are_usize(starts);
        }
        assert_eq!(v.len(), 2);
        assert_eq!(v.bytes_at(0), Some(&[1, 2][..]));
        assert_eq!(v.bytes_at(1), Some(&[3][..]));
    }

    #[test]
    fn byte_samples_are_addressed_by_width() {
        let v = SignalValues::Bytes {
            data: vec![1, 2, 3, 4, 5, 6],
            width: 3,
        };
        assert_eq!(v.len(), 2);
        assert_eq!(v.bytes_at(0), Some(&[1, 2, 3][..]));
        assert_eq!(v.bytes_at(1), Some(&[4, 5, 6][..]));
        assert_eq!(v.bytes_at(2), None);
    }

    #[test]
    fn zero_width_bytes_do_not_divide_by_zero() {
        let v = SignalValues::Bytes {
            data: vec![1, 2, 3],
            width: 0,
        };
        assert_eq!(v.len(), 0);
        assert_eq!(v.bytes_at(0), None);
    }

    #[test]
    fn bytes_at_returns_none_for_numeric_variants() {
        assert_eq!(SignalValues::U8(vec![1, 2]).bytes_at(0), None);
    }

    #[test]
    fn converts_numeric_variants_to_f64() {
        assert_eq!(SignalValues::U16(vec![7, 9]).to_f64(), vec![7.0, 9.0]);
        assert_eq!(SignalValues::I8(vec![-3]).to_f64(), vec![-3.0]);
        assert_eq!(SignalValues::F32(vec![0.5]).to_f64(), vec![0.5]);
    }

    #[test]
    fn non_numeric_variants_convert_to_nan_not_a_wrong_number() {
        let v = SignalValues::Bytes {
            data: vec![0xFF; 16],
            width: 8,
        };
        let f = v.to_f64();
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|x| x.is_nan()));
    }

    #[test]
    fn large_u64_values_survive_as_integers() {
        // 2^63 + 1 is not representable in f64; the typed variant keeps it.
        let big = (1u64 << 63) + 1;
        let v = SignalValues::U64(vec![big]);
        assert_eq!(v, SignalValues::U64(vec![big]));
        assert_ne!(
            v.to_f64()[0] as u64,
            big,
            "f64 round-trip is lossy, as documented"
        );
    }

    #[test]
    fn kind_round_trips() {
        assert_eq!(SignalValues::U32(vec![]).kind(), ValueKind::U32);
        assert_eq!(SignalValues::U32(vec![]).kind().name(), "u32");
        assert!(ValueKind::I16.is_numeric());
        assert!(!ValueKind::Bytes.is_numeric());
        assert!(!ValueKind::Str.is_numeric());
    }
}

#[cfg(test)]
mod canopen_tests {
    use super::*;

    #[test]
    fn days_from_civil_matches_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(1984, 1, 1), 5_113);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        // 2000 was a leap year and 1900 was not; a naive rule gets this wrong.
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
        assert_eq!(days_from_civil(2026, 8, 3), 20_668);
    }

    #[test]
    fn the_canopen_epoch_is_where_the_time_type_counts_from() {
        // CanopenTime::EPOCH_DAYS is asserted against the same algorithm the
        // date type uses, so the two cannot drift apart.
        assert_eq!(CanopenTime::EPOCH_DAYS, days_from_civil(1984, 1, 1));
    }

    #[test]
    fn a_canopen_date_places_itself_on_the_unix_epoch() {
        let d = CanopenDate {
            year: 1984,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            ms: 0,
            day_of_week: 7,
            summer_time: false,
        };
        assert_eq!(d.to_unix_nanos(), 5_113 * 86_400 * 1_000_000_000);

        // 2026-08-03T12:34:56.789Z. The ms field spans the whole minute, so
        // the seconds live inside it.
        let d = CanopenDate {
            year: 2026,
            month: 8,
            day: 3,
            hour: 12,
            minute: 34,
            ms: 56_789,
            day_of_week: 1,
            summer_time: true,
        };
        let expected =
            (20_668i64 * 86_400 + 12 * 3_600 + 34 * 60) * 1_000_000_000 + 56_789 * 1_000_000;
        assert_eq!(d.to_unix_nanos(), expected);
    }

    #[test]
    fn a_canopen_time_places_itself_on_the_unix_epoch() {
        let t = CanopenTime {
            ms_since_midnight: 0,
            days_since_1984: 0,
        };
        assert_eq!(t.to_unix_nanos(), 5_113 * 86_400 * 1_000_000_000);

        // The two types must agree on the same instant.
        let days = days_from_civil(2026, 8, 3) - days_from_civil(1984, 1, 1);
        let t = CanopenTime {
            ms_since_midnight: (12 * 3_600 + 34 * 60) * 1_000 + 56_789,
            days_since_1984: days as u16,
        };
        let d = CanopenDate {
            year: 2026,
            month: 8,
            day: 3,
            hour: 12,
            minute: 34,
            ms: 56_789,
            day_of_week: 1,
            summer_time: false,
        };
        assert_eq!(t.to_unix_nanos(), d.to_unix_nanos());
    }

    #[test]
    fn the_new_kinds_are_not_numeric() {
        // to_f64 has no honest answer for any of them, so callers testing
        // is_numeric before converting must be told no.
        for k in [
            ValueKind::Complex,
            ValueKind::CanopenDate,
            ValueKind::CanopenTime,
        ] {
            assert!(!k.is_numeric(), "{k} must not claim to be numeric");
        }
    }
}
