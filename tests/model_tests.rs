//! Tests for the model layer.
//!
//! Note: Signal is now primarily an internal type with `pub(crate)` constructor.
//! These tests focus on the public interfaces exposed through the library.

use falcon_mdf::{ChannelLocation, ChannelsDB, MastersDB};

#[test]
fn test_channels_db_empty() {
    let db = ChannelsDB::new();
    assert!(db.find_first("nonexistent").is_none());
    assert!(db.find_all("nonexistent").is_empty());
    assert_eq!(db.unique_name_count(), 0);
    assert_eq!(db.total_channel_count(), 0);
}

#[test]
fn test_channels_db_insert_and_find() {
    let mut db = ChannelsDB::new();

    // Insert some channel locations
    db.insert("temperature", ChannelLocation::new(0, 0, 0));
    db.insert("pressure", ChannelLocation::new(0, 0, 1));
    db.insert("temperature", ChannelLocation::new(0, 1, 0)); // Duplicate name in different group

    // Find first should return the first inserted location
    let first = db.find_first("temperature");
    assert!(first.is_some());
    let loc = first.unwrap();
    assert_eq!(loc.data_group_index, 0);
    assert_eq!(loc.channel_group_index, 0);
    assert_eq!(loc.channel_index, 0);

    // Find all should return both locations
    let all = db.find_all("temperature");
    assert_eq!(all.len(), 2);

    // Single channel should return only one
    let pressure_all = db.find_all("pressure");
    assert_eq!(pressure_all.len(), 1);

    // Total counts
    assert_eq!(db.unique_name_count(), 2);
    assert_eq!(db.total_channel_count(), 3);
}

#[test]
fn test_channels_db_names() {
    let mut db = ChannelsDB::new();

    db.insert("alpha", ChannelLocation::new(0, 0, 0));
    db.insert("beta", ChannelLocation::new(0, 0, 1));
    db.insert("gamma", ChannelLocation::new(0, 0, 2));

    // Collect names into a Vec for easier testing
    let names: Vec<&str> = db.names().collect();
    assert_eq!(names.len(), 3);
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
    assert!(names.contains(&"gamma"));
}

#[test]
fn test_channels_db_contains() {
    let mut db = ChannelsDB::new();

    db.insert("voltage", ChannelLocation::new(0, 0, 0));

    assert!(db.contains("voltage"));
    assert!(!db.contains("current"));
}

#[test]
fn test_channel_location_equality() {
    let loc1 = ChannelLocation::new(1, 2, 3);
    let loc2 = ChannelLocation::new(1, 2, 3);
    let loc3 = ChannelLocation::new(1, 2, 4);

    assert_eq!(loc1, loc2);
    assert_ne!(loc1, loc3);
}

#[test]
fn test_masters_db_empty() {
    let db = MastersDB::new();
    assert!(db.is_empty());
    assert_eq!(db.len(), 0);
    assert!(db.find(0, 0).is_none());
}

#[test]
fn test_masters_db_insert_and_find() {
    let mut db = MastersDB::new();

    // Set master channels for different groups
    db.insert(0, 0, 0); // DG 0, CG 0, master is channel 0
    db.insert(0, 1, 2); // DG 0, CG 1, master is channel 2
    db.insert(1, 0, 1); // DG 1, CG 0, master is channel 1

    // Retrieve master channels
    assert_eq!(db.find(0, 0), Some(0));
    assert_eq!(db.find(0, 1), Some(2));
    assert_eq!(db.find(1, 0), Some(1));
    assert!(db.find(2, 0).is_none());

    assert!(!db.is_empty());
    assert_eq!(db.len(), 3);
}

/// Helper function to compute byte size from bit count (same logic as Channel::byte_size)
fn compute_byte_size(bit_count: u32) -> usize {
    bit_count.div_ceil(8) as usize
}

#[test]
fn test_byte_size_calculation() {
    // 8 bits should be 1 byte
    assert_eq!(compute_byte_size(8), 1);

    // 1-8 bits should all be 1 byte
    assert_eq!(compute_byte_size(1), 1);

    // 9-16 bits should be 2 bytes
    assert_eq!(compute_byte_size(9), 2);
    assert_eq!(compute_byte_size(16), 2);

    // 17-24 bits should be 3 bytes
    assert_eq!(compute_byte_size(17), 3);
    assert_eq!(compute_byte_size(24), 3);

    // 32 bits should be 4 bytes
    assert_eq!(compute_byte_size(32), 4);

    // 64 bits should be 8 bytes
    assert_eq!(compute_byte_size(64), 8);
}
