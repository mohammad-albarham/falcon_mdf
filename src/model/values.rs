//! Typed sample values.
//!
//! MF4 channels carry integers of any bit width from 1 to 64, floats, strings
//! and opaque byte blobs. Forcing all of those through `f64` loses information:
//! a `u64` above 2^53 stops being exact, and a byte array such as a CAN frame's
//! payload becomes a meaningless number. [`SignalValues`] preserves the channel's
//! own type instead, and [`SignalValues::to_f64`] remains available where a
//! uniform numeric view is genuinely what is wanted.

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
    /// Fixed-width opaque bytes: byte arrays, MIME samples, CANopen date/time.
    Bytes,
    /// Text.
    Str,
}

impl ValueKind {
    /// Returns true if samples of this kind are integers or floats.
    pub fn is_numeric(&self) -> bool {
        !matches!(self, ValueKind::Bytes | ValueKind::Str)
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
        }
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
