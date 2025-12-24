//! MF4 version handling and negotiation.
//!
//! This module provides version detection and dispatch for different
//! MF4 format versions. The design allows for adding support for
//! additional versions without changing existing code.

use crate::error::{Mf4Error, Result};
use crate::blocks::IdBlock;
use std::fmt;

/// MF4 version identifier.
///
/// This enum represents known MF4 versions with room for unknown
/// versions that might be encountered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Mf4Version {
    /// MF4 version 4.0 (initial MF4 release).
    V4_0 { raw: u16 },
    /// MF4 version 4.1 (added sample reduction, etc.).
    V4_1 { raw: u16 },
    /// MF4 version 4.2 (current standard version).
    V4_2 { raw: u16 },
    /// Unknown version with major.minor numbers.
    Unknown { major: u16, minor: u16, raw: u16 },
}

impl Mf4Version {
    /// Creates an Mf4Version from major and minor version numbers.
    ///
    /// # Arguments
    /// * `major` - Major version number (typically 4)
    /// * `minor` - Minor version number (0, 1, 2, etc.)
    ///
    /// # Example
    /// ```
    /// use falcon_mdf::parser::Mf4Version;
    ///
    /// let v = Mf4Version::from_parts(4, 2);
    /// assert!(matches!(v, Mf4Version::V4_2 { .. }));
    /// ```
    pub fn from_parts(major: u16, minor: u16) -> Self {
        let raw = major * 100 + minor;
        match (major, minor) {
            (4, 0) => Mf4Version::V4_0 { raw },
            (4, 1) => Mf4Version::V4_1 { raw },
            (4, 10..=19) => Mf4Version::V4_1 { raw }, // 4.10-4.19 are 4.1.x
            (4, 2) => Mf4Version::V4_2 { raw },
            (4, 20..=29) => Mf4Version::V4_2 { raw }, // 4.20-4.29 are 4.2.x
            _ => Mf4Version::Unknown { major, minor, raw },
        }
    }

    /// Creates an Mf4Version from the combined version number stored in ID block.
    ///
    /// The version number is stored as major * 100 + minor (e.g., 420 for v4.20).
    pub fn from_version_number(version: u16) -> Self {
        let major = version / 100;
        let minor = version % 100;
        Self::from_parts(major, minor)
    }

    /// Creates an Mf4Version from an ID block.
    pub fn from_id_block(id: &IdBlock) -> Self {
        Self::from_version_number(id.version_number)
    }

    /// Returns the major version number.
    pub fn major(&self) -> u16 {
        match self {
            Mf4Version::V4_0 { .. } => 4,
            Mf4Version::V4_1 { .. } => 4,
            Mf4Version::V4_2 { .. } => 4,
            Mf4Version::Unknown { major, .. } => *major,
        }
    }

    /// Returns the minor version number (raw, e.g., 11 for version 4.11).
    pub fn minor(&self) -> u16 {
        match self {
            Mf4Version::V4_0 { raw } => raw % 100,
            Mf4Version::V4_1 { raw } => raw % 100,
            Mf4Version::V4_2 { raw } => raw % 100,
            Mf4Version::Unknown { minor, .. } => *minor,
        }
    }

    /// Returns the raw version number (e.g., 411 for version 4.11).
    pub fn raw(&self) -> u16 {
        match self {
            Mf4Version::V4_0 { raw } => *raw,
            Mf4Version::V4_1 { raw } => *raw,
            Mf4Version::V4_2 { raw } => *raw,
            Mf4Version::Unknown { raw, .. } => *raw,
        }
    }

    /// Returns true if this version is supported by this library.
    pub fn is_supported(&self) -> bool {
        matches!(self, Mf4Version::V4_0 { .. } | Mf4Version::V4_1 { .. } | Mf4Version::V4_2 { .. })
    }

    /// Validates that this version is supported, returning an error if not.
    pub fn validate(&self) -> Result<()> {
        if self.is_supported() {
            Ok(())
        } else {
            Err(Mf4Error::UnsupportedVersion {
                major: self.major(),
                minor: self.minor(),
            })
        }
    }
}

impl fmt::Display for Mf4Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major(), self.minor())
    }
}

impl Default for Mf4Version {
    fn default() -> Self {
        Mf4Version::V4_2 { raw: 420 }
    }
}

/// Trait for version-specific parsing behavior.
///
/// Implement this trait to add support for version-specific
/// parsing variations. The default implementations handle
/// the common case for MF4 4.0-4.2.
pub trait VersionedParser {
    /// Returns the version this parser handles.
    fn version(&self) -> Mf4Version;

    /// Returns true if this parser can handle the given version.
    fn can_handle(&self, version: Mf4Version) -> bool {
        self.version() == version
    }

