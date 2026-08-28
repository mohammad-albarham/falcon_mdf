//! Unit tests for the parser module.

use falcon_mdf::parser::Mf4Version;

#[test]
fn test_version_from_version_number() {
    // Test all known versions
    assert!(matches!(
        Mf4Version::from_version_number(400),
        Mf4Version::V4_0 { .. }
    ));
    assert!(matches!(
        Mf4Version::from_version_number(410),
        Mf4Version::V4_1 { .. }
    ));
    assert!(matches!(
        Mf4Version::from_version_number(411),
        Mf4Version::V4_1 { raw: 411 }
    ));
    assert!(matches!(
        Mf4Version::from_version_number(420),
        Mf4Version::V4_2 { .. }
    ));

    // Future version
    let future = Mf4Version::from_version_number(500);
    assert!(!future.is_supported());
}

#[test]
fn test_version_comparison() {
    assert!(Mf4Version::V4_0 { raw: 400 } < Mf4Version::V4_1 { raw: 410 });
    assert!(Mf4Version::V4_1 { raw: 410 } < Mf4Version::V4_2 { raw: 420 });
    assert!(Mf4Version::V4_0 { raw: 400 } < Mf4Version::V4_2 { raw: 420 });
}

#[test]
fn test_version_is_supported() {
    assert!(Mf4Version::V4_0 { raw: 400 }.is_supported());
    assert!(Mf4Version::V4_1 { raw: 410 }.is_supported());
    assert!(Mf4Version::V4_2 { raw: 420 }.is_supported());

    let unknown = Mf4Version::Unknown {
        major: 9,
        minor: 99,
        raw: 999,
    };
    assert!(!unknown.is_supported());
}

#[test]
fn test_version_parts() {
    let v40 = Mf4Version::V4_0 { raw: 400 };
    assert_eq!(v40.major(), 4);
    assert_eq!(v40.minor(), 0);

    let v41 = Mf4Version::V4_1 { raw: 411 };
    assert_eq!(v41.major(), 4);
    assert_eq!(v41.minor(), 11);

    let v42 = Mf4Version::V4_2 { raw: 420 };
    assert_eq!(v42.major(), 4);
    assert_eq!(v42.minor(), 20);

    let unknown = Mf4Version::Unknown {
        major: 5,
        minor: 0,
        raw: 500,
    };
    assert_eq!(unknown.major(), 5);
    assert_eq!(unknown.minor(), 0);
}

#[test]
fn test_version_display() {
    // Two-digit minor, matching how the identification block spells it.
    assert_eq!(format!("{}", Mf4Version::V4_0 { raw: 400 }), "4.00");
    assert_eq!(format!("{}", Mf4Version::V4_1 { raw: 411 }), "4.11");
    assert_eq!(format!("{}", Mf4Version::V4_2 { raw: 420 }), "4.20");
    assert_eq!(
        format!(
            "{}",
            Mf4Version::Unknown {
                major: 5,
                minor: 0,
                raw: 500
            }
        ),
        "5.00"
    );
}

#[test]
fn test_version_validate() {
    assert!(Mf4Version::V4_0 { raw: 400 }.validate().is_ok());
    assert!(Mf4Version::V4_1 { raw: 410 }.validate().is_ok());
    assert!(Mf4Version::V4_2 { raw: 420 }.validate().is_ok());

    let unknown = Mf4Version::Unknown {
        major: 9,
        minor: 0,
        raw: 900,
    };
    assert!(unknown.validate().is_err());
}

#[test]
fn test_read_bits_boundary_cases_and_overflow_guards() {
    use falcon_mdf::parser::binary::{read_bits, read_uint};

    // 1. bit_offset >= 8 must return 0 in both LE and BE.
    let full_buf = [0xFFu8; 64];
    for &bo in &[8u8, 9, 64, 128, 255] {
        assert_eq!(read_bits(&full_buf, 0, bo, 8, true), 0);
        assert_eq!(read_bits(&full_buf, 0, bo, 8, false), 0);
        assert_eq!(read_uint(&full_buf, 0, bo, 64, true), 0);
        assert_eq!(read_uint(&full_buf, 0, bo, 64, false), 0);
    }

    // 2. 64-bit unaligned field spanning 9 bytes in little-endian.
    let data_le = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0x11];
    let val_le = read_bits(&data_le, 0, 4, 64, true);
    assert_eq!(val_le, 0x1F0D_EBC9_A785_6341);
    assert_eq!(read_uint(&data_le, 0, 4, 64, true), 0x1F0D_EBC9_A785_6341);

    // 3. 64-bit unaligned field spanning 9 bytes in big-endian.
    let data_be = [0x80, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    let val_be = read_bits(&data_be, 0, 1, 64, false);
    assert_eq!(val_be, 0x0081_0182_0283_0384);
    assert_eq!(read_uint(&data_be, 0, 1, 64, false), 0x0081_0182_0283_0384);

    // 4. Shift with 9th byte (i = 8, shift by 64 bits) in little-endian.
    // Bit 64 is bit 0 of byte 8 (value 0x01). Shifted by bit_offset 1, it becomes bit 63 (0x8000_0000_0000_0000).
    let data_shift = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
    let val_shift = read_bits(&data_shift, 0, 1, 64, true);
    assert_eq!(val_shift, 0x8000_0000_0000_0000);

    // 5. Slice bounds checks: 9 bytes needed for unaligned 64-bit read.
    let short_buf = [0xFFu8; 8];
    assert_eq!(read_bits(&short_buf, 0, 4, 64, true), 0); // 4 bit_offset + 64 bits = 9 bytes > 8
    assert_eq!(read_bits(&short_buf, 0, 4, 64, false), 0);
    assert_eq!(read_bits(&short_buf, 1, 0, 64, true), 0); // 1 + 8 bytes = 9 > 8
    assert_eq!(read_bits(&short_buf, 0, 0, 64, true), u64::MAX); // Aligned 8-byte read fits

    // 6. Invalid bit counts (0 or > 64).
    assert_eq!(read_bits(&full_buf, 0, 0, 0, true), 0);
    assert_eq!(read_bits(&full_buf, 0, 0, 65, true), 0);
    assert_eq!(read_bits(&full_buf, 0, 0, 128, false), 0);

    // 7. Large byte_offset near usize::MAX that would overflow usize addition.
    assert_eq!(read_bits(&full_buf, usize::MAX, 0, 8, true), 0);
    assert_eq!(read_bits(&full_buf, usize::MAX - 1, 0, 16, true), 0);
    assert_eq!(read_uint(&full_buf, usize::MAX, 0, 8, false), 0);
    assert_eq!(read_uint(&full_buf, usize::MAX - 8, 0, 64, true), 0);
    assert_eq!(read_bits(&full_buf, usize::MAX - 8, 4, 64, true), 0);
}
