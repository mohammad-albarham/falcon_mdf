//! Integration and unit tests for WriteCodec (`##DZ` zip types 1, 4, 5) and transposition.

use falcon_mdf::blocks::data_block::{CompressionType, DzBlock, HlBlock};
use falcon_mdf::blocks::ParseBlock;
use falcon_mdf::write::transpose;
use falcon_mdf::{Mf4File, Mf4Writer, SignalValues, WriteCodec};

#[test]
fn transpose_then_un_transpose_is_identity_on_non_multiple_lengths() {
    // Test direct unit roundtrips on buffers whose length is not a multiple of column_size.
    let test_cases: &[(usize, usize)] = &[
        // (buffer length, column size)
        (7, 3),    // lines = 2, prefix = 6, tail = 1
        (8, 3),    // lines = 2, prefix = 6, tail = 2
        (10, 4),   // lines = 2, prefix = 8, tail = 2
        (11, 4),   // lines = 2, prefix = 8, tail = 3
        (2, 4),    // lines = 0, prefix = 0, tail = 2
        (1, 5),    // lines = 0, prefix = 0, tail = 1
        (37, 5),   // lines = 7, prefix = 35, tail = 2
        (101, 17), // lines = 5, prefix = 85, tail = 16
        (255, 13), // lines = 19, prefix = 247, tail = 8
    ];

    for &(len, col_size) in test_cases {
        let original: Vec<u8> = (0..len).map(|i| (i * 17 + 3) as u8).collect();
        let transposed = transpose(&original, col_size);
        assert_eq!(
            transposed.len(),
            original.len(),
            "transposed length must match original length"
        );

        let un_transposed = Mf4File::un_transpose(&transposed, col_size)
            .expect("un_transpose should succeed for valid column_size");
        assert_eq!(
            un_transposed, original,
            "transpose then un_transpose must be identity for len={len}, col_size={col_size}"
        );
    }
}

#[test]
fn transpose_matches_known_permutations() {
    // Exact multiple: size 9, param 3
    let in_9_3: Vec<u8> = (0..9).collect();
    assert_eq!(transpose(&in_9_3, 3), vec![0, 3, 6, 1, 4, 7, 2, 5, 8]);

    // Tail case: size 8, param 3 (lines = 2, prefix = 6, tail = 2)
    let in_8_3: Vec<u8> = (0..8).collect();
    assert_eq!(transpose(&in_8_3, 3), vec![0, 3, 1, 4, 2, 5, 6, 7]);

    // Tail case: size 7, param 3 (lines = 2, prefix = 6, tail = 1)
    let in_7_3: Vec<u8> = (0..7).collect();
    assert_eq!(transpose(&in_7_3, 3), vec![0, 3, 1, 4, 2, 5, 6]);

    // Smaller than column size (lines = 0)
    let in_2_4: Vec<u8> = (0..2).collect();
    assert_eq!(transpose(&in_2_4, 4), vec![0, 1]);

    // Param 0 returns clone
    assert_eq!(transpose(&in_9_3, 0), in_9_3);
}

