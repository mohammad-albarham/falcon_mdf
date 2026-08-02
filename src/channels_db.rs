//! Channel database for efficient channel lookup.
//!
//! This module provides `ChannelsDB`, a lookup structure inspired by asammdf's
//! `channels_db` that enables O(1) channel lookup by name.
//!
//! ## Why This Matters
//!
//! MDF4 files can contain thousands of channels. Without an index, finding a
//! channel by name requires iterating through all data groups and channel groups,
//! which is O(n) per lookup. With `ChannelsDB`, lookups are O(1) on average.
//!
//! ## Design
//!
//! - Channels are indexed by name
//! - Multiple channels can have the same name (common in bus logging)
//! - Each entry stores `(data_group_index, channel_group_index, channel_index)`

use std::collections::HashMap;

/// Location of a channel within the file structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelLocation {
    /// Index of the data group containing this channel.
    pub data_group_index: usize,
    /// Index of the channel group within the data group.
    pub channel_group_index: usize,
    /// Index of the channel within the channel group.
    pub channel_index: usize,
}

impl ChannelLocation {
    /// Creates a new channel location.
    pub fn new(data_group_index: usize, channel_group_index: usize, channel_index: usize) -> Self {
        Self {
            data_group_index,
            channel_group_index,
            channel_index,
        }
    }
}

/// A database for fast channel lookup by name.
///
/// This structure provides O(1) average-case lookup of channels by name,
/// similar to asammdf's `channels_db`. It handles the common case where
/// multiple channels share the same name (e.g., "EngineSpeed" appearing
/// in multiple CAN message groups).
///
/// # Example
///
/// ```ignore
/// let mut db = ChannelsDB::new();
///
/// // Index channels during parsing
/// db.insert("EngineSpeed", ChannelLocation::new(0, 0, 1));
/// db.insert("EngineSpeed", ChannelLocation::new(1, 0, 2)); // Same name, different group
/// db.insert("VehicleSpeed", ChannelLocation::new(0, 0, 3));
///
/// // Fast lookup
/// assert_eq!(db.find_first("VehicleSpeed"), Some(&ChannelLocation::new(0, 0, 3)));
///
/// // Find all channels with same name
/// let engine_speeds = db.find_all("EngineSpeed");
/// assert_eq!(engine_speeds.len(), 2);
/// ```
#[derive(Debug, Clone, Default)]
pub struct ChannelsDB {
    /// Map from channel name to list of locations.
    /// Using Vec for locations since same-name channels are common but few.
    db: HashMap<String, Vec<ChannelLocation>>,

    /// Total number of channels indexed.
    total_channels: usize,
}

impl ChannelsDB {
    /// Creates a new empty channel database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a channel database with pre-allocated capacity.
    ///
    /// # Arguments
    /// * `unique_names` - Expected number of unique channel names
    pub fn with_capacity(unique_names: usize) -> Self {
        Self {
            db: HashMap::with_capacity(unique_names),
            total_channels: 0,
        }
    }

    /// Inserts a channel into the database.
    ///
    /// If a channel with the same name already exists, the new location
    /// is added to the list of locations for that name.
    pub fn insert(&mut self, name: impl Into<String>, location: ChannelLocation) {
        let name = name.into();
        self.db.entry(name).or_default().push(location);
        self.total_channels += 1;
    }

    /// Finds the first channel with the given name.
    ///
    /// Returns `None` if no channel with that name exists.
    /// This is the fastest lookup for the common case where you just
    /// need any channel with a given name.
    pub fn find_first(&self, name: &str) -> Option<&ChannelLocation> {
        self.db.get(name).and_then(|locs| locs.first())
    }

    /// Finds all channels with the given name.
    ///
    /// Returns an empty slice if no channels match.
    pub fn find_all(&self, name: &str) -> &[ChannelLocation] {
        self.db.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Returns true if a channel with the given name exists.
    pub fn contains(&self, name: &str) -> bool {
        self.db.contains_key(name)
    }

    /// Returns true if the index holds no channels.
    ///
    /// True when indexing was switched off at open, which lookups use to decide
    /// whether they can consult the index or must scan the groups directly.
    pub fn is_empty(&self) -> bool {
        self.db.is_empty()
    }

    /// Returns the number of unique channel names.
    pub fn unique_name_count(&self) -> usize {
        self.db.len()
    }

    /// Returns the total number of channels indexed.
    pub fn total_channel_count(&self) -> usize {
        self.total_channels
    }

    /// Returns an iterator over all unique channel names.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.db.keys().map(String::as_str)
    }

    /// Returns an iterator over all (name, locations) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[ChannelLocation])> {
        self.db.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    /// Clears the database.
    pub fn clear(&mut self) {
        self.db.clear();
        self.total_channels = 0;
    }
}

/// Index of master (time/angle/distance) channels per channel group.
///
/// This provides O(1) lookup of the master channel for any channel group,
/// which is frequently needed when constructing time series data.
#[derive(Debug, Clone, Default)]
pub struct MastersDB {
    /// Map from (data_group_index, channel_group_index) to master channel index.
    db: HashMap<(usize, usize), usize>,
}

impl MastersDB {
    /// Creates a new empty masters database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a master channel reference.
    pub fn insert(
        &mut self,
        data_group_index: usize,
        channel_group_index: usize,
        channel_index: usize,
    ) {
        self.db
            .insert((data_group_index, channel_group_index), channel_index);
    }

    /// Finds the master channel index for a channel group.
    pub fn find(&self, data_group_index: usize, channel_group_index: usize) -> Option<usize> {
        self.db
            .get(&(data_group_index, channel_group_index))
            .copied()
    }

    /// Returns the number of channel groups with identified masters.
    pub fn len(&self) -> usize {
        self.db.len()
    }

    /// Returns true if no masters have been identified.
    pub fn is_empty(&self) -> bool {
        self.db.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channels_db_basic() {
        let mut db = ChannelsDB::new();

        db.insert("Speed", ChannelLocation::new(0, 0, 0));
        db.insert("RPM", ChannelLocation::new(0, 0, 1));

        assert_eq!(db.find_first("Speed"), Some(&ChannelLocation::new(0, 0, 0)));
        assert_eq!(db.find_first("RPM"), Some(&ChannelLocation::new(0, 0, 1)));
        assert_eq!(db.find_first("Unknown"), None);
    }

    #[test]
    fn test_channels_db_duplicate_names() {
        let mut db = ChannelsDB::new();

        db.insert("Speed", ChannelLocation::new(0, 0, 0));
        db.insert("Speed", ChannelLocation::new(1, 0, 0));
        db.insert("Speed", ChannelLocation::new(2, 0, 0));

        assert_eq!(db.unique_name_count(), 1);
        assert_eq!(db.total_channel_count(), 3);

        let all = db.find_all("Speed");
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_channels_db_iteration() {
        let mut db = ChannelsDB::new();

        db.insert("A", ChannelLocation::new(0, 0, 0));
        db.insert("B", ChannelLocation::new(0, 0, 1));
        db.insert("C", ChannelLocation::new(0, 0, 2));

        let names: Vec<&str> = db.names().collect();
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn test_masters_db() {
        let mut db = MastersDB::new();

        db.insert(0, 0, 0); // DG0, CG0 has master at index 0
        db.insert(0, 1, 2); // DG0, CG1 has master at index 2
        db.insert(1, 0, 0); // DG1, CG0 has master at index 0

        assert_eq!(db.find(0, 0), Some(0));
        assert_eq!(db.find(0, 1), Some(2));
        assert_eq!(db.find(1, 0), Some(0));
        assert_eq!(db.find(2, 0), None);
    }
}
