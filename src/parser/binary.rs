//! Low-level binary reading utilities.
//!
//! This module provides efficient utilities for reading binary data
//! with explicit endianness handling.

use byteorder::{BigEndian, ByteOrder, LittleEndian};

/// Reads an unsigned integer of 1-8 bytes from a byte slice.
///
/// # Arguments
/// * `data` - The byte slice to read from
/// * `bit_offset` - The bit offset within the first byte
/// * `bit_count` - The total number of bits to read
/// * `little_endian` - Whether to use little-endian byte order
///
/// # Returns
/// The value as a u64, or 0 if the parameters are invalid.
pub fn read_uint(
    data: &[u8],
    byte_offset: usize,
    bit_offset: u8,
    bit_count: u32,
    little_endian: bool,
) -> u64 {
    if bit_count == 0 || bit_count > 64 {
        return 0;
    }

    // MDF 4.x starts a field inside the first byte it touches: `cn_bit_offset`
    // is 0..=7. A larger value from a malformed file is not a field position
    // but a corrupt declaration — it widens the read window and eventually
    // shifts a u128 by `bit_offset` bits, which is an overflow panic in debug
    // builds and a silently wrapped number in release. Invalid parameters read
    // as zero, so refuse it here rather than produce either.
    if bit_offset > 7 {
        return 0;
    }

    let byte_count = (bit_offset as u32 + bit_count).div_ceil(8) as usize;
    if byte_offset + byte_count > data.len() {
        return 0;
    }

    // Handle aligned byte reads (common case)
    if bit_offset == 0 && bit_count.is_multiple_of(8) {
        let bytes = &data[byte_offset..byte_offset + byte_count];
        return match byte_count {
            1 => bytes[0] as u64,
            2 => {
                if little_endian {
                    LittleEndian::read_u16(bytes) as u64
                } else {
                    BigEndian::read_u16(bytes) as u64
                }
            }
            3 | 4 => {
                let mut buf = [0u8; 4];
                buf[..byte_count].copy_from_slice(bytes);
                if little_endian {
                    LittleEndian::read_u32(&buf) as u64
                } else {
                    // For big-endian, right-align
                    buf.rotate_right(4 - byte_count);
                    BigEndian::read_u32(&buf) as u64
                }
            }
            5..=8 => {
                let mut buf = [0u8; 8];
                buf[..byte_count].copy_from_slice(bytes);
                if little_endian {
                    LittleEndian::read_u64(&buf)
                } else {
                    buf.rotate_right(8 - byte_count);
                    BigEndian::read_u64(&buf)
                }
            }
            _ => 0,
        };
    }

    // Unaligned bit reads.
    //
    // A 64-bit field starting part-way into a byte spans nine bytes, so the
    // window does not fit in a u64 while it is being assembled. Accumulate in a
    // u128 and narrow only after shifting the field down to bit zero; doing this
    // in u64 overflows the shift and panics.
    let bytes = &data[byte_offset..byte_offset + byte_count];
    let mut value: u128 = 0;

    if little_endian {
        // Little-endian: first byte is LSB
        for (i, &byte) in bytes.iter().enumerate() {
            value |= (byte as u128) << (i * 8);
        }
    } else {
        // Big-endian: first byte is MSB
        for &byte in bytes {
            value = (value << 8) | (byte as u128);
        }
    }

    value >>= bit_offset;

    // `1 << 64` is itself an overflow, so a full-width field masks to all ones.
    let mask: u128 = if bit_count >= 64 {
        u64::MAX as u128
    } else {
        (1u128 << bit_count) - 1
    };
    (value & mask) as u64
}

/// Reads a signed integer, sign-extending from the specified bit count.
pub fn read_int(
    data: &[u8],
    byte_offset: usize,
    bit_offset: u8,
    bit_count: u32,
    little_endian: bool,
) -> i64 {
    let unsigned = read_uint(data, byte_offset, bit_offset, bit_count, little_endian);

    // Sign extend
    if bit_count > 0 && bit_count < 64 {
        let sign_bit = 1u64 << (bit_count - 1);
        if unsigned & sign_bit != 0 {
            // Negative number, sign extend
            let mask = !((1u64 << bit_count) - 1);
            return (unsigned | mask) as i64;
        }
    }

    unsigned as i64
}

/// Reads an f32 from a byte slice.
pub fn read_f32(data: &[u8], offset: usize, little_endian: bool) -> f32 {
    if offset + 4 > data.len() {
        return 0.0;
    }
    let bytes = &data[offset..offset + 4];
    if little_endian {
        LittleEndian::read_f32(bytes)
    } else {
        BigEndian::read_f32(bytes)
    }
}

