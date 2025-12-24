//! Low-level binary reading utilities.
//!
//! This module provides efficient utilities for reading binary data
//! with explicit endianness handling.

use byteorder::{ByteOrder, LittleEndian, BigEndian};

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
pub fn read_uint(data: &[u8], byte_offset: usize, bit_offset: u8, bit_count: u32, little_endian: bool) -> u64 {
    if bit_count == 0 || bit_count > 64 {
        return 0;
    }

    let byte_count = ((bit_offset as u32 + bit_count + 7) / 8) as usize;
    if byte_offset + byte_count > data.len() {
        return 0;
    }

    // Handle aligned byte reads (common case)
    if bit_offset == 0 && bit_count % 8 == 0 {
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

    // Handle unaligned bit reads
    let bytes = &data[byte_offset..byte_offset + byte_count];
    let mut value: u64 = 0;
    
    if little_endian {
        // Little-endian: first byte is LSB
        for (i, &byte) in bytes.iter().enumerate() {
            value |= (byte as u64) << (i * 8);
        }
    } else {
        // Big-endian: first byte is MSB
        for &byte in bytes {
            value = (value << 8) | (byte as u64);
        }
    }

    // Apply bit offset and mask
    value >>= bit_offset;
    let mask = (1u64 << bit_count) - 1;
    value & mask
}

/// Reads a signed integer, sign-extending from the specified bit count.
pub fn read_int(data: &[u8], byte_offset: usize, bit_offset: u8, bit_count: u32, little_endian: bool) -> i64 {
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