    /// Returns the offset where the HD block starts.
    /// This is always 64 (after the ID block) for all known versions.
    fn hd_block_offset(&self) -> u64 {
        64
    }

    /// Returns whether this version supports sample reduction blocks.
    fn supports_sample_reduction(&self) -> bool {
        matches!(self.version(), Mf4Version::V4_1 { .. } | Mf4Version::V4_2 { .. })
    }

    /// Returns whether this version supports events.
    fn supports_events(&self) -> bool {
        true // Supported in all MF4 versions
    }

    /// Returns whether this version supports attachments.
    fn supports_attachments(&self) -> bool {
        true // Supported in all MF4 versions
    }
}

/// Parser for MF4 4.0 files.
#[derive(Debug, Clone, Copy, Default)]
pub struct V40Parser;

impl VersionedParser for V40Parser {
    fn version(&self) -> Mf4Version {
        Mf4Version::V4_0 { raw: 400 }
    }

    fn supports_sample_reduction(&self) -> bool {
        false
    }
}

/// Parser for MF4 4.1 files.
#[derive(Debug, Clone, Copy, Default)]
pub struct V41Parser;

impl VersionedParser for V41Parser {
    fn version(&self) -> Mf4Version {
        Mf4Version::V4_1 { raw: 410 }
    }
}

/// Parser for MF4 4.2 files.
#[derive(Debug, Clone, Copy, Default)]
pub struct V42Parser;

impl VersionedParser for V42Parser {
    fn version(&self) -> Mf4Version {
        Mf4Version::V4_2 { raw: 420 }
    }
}

/// Gets a versioned parser for the given version.
pub fn get_parser_for_version(version: Mf4Version) -> Box<dyn VersionedParser + Send + Sync> {
    match version {
        Mf4Version::V4_0 { .. } => Box::new(V40Parser),
        Mf4Version::V4_1 { .. } => Box::new(V41Parser),
        Mf4Version::V4_2 { .. } => Box::new(V42Parser),
        Mf4Version::Unknown { .. } => {
            // Default to 4.2 parser for unknown versions
            // (forward compatibility attempt)
            Box::new(V42Parser)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_from_parts() {
        assert!(matches!(Mf4Version::from_parts(4, 0), Mf4Version::V4_0 { raw: 400 }));
        assert!(matches!(Mf4Version::from_parts(4, 1), Mf4Version::V4_1 { raw: 401 }));
        assert!(matches!(Mf4Version::from_parts(4, 2), Mf4Version::V4_2 { raw: 402 }));
        assert!(matches!(Mf4Version::from_parts(4, 10), Mf4Version::V4_1 { raw: 410 }));
        assert!(matches!(Mf4Version::from_parts(4, 11), Mf4Version::V4_1 { raw: 411 }));
        assert!(matches!(Mf4Version::from_parts(4, 20), Mf4Version::V4_2 { raw: 420 }));
        assert!(matches!(
            Mf4Version::from_parts(5, 0),
            Mf4Version::Unknown { major: 5, minor: 0, raw: 500 }
        ));
    }

    #[test]
    fn test_version_from_version_number() {
        assert!(matches!(Mf4Version::from_version_number(400), Mf4Version::V4_0 { .. }));
        assert!(matches!(Mf4Version::from_version_number(410), Mf4Version::V4_1 { .. }));
        assert!(matches!(Mf4Version::from_version_number(411), Mf4Version::V4_1 { raw: 411 }));
        assert!(matches!(Mf4Version::from_version_number(420), Mf4Version::V4_2 { .. }));
    }

    #[test]
    fn test_version_display() {
        assert_eq!(format!("{}", Mf4Version::from_version_number(400)), "4.0");
        assert_eq!(format!("{}", Mf4Version::from_version_number(411)), "4.11");
        assert_eq!(format!("{}", Mf4Version::from_version_number(420)), "4.20");
    }

    #[test]
    fn test_version_is_supported() {
        assert!(Mf4Version::V4_0 { raw: 400 }.is_supported());
        assert!(Mf4Version::V4_1 { raw: 411 }.is_supported());
        assert!(Mf4Version::V4_2 { raw: 420 }.is_supported());
        assert!(!Mf4Version::Unknown { major: 5, minor: 0, raw: 500 }.is_supported());
    }

    #[test]
    fn test_version_validate() {
        assert!(Mf4Version::V4_2 { raw: 420 }.validate().is_ok());
        assert!(Mf4Version::Unknown { major: 5, minor: 0, raw: 500 }.validate().is_err());
    }

    #[test]
    fn test_versioned_parser() {
        let parser = get_parser_for_version(Mf4Version::V4_0 { raw: 400 });
        assert!(!parser.supports_sample_reduction());

        let parser = get_parser_for_version(Mf4Version::V4_2 { raw: 420 });
        assert!(parser.supports_sample_reduction());
    }
}