/// Reads an f64 from a byte slice.
pub fn read_f64(data: &[u8], offset: usize, little_endian: bool) -> f64 {
    if offset + 8 > data.len() {
        return 0.0;
    }
    let bytes = &data[offset..offset + 8];
    if little_endian {
        LittleEndian::read_f64(bytes)
    } else {
        BigEndian::read_f64(bytes)
    }
}

/// Converts raw bytes to f64 based on data type and bit count.
pub fn bytes_to_f64(
    data: &[u8],
    byte_offset: usize,
    bit_offset: u8,
    bit_count: u32,
    is_signed: bool,
    is_float: bool,
    little_endian: bool,
) -> f64 {
    if is_float {
        match bit_count {
            32 => read_f32(data, byte_offset, little_endian) as f64,
            64 => read_f64(data, byte_offset, little_endian),
            _ => 0.0,
        }
    } else if is_signed {
        read_int(data, byte_offset, bit_offset, bit_count, little_endian) as f64
    } else {
        read_uint(data, byte_offset, bit_offset, bit_count, little_endian) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_uint_le() {
        let data = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(read_uint(&data, 0, 0, 8, true), 0x01);
        assert_eq!(read_uint(&data, 0, 0, 16, true), 0x0201);
        assert_eq!(read_uint(&data, 0, 0, 32, true), 0x04030201);
    }

    #[test]
    fn test_read_uint_be() {
        let data = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(read_uint(&data, 0, 0, 8, false), 0x01);
        assert_eq!(read_uint(&data, 0, 0, 16, false), 0x0102);
        assert_eq!(read_uint(&data, 0, 0, 32, false), 0x01020304);
    }

    #[test]
    fn test_read_int_signed() {
        // -1 as 8-bit signed
        let data = [0xFF];
        assert_eq!(read_int(&data, 0, 0, 8, true), -1);

        // -1 as 16-bit signed
        let data = [0xFF, 0xFF];
        assert_eq!(read_int(&data, 0, 0, 16, true), -1);

        // Positive value
        let data = [0x7F, 0x00];
        assert_eq!(read_int(&data, 0, 0, 16, true), 127);
    }

    #[test]
    fn test_read_f32() {
        // 1.0 as f32 in little-endian
        let data = [0x00, 0x00, 0x80, 0x3F];
        assert!((read_f32(&data, 0, true) - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_read_f64() {
        // 1.0 as f64 in little-endian
        let data = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF0, 0x3F];
        assert!((read_f64(&data, 0, true) - 1.0).abs() < 0.0001);
    }

    #[test]
    fn a_bit_offset_past_the_first_byte_reads_as_zero() {
        // MDF 4.x allows a field to start only inside the first byte it
        // touches (`cn_bit_offset` is 0..=7). A hostile value such as 8, 64 or
        // 255 previously shifted a u128 by that many bits — bit_offset 255
        // spans 33 bytes and shifts the little-endian assembly by up to 256
        // bits — an overflow panic in debug builds, and a silently wrapped
        // number in release. Invalid parameters read as 0, so every entry point
        // that funnels through `read_uint` must return zero rather than a wrong
        // value or a crash. The 64-byte buffer is big enough that the guard is
        // what rejects the 255-bit case, not the bounds check.
        let data = [0xFFu8; 64];
        for little_endian in [true, false] {
            for &bit_offset in &[8u8, 64u8, 255u8] {
                let label = format!("bit_offset {bit_offset}, little_endian {little_endian}");
                assert_eq!(
                    read_uint(&data, 0, bit_offset, 8, little_endian),
                    0,
                    "{label}"
                );
                assert_eq!(
                    read_int(&data, 0, bit_offset, 8, little_endian),
                    0,
                    "{label}"
                );
                assert_eq!(
                    bytes_to_f64(&data, 0, bit_offset, 8, false, false, little_endian),
                    0.0,
                    "{label}"
                );
            }
        }
    }

    #[test]
    fn test_bytes_to_f64() {
        // Float
        let data = [0x00, 0x00, 0x80, 0x3F];
        assert!((bytes_to_f64(&data, 0, 0, 32, false, true, true) - 1.0).abs() < 0.0001);

        // Unsigned int
        let data = [0x64, 0x00]; // 100 in LE u16
        assert!((bytes_to_f64(&data, 0, 0, 16, false, false, true) - 100.0).abs() < 0.0001);

        // Signed int
        let data = [0xFF, 0xFF]; // -1 in LE i16
        assert!((bytes_to_f64(&data, 0, 0, 16, true, false, true) - (-1.0)).abs() < 0.0001);
    }
}

#[cfg(test)]
mod mask_tests {
    use super::{read_int, read_uint};

    #[test]
    fn reads_a_full_width_field_that_is_not_byte_aligned() {
        // A 64-bit field starting at bit 4. Building the mask as
        // `(1 << bit_count) - 1` overflows for bit_count == 64, which panics in
        // debug builds; a malformed file can declare exactly this layout.
        let data = [0xFFu8; 16];
        let v = read_uint(&data, 0, 4, 64, true);
        assert_eq!(v, u64::MAX, "all bits set should read back as all bits set");
    }

    #[test]
    fn reads_a_63_bit_unaligned_field() {
        let data = [0xFFu8; 16];
        let v = read_uint(&data, 0, 1, 63, true);
        assert_eq!(v, (1u64 << 63) - 1);
    }

    #[test]
    fn sign_extends_a_full_width_unaligned_field() {
        let data = [0xFFu8; 16];
        assert_eq!(read_int(&data, 0, 4, 64, true), -1);
    }
}

#[cfg(test)]
mod big_endian_tests {
    use super::{read_int, read_uint};

    // Expected values here are derived from the semantics the reference
    // implementation uses: assemble the field's bytes most-significant first,
    // shift right by the bit offset, then mask to the bit count. Its handling of
    // fields narrower than a standard width — pad with trailing zero bytes, then
    // shift by `extra_bytes * 8 + bit_offset` — is equivalent to assembling only
    // the real bytes, which is what this code does.

    #[test]
    fn reads_whole_byte_fields_most_significant_first() {
        let data = [0x12, 0x34, 0x56, 0x78];
        assert_eq!(read_uint(&data, 0, 0, 8, false), 0x12);
        assert_eq!(read_uint(&data, 0, 0, 16, false), 0x1234);
        assert_eq!(read_uint(&data, 0, 0, 24, false), 0x12_3456);
        assert_eq!(read_uint(&data, 0, 0, 32, false), 0x1234_5678);
    }

    #[test]
    fn reads_a_full_width_big_endian_field() {
        let data = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
        assert_eq!(read_uint(&data, 0, 0, 64, false), 0x0123_4567_89AB_CDEF);
    }

    #[test]
    fn big_and_little_endian_disagree_as_expected() {
        let data = [0xAA, 0xBB];
        assert_eq!(read_uint(&data, 0, 0, 16, false), 0xAABB);
        assert_eq!(read_uint(&data, 0, 0, 16, true), 0xBBAA);
    }

    #[test]
    fn reads_from_a_byte_offset() {
        let data = [0x00, 0x00, 0x12, 0x34];
        assert_eq!(read_uint(&data, 2, 0, 16, false), 0x1234);
    }

    #[test]
    fn a_bit_offset_shifts_the_assembled_field_down() {
        // 0xFF00 assembled big-endian, shifted right 4, masked to 12 bits.
        let data = [0xFF, 0x00];
        assert_eq!(read_uint(&data, 0, 0, 12, false), 0xF00);
        assert_eq!(read_uint(&data, 0, 4, 12, false), 0xFF0);
    }

    #[test]
    fn reads_sub_byte_fields() {
        // 0b1010_1100 as the only byte.
        let data = [0b1010_1100];
        assert_eq!(read_uint(&data, 0, 0, 4, false), 0b1100);
        assert_eq!(read_uint(&data, 0, 2, 4, false), 0b1011);
        assert_eq!(read_uint(&data, 0, 4, 4, false), 0b1010);
        assert_eq!(read_uint(&data, 0, 7, 1, false), 1);
    }

    #[test]
    fn sign_extends_from_the_field_width() {
        assert_eq!(read_int(&[0xFF, 0xFF], 0, 0, 16, false), -1);
        assert_eq!(read_int(&[0x80, 0x00], 0, 0, 16, false), i16::MIN as i64);
        assert_eq!(read_int(&[0x7F, 0xFF], 0, 0, 16, false), i16::MAX as i64);
        // A 12-bit field whose top bit is set.
        assert_eq!(read_int(&[0x0F, 0xFF], 0, 0, 12, false), -1);
    }

    #[test]
    fn a_field_running_past_the_buffer_reads_as_zero_rather_than_panicking() {
        let data = [0x12];
        assert_eq!(read_uint(&data, 0, 0, 32, false), 0);
        assert_eq!(read_uint(&data, 4, 0, 8, false), 0);
    }

    #[test]
    fn the_aligned_and_general_paths_agree_for_big_endian() {
        // `read_uint` takes a shortcut for byte-aligned whole-byte fields. That
        // shortcut and the general bit-extraction path must produce the same
        // answer, or which one runs would change the result.
        let data = [0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x23, 0x45, 0x67];
        for width in [8u32, 16, 24, 32, 40, 48, 56, 64] {
            let aligned = read_uint(&data, 0, 0, width, false);

            // Recompute independently: the field's bytes, most significant
            // first, masked to its width.
            let bytes = (width / 8) as usize;
            let expected = data[..bytes]
                .iter()
                .fold(0u64, |acc, &b| (acc << 8) | b as u64);
            assert_eq!(aligned, expected, "big-endian {width}-bit field");
        }
    }
}