fn test_roundtrip_codec(
    codec: WriteCodec,
    expected_zip_type: u8,
    expected_compression_type: CompressionType,
    expected_param: u32,
) {
    let mut writer = Mf4Writer::with_start_time_ns(1_700_000_000_000_000_000);
    writer.set_compression(true);
    writer.set_codec(codec);
    assert_eq!(writer.codec(), codec);
    assert!(writer.is_compressed());

    // Record size: 8 (Time f64) + 1 (U8) + 8 (F64) = 17 bytes per record.
    // 53 samples => 53 * 17 = 901 bytes of record data.
    // 901 / 17 = 53 lines (exact multiple per record), with column size = 17.
    let sample_count = 53;
    let times: Vec<f64> = (0..sample_count).map(|i| i as f64 * 0.05).collect();
    let u8_values: Vec<u8> = (0..sample_count).map(|i| (i * 7 + 13) as u8).collect();
    let f64_values: Vec<f64> = (0..sample_count)
        .map(|i| (i as f64 * 1.23456789).sin() * 100.0)
        .collect();

    let group = writer.add_group(&times).unwrap();
    group
        .add_channel_typed("ByteChannel", "raw", SignalValues::U8(u8_values.clone()))
        .unwrap();
    group.add_channel("FloatChannel", "V", &f64_values).unwrap();

    let temp = tempfile::NamedTempFile::new().unwrap();
    writer.write_to_file(temp.path()).unwrap();

    // 1. Verify the raw file structure on disk:
    let raw_bytes = std::fs::read(temp.path()).unwrap();

    let dz_pos = raw_bytes
        .windows(4)
        .position(|w| w == b"##DZ")
        .expect("Written file must contain a ##DZ block");
    let dz_slice = &raw_bytes[dz_pos..];

    let raw_dz_zip_type = dz_slice[26];
    assert_eq!(
        raw_dz_zip_type, expected_zip_type,
        "Raw dz_zip_type byte at offset 26 must match expected zip type"
    );

    let raw_dz_param = u32::from_le_bytes(dz_slice[28..32].try_into().unwrap());
    assert_eq!(
        raw_dz_param, expected_param,
        "Raw dz_zip_parameter at offset 28..32 must match expected parameter"
    );

    let dz_block = DzBlock::parse(dz_slice, dz_pos as u64).expect("DzBlock must parse cleanly");
    assert_eq!(
        dz_block.zip_type, expected_compression_type,
        "Parsed DzBlock compression type must match"
    );
    assert_eq!(
        dz_block.zip_parameter, expected_param,
        "Parsed DzBlock zip_parameter must match"
    );

    let hl_pos = raw_bytes
        .windows(4)
        .position(|w| w == b"##HL")
        .expect("Written file must contain a ##HL block");
    let hl_slice = &raw_bytes[hl_pos..];

    let raw_hl_zip_type = hl_slice[34];
    assert_eq!(
        raw_hl_zip_type, expected_zip_type,
        "Raw hl_zip_type byte at offset 34 must match expected zip type"
    );

    let hl_block = HlBlock::parse(hl_slice, hl_pos as u64).expect("HlBlock must parse cleanly");
    assert_eq!(
        hl_block.zip_type, expected_compression_type,
        "Parsed HlBlock compression type must match"
    );

    // 2. Open via Mf4File reader and assert all channel samples match exactly:
    let file = Mf4File::open(temp.path()).expect("Mf4File::open must succeed on written file");

    let time_ch = file
        .find_channel("Time")
        .expect("Time master channel must be present");
    let read_times = file
        .signal(time_ch)
        .unwrap()
        .values_f64()
        .expect("Time channel must decode as f64");
    assert_eq!(read_times.len(), sample_count);
    assert_eq!(read_times, times, "Every Time sample must match");

    let byte_ch = file
        .find_channel("ByteChannel")
        .expect("ByteChannel must be present");
    let read_bytes = file
        .signal(byte_ch)
        .unwrap()
        .values()
        .expect("ByteChannel must decode");
    assert_eq!(
        read_bytes,
        SignalValues::U8(u8_values.clone()),
        "Every ByteChannel sample must match"
    );

    let float_ch = file
        .find_channel("FloatChannel")
        .expect("FloatChannel must be present");
    let read_floats = file
        .signal(float_ch)
        .unwrap()
        .values_f64()
        .expect("FloatChannel must decode as f64");
    assert_eq!(read_floats.len(), sample_count);
    assert_eq!(
        read_floats, f64_values,
        "Every FloatChannel sample must match"
    );
}

#[test]
fn write_codec_transposed_deflate_roundtrip() {
    // Record size = 8 (Time) + 1 (U8) + 8 (F64) = 17 bytes.
    test_roundtrip_codec(
        WriteCodec::TransposedDeflate,
        1,
        CompressionType::TransposedDeflate,
        17,
    );
}

#[cfg(feature = "lz4")]
#[test]
fn write_codec_lz4_roundtrip() {
    // Non-transposed LZ4 frame: zip type 4, parameter 0.
    test_roundtrip_codec(WriteCodec::Lz4, 4, CompressionType::Lz4, 0);
}

#[cfg(feature = "lz4")]
#[test]
fn write_codec_transposed_lz4_roundtrip() {
    // Transposed LZ4 frame: zip type 5, parameter = record size = 17.
    test_roundtrip_codec(
        WriteCodec::TransposedLz4,
        5,
        CompressionType::TransposedLz4,
        17,
    );
}
