//! Disk-provided link counts must be bounded before allocation or arithmetic.
use falcon_mdf::blocks::common::{read_link, read_links, BlockHeader, ParseBlock};
use falcon_mdf::blocks::ChBlock;

#[test]
fn overflowing_link_offsets_return_errors() {
    for offset in [usize::MAX, usize::MAX - 7, 9] {
        assert!(read_link(&[0; 16], offset).is_err());
    }
}

#[test]
fn excessive_link_counts_return_errors_before_allocating() {
    assert!(read_links(&[0; 16], 0, usize::MAX).is_err());
    assert!(read_links(&[0; 16], usize::MAX, 1).is_err());
    assert!(read_links(&[0; 16], 8, 2).is_err());
}

#[test]
fn exact_end_and_empty_link_tables_are_readable() {
    let bytes = [1u64.to_le_bytes(), u64::MAX.to_le_bytes()].concat();
    assert_eq!(read_link(&bytes, 8).unwrap(), u64::MAX);
    assert_eq!(read_links(&bytes, 0, 2).unwrap(), vec![1, u64::MAX]);
    assert!(read_links(&bytes, bytes.len(), 0).unwrap().is_empty());
}

#[test]
fn header_rejects_overflow_of_links_plus_header() {
    let mut bytes = [0; 24];
    bytes[..4].copy_from_slice(b"##CH");
    bytes[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
    bytes[16..24].copy_from_slice(&(u64::MAX / 8).to_le_bytes());
    assert!(BlockHeader::parse(&bytes, 64).is_err());
}

#[test]
fn truncated_hierarchy_cannot_allocate_its_declared_link_table() {
    let mut bytes = [0; 24];
    bytes[..4].copy_from_slice(b"##CH");
    bytes[8..16].copy_from_slice(&(u64::MAX - 7).to_le_bytes());
    bytes[16..24].copy_from_slice(&((u64::MAX - 31) / 8).to_le_bytes());
    assert!(ChBlock::parse(&bytes, 64).is_err());
}
