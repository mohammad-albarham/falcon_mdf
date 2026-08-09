//! Integration tests for the falcon_mdf crate.
//!
//! These tests verify the overall functionality of the crate by creating
//! synthetic MF4 data and reading it back.

use std::io::Write;
use tempfile::NamedTempFile;

use falcon_mdf::blocks::*;
use falcon_mdf::parser::Mf4Version;

/// Creates a minimal valid MF4 file structure in memory.
fn create_minimal_mf4() -> Vec<u8> {
    let mut data = Vec::new();

    // === ID Block (offset 0, 64 bytes) ===
    let id_block = create_id_block(420); // Version 4.20
    data.extend_from_slice(&id_block);

    // === HD Block (offset 64) ===
    let _hd_offset = 64u64;
    let dg_offset = 168u64; // HD is 104 bytes, so DG starts at 64+104=168
    let hd_block = create_hd_block(dg_offset);
    data.extend_from_slice(&hd_block);

    // === DG Block (offset 168) ===
    let cg_offset = 232u64; // DG is 64 bytes, so CG starts at 168+64=232
                            // Metadata occupies 0..496: ID 64 + HD 104 + DG 64 + CG 104 + CN 160.
                            // This previously said 400, which pointed into the middle of the CN block.
    let dt_offset = 496u64;
    let dg_block = create_dg_block(0, cg_offset, dt_offset);
    data.extend_from_slice(&dg_block);

    // === CG Block (offset 232) ===
    let cn_offset = 336u64; // CG is ~104 bytes, CN starts after
    let cg_block = create_cg_block(0, cn_offset, 5); // 5 samples
    data.extend_from_slice(&cg_block);

    // === CN Block (offset 336) ===
    let cn_block = create_cn_block(0); // Fixed-length float channel
    data.extend_from_slice(&cn_block);

    // Pad to data block offset
    while data.len() < dt_offset as usize {
        data.push(0);
    }

    // === DT Block (offset 400) ===
    // 5 samples of f64 data
    let sample_data: [f64; 5] = [1.0, 2.0, 3.0, 4.0, 5.0];
    let dt_block = create_dt_block(&sample_data);
    data.extend_from_slice(&dt_block);

    data
}

fn create_id_block(version: u16) -> Vec<u8> {
    let mut block = vec![0u8; 64];
    block[0..8].copy_from_slice(b"MDF     ");
    block[8..16].copy_from_slice(b"4.20    ");
    block[16..24].copy_from_slice(b"TestProg");
    block[28..30].copy_from_slice(&version.to_le_bytes());
    block
}

fn create_hd_block(dg_first: u64) -> Vec<u8> {
    let mut block = vec![0u8; 104];

    // Header
    block[0..4].copy_from_slice(b"##HD");
    block[8..16].copy_from_slice(&104u64.to_le_bytes()); // length
    block[16..24].copy_from_slice(&6u64.to_le_bytes()); // link_count

    // Links (6 x 8 bytes starting at offset 24)
    block[24..32].copy_from_slice(&dg_first.to_le_bytes()); // dg_first
                                                            // Rest of links are 0

    // Data section (starting at offset 72)
    let start_time_ns: i64 = 1_640_000_000_000_000_000;
    block[72..80].copy_from_slice(&start_time_ns.to_le_bytes());
    // tz_offset, dst_offset, flags etc are 0

    block
}

fn create_dg_block(dg_next: u64, cg_first: u64, data_link: u64) -> Vec<u8> {
    let mut block = vec![0u8; 64];

    block[0..4].copy_from_slice(b"##DG");
    block[8..16].copy_from_slice(&64u64.to_le_bytes());
    block[16..24].copy_from_slice(&4u64.to_le_bytes()); // link_count

    block[24..32].copy_from_slice(&dg_next.to_le_bytes());
    block[32..40].copy_from_slice(&cg_first.to_le_bytes());
    block[40..48].copy_from_slice(&data_link.to_le_bytes());
    // md_comment = 0

    // rec_id_size = 0 (only one CG)
    block[56] = 0;

    block
}

fn create_cg_block(cg_next: u64, cn_first: u64, cycle_count: u64) -> Vec<u8> {
    let mut block = vec![0u8; 104];

    block[0..4].copy_from_slice(b"##CG");
    block[8..16].copy_from_slice(&104u64.to_le_bytes());
    block[16..24].copy_from_slice(&6u64.to_le_bytes()); // link_count

    block[24..32].copy_from_slice(&cg_next.to_le_bytes());
    block[32..40].copy_from_slice(&cn_first.to_le_bytes());
    // Other links are 0

    // Data section (offset 72)
    block[72..80].copy_from_slice(&0u64.to_le_bytes()); // record_id
    block[80..88].copy_from_slice(&cycle_count.to_le_bytes()); // cycle_count
    block[88..90].copy_from_slice(&0u16.to_le_bytes()); // flags
    block[90..92].copy_from_slice(&('.' as u16).to_le_bytes()); // path_separator
                                                                // reserved: 4 bytes
    block[96..100].copy_from_slice(&8u32.to_le_bytes()); // data_bytes (8 for f64)
    block[100..104].copy_from_slice(&0u32.to_le_bytes()); // inval_bytes

    block
}

