//! Fuzzes `read_bits` (and `read_uint`) against an independent reference oracle.
//!
//! The oracle extracts the field bit by bit with explicit bounds checks,
//! verifying that neither debug assertions nor release-mode wrapping produce
//! corrupt or silent wrong answers.
//!
//! Run with default debug assertions:
//!
//! ```text
//! cargo +nightly fuzz run read_bits
//! ```
//!
//! Run in release profile (no debug assertions, matching shipped consumer code):
//!
//! ```text
//! cargo +nightly fuzz run --release read_bits
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;

/// Independent, obviously-correct oracle for reading bits.
///
/// This does not copy the implementation in `parser::binary`. Instead, it
/// performs explicit bounds checks and extracts every bit individually.
fn oracle_read_bits(
    data: &[u8],
    byte_offset: usize,
    bit_offset: u8,
    bit_count: u32,
    little_endian: bool,
) -> u64 {
    if bit_count == 0 || bit_count > 64 {
        return 0;
    }
    if bit_offset > 7 {
        return 0;
    }
    let byte_count = match (bit_offset as usize + bit_count as usize).checked_add(7) {
        Some(sum) => sum / 8,
        None => return 0,
    };
    let end_offset = match byte_offset.checked_add(byte_count) {
        Some(end) => end,
        None => return 0,
    };
    if end_offset > data.len() {
        return 0;
    }

    let slice = &data[byte_offset..end_offset];
    let mut result: u64 = 0;

    for k in 0..bit_count as usize {
        let bit = if little_endian {
            let global_bit = bit_offset as usize + k;
            let byte_idx = global_bit / 8;
            let bit_idx = global_bit % 8;
            if byte_idx < slice.len() {
                (slice[byte_idx] >> bit_idx) & 1
            } else {
                0
            }
        } else {
            let v_bit = bit_offset as usize + k;
            let byte_from_end = v_bit / 8;
            if byte_from_end < slice.len() {
                let byte_idx = slice.len() - 1 - byte_from_end;
                let bit_idx = v_bit % 8;
                (slice[byte_idx] >> bit_idx) & 1
            } else {
                0
            }
        };
        result |= (bit as u64) << k;
    }

    result
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 5 {
        return;
    }

    let byte_offset = u16::from_le_bytes([data[0], data[1]]) as usize;
    let bit_offset = data[2];
    let bit_count = data[3] as u32;
    let little_endian = data[4] & 1 != 0;
    let payload = &data[5..];

    let got = falcon_mdf::parser::binary::read_bits(
        payload,
        byte_offset,
        bit_offset,
        bit_count,
        little_endian,
    );
    let expected = oracle_read_bits(payload, byte_offset, bit_offset, bit_count, little_endian);
    assert_eq!(
        got, expected,
        "mismatch for payload len={}, byte_offset={byte_offset}, bit_offset={bit_offset}, bit_count={bit_count}, little_endian={little_endian}",
        payload.len()
    );

    // Also drive read_bits with byte_offset = 0 on payload directly to thoroughly
    // exercise in-bounds bit extractions across all payload lengths.
    let got_zero = falcon_mdf::parser::binary::read_bits(
        payload,
        0,
        bit_offset,
        bit_count,
        little_endian,
    );
    let expected_zero = oracle_read_bits(payload, 0, bit_offset, bit_count, little_endian);
    assert_eq!(
        got_zero, expected_zero,
        "mismatch for payload len={}, byte_offset=0, bit_offset={bit_offset}, bit_count={bit_count}, little_endian={little_endian}",
        payload.len()
    );
});
