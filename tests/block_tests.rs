//! Unit tests for block header and common parsing functionality.

use falcon_mdf::blocks::{block_ids, BlockHeader, BLOCK_HEADER_SIZE};

#[test]
fn test_block_header_size() {
    // MDF4 block headers are always 24 bytes
    assert_eq!(BLOCK_HEADER_SIZE, 24);
}

#[test]
fn test_block_ids() {
    assert_eq!(*block_ids::ID, *b"MDF ");
    assert_eq!(*block_ids::HD, *b"##HD");
    assert_eq!(*block_ids::DG, *b"##DG");
    assert_eq!(*block_ids::CG, *b"##CG");
    assert_eq!(*block_ids::CN, *b"##CN");
    assert_eq!(*block_ids::DT, *b"##DT");
    assert_eq!(*block_ids::DZ, *b"##DZ");
    assert_eq!(*block_ids::DL, *b"##DL");
    assert_eq!(*block_ids::HL, *b"##HL");
    assert_eq!(*block_ids::TX, *b"##TX");
    assert_eq!(*block_ids::MD, *b"##MD");
    assert_eq!(*block_ids::CC, *b"##CC");
    assert_eq!(*block_ids::SI, *b"##SI");
    assert_eq!(*block_ids::FH, *b"##FH");
}

#[test]
fn test_block_header_parsing() {
    let mut data = [0u8; 24];
    data[0..4].copy_from_slice(b"##HD");
    data[8..16].copy_from_slice(&104u64.to_le_bytes());
    data[16..24].copy_from_slice(&6u64.to_le_bytes());

    let header = BlockHeader::parse(&data, 0).unwrap();

    assert_eq!(&header.block_type, b"##HD");
    assert_eq!(header.length, 104);
    assert_eq!(header.link_count, 6);
    assert_eq!(header.offset, 0);
}

#[test]
fn test_block_header_invalid_id() {
    let mut data = [0u8; 24];
    data[0..4].copy_from_slice(b"XXXX"); // Invalid ID
    data[8..16].copy_from_slice(&64u64.to_le_bytes());
    data[16..24].copy_from_slice(&0u64.to_le_bytes());

    // Should still parse (ID validation happens at higher level)
    let header = BlockHeader::parse(&data, 100).unwrap();
    assert_eq!(&header.block_type, b"XXXX");
    assert_eq!(header.offset, 100);
}

#[test]
fn test_block_header_data_offset() {
    // The data offset should be: header size (24) + link_count * 8
    let header = BlockHeader {
        block_type: *b"##CN",
        reserved: [0; 4],
        length: 160,
        link_count: 8,
        offset: 0,
    };

    let expected_data_offset = BLOCK_HEADER_SIZE + (8 * 8);
    assert_eq!(header.data_offset(), expected_data_offset);
}

#[test]
fn test_block_header_too_short() {
    let data = [0u8; 16]; // Too short (need 24 bytes)
    let result = BlockHeader::parse(&data, 0);
    assert!(result.is_err());
}