fn create_cn_block(cn_next: u64) -> Vec<u8> {
    let mut block = vec![0u8; 160];

    block[0..4].copy_from_slice(b"##CN");
    block[8..16].copy_from_slice(&160u64.to_le_bytes());
    block[16..24].copy_from_slice(&8u64.to_le_bytes()); // link_count

    block[24..32].copy_from_slice(&cn_next.to_le_bytes());
    // Other links are 0 (no name, unit, conversion)

    // Data section (offset 88)
    block[88] = 0; // channel_type = FixedLength
    block[89] = 0; // sync_type = None
    block[90] = 4; // data_type = FloatLe
    block[91] = 0; // bit_offset
    block[92..96].copy_from_slice(&0u32.to_le_bytes()); // byte_offset
    block[96..100].copy_from_slice(&64u32.to_le_bytes()); // bit_count (64 = f64)
    block[100..104].copy_from_slice(&0u32.to_le_bytes()); // flags
                                                          // Rest is zeros for limits, precision, etc.

    block
}

fn create_dt_block(samples: &[f64]) -> Vec<u8> {
    let data_size = samples.len() * 8;
    let total_size = BLOCK_HEADER_SIZE + data_size;
    let mut block = vec![0u8; total_size];

    block[0..4].copy_from_slice(b"##DT");
    block[8..16].copy_from_slice(&(total_size as u64).to_le_bytes());
    block[16..24].copy_from_slice(&0u64.to_le_bytes()); // link_count

    // Data section
    for (i, &sample) in samples.iter().enumerate() {
        let offset = BLOCK_HEADER_SIZE + i * 8;
        block[offset..offset + 8].copy_from_slice(&sample.to_le_bytes());
    }

    block
}

#[test]
fn test_version_parsing() {
    assert!(matches!(
        Mf4Version::from_version_number(400),
        Mf4Version::V4_0 { .. }
    ));
    assert!(matches!(
        Mf4Version::from_version_number(410),
        Mf4Version::V4_1 { .. }
    ));
    assert!(matches!(
        Mf4Version::from_version_number(420),
        Mf4Version::V4_2 { .. }
    ));

    let unknown = Mf4Version::from_version_number(500);
    assert!(!unknown.is_supported());
}

#[test]
fn test_id_block_parsing() {
    let data = create_id_block(420);
    let id = IdBlock::parse(&data).unwrap();

    assert_eq!(id.version_major(), 4);
    assert_eq!(id.version_minor(), 20);
    assert!(id.is_finalized());
}

#[test]
fn test_hd_block_parsing() {
    let data = create_hd_block(168);
    let hd = HdBlock::parse(&data, 64).unwrap();

    assert_eq!(hd.dg_first, 168);
    assert_eq!(hd.fh_first, 0);
}

#[test]
fn test_dg_block_parsing() {
    let data = create_dg_block(0, 200, 300);
    let dg = DgBlock::parse(&data, 168).unwrap();

    assert_eq!(dg.dg_next, 0);
    assert_eq!(dg.cg_first, 200);
    assert_eq!(dg.data, 300);
    assert_eq!(dg.rec_id_size, 0);
}

#[test]
fn test_cg_block_parsing() {
    let data = create_cg_block(0, 500, 1000);
    let cg = CgBlock::parse(&data, 232).unwrap();

    assert_eq!(cg.cg_next, 0);
    assert_eq!(cg.cn_first, 500);
    assert_eq!(cg.cycle_count, 1000);
    assert_eq!(cg.data_bytes, 8);
}

#[test]
fn test_cn_block_parsing() {
    let data = create_cn_block(0);
    let cn = CnBlock::parse(&data, 336).unwrap();

    assert_eq!(cn.cn_next, 0);
    assert_eq!(cn.channel_type, ChannelType::FixedLength);
    assert_eq!(cn.data_type, DataType::FloatLe);
    assert_eq!(cn.bit_count, 64);
}

#[test]
fn test_dt_block_parsing() {
    let samples = [1.0f64, 2.0, 3.0];
    let data = create_dt_block(&samples);
    let dt = DtBlock::parse(&data, 0).unwrap();

    assert_eq!(dt.data_length, 24); // 3 * 8 bytes
}

#[test]
fn test_conversion_identity() {
    let conv = Conversion::None;
    assert_eq!(conv.convert(42.0, false), 42.0);
}

#[test]
fn test_conversion_linear() {
    let conv = Conversion::Linear {
        offset: 10.0,
        factor: 2.0,
    };
    assert!((conv.convert(5.0, false) - 20.0).abs() < 0.001);
}

#[test]
fn test_mf4_file_structure() {
    // Create a minimal MF4 file and write to temp file
    let mf4_data = create_minimal_mf4();

    let mut temp_file = NamedTempFile::new().unwrap();
    temp_file.write_all(&mf4_data).unwrap();
    temp_file.flush().unwrap();

    // Try to open it with our library
    // Note: This will fail because our minimal file doesn't have proper TX blocks
    // for channel names, but it tests the basic parsing infrastructure
    let result = falcon_mdf::Mf4File::open(temp_file.path());

    // We expect this to succeed (channel name will be empty but parsing should work)
    assert!(
        result.is_ok(),
        "Failed to open test MF4: {:?}",
        result.err()
    );

    let file = result.unwrap();
    assert!(matches!(file.version(), Mf4Version::V4_2 { .. }));
    assert_eq!(file.data_group_count(), 1);
}
