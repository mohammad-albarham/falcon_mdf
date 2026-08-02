//! Error types and Result aliases for the falcon_mdf crate.
//!
//! This module provides comprehensive error handling for all operations
//! that can fail when reading MF4 files, including I/O errors, parsing
//! errors, and version compatibility issues.

use std::io;
use thiserror::Error;

/// The main error type for all falcon_mdf operations.
#[derive(Error, Debug)]
pub enum Mf4Error {
    /// An I/O error occurred while reading the file.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// The file does not have a valid MF4 signature.
    #[error("Invalid MF4 file signature: expected 'MDF     ' or 'UnFinMF ', got '{0}'")]
    InvalidSignature(String),

    /// The MF4 version is not supported.
    #[error("Unsupported MF4 version: {major}.{minor}")]
    UnsupportedVersion {
        /// Major version number parsed from the ID block.
        major: u16,
        /// Minor version number parsed from the ID block.
        minor: u16,
    },

    /// A required block is missing from the file.
    #[error("Missing required block: {block_type} at offset {offset:#x}")]
    MissingBlock {
        /// Four-character block identifier that was expected, e.g. `##HD`.
        block_type: String,
        /// File offset at which the block was expected.
        offset: u64,
    },

    /// A block has an invalid size.
    #[error("Invalid block size: {block_type} has size {size}, expected at least {min_size}")]
    InvalidBlockSize {
        /// Four-character block identifier, e.g. `##CN`.
        block_type: String,
        /// Size declared in the block header.
        size: u64,
        /// Minimum size required for this block type.
        min_size: u64,
    },

    /// A block has an invalid identifier.
    #[error(
        "Invalid block identifier at offset {offset:#x}: expected '{expected}', got '{actual}'"
    )]
    InvalidBlockId {
        /// File offset of the block header.
        offset: u64,
        /// Block identifier that was expected.
        expected: String,
        /// Block identifier actually found.
        actual: String,
    },

    /// The file is truncated or corrupted.
    #[error("File is truncated: expected {expected} bytes at offset {offset:#x}, got {actual}")]
    TruncatedFile {
        /// File offset at which the read was attempted.
        offset: u64,
        /// Number of bytes required.
        expected: usize,
        /// Number of bytes actually available.
        actual: usize,
    },

    /// A link points to an invalid location.
    #[error("Invalid link at offset {offset:#x}: points to {target:#x}")]
    InvalidLink {
        /// File offset of the link field itself.
        offset: u64,
        /// Target offset the link points to.
        target: u64,
    },

    /// Channel not found.
    #[error("Channel not found: '{name}'")]
    ChannelNotFound {
        /// Name that was looked up.
        name: String,
    },

    /// Data type conversion error.
    #[error("Data type conversion error: {message}")]
    DataTypeConversion {
        /// Description of what could not be converted.
        message: String,
    },

    /// Compression error.
    #[error("Compression error: {0}")]
    Compression(String),

    /// Decompression error.
    #[error("Decompression error: {0}")]
    Decompression(String),

    /// Invalid data block format.
    #[error("Invalid data block format: {message}")]
    InvalidDataBlock {
        /// Description of the malformed data block.
        message: String,
    },

    /// Invalid channel conversion.
    #[error("Invalid channel conversion: {message}")]
    InvalidConversion {
        /// Description of the invalid conversion.
        message: String,
    },

    /// Memory mapping failed.
    #[error("Memory mapping failed: {0}")]
    MmapFailed(String),

    /// UTF-8 decoding error.
    #[error("UTF-8 decoding error: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),

    /// Generic parsing error.
    #[error("Parse error: {message}")]
    ParseError {
        /// Description of the parse failure.
        message: String,
    },

    /// A well-formed file uses a feature this version does not yet decode.
    ///
    /// Returned instead of a plausible-looking wrong answer. Reading a channel
    /// this crate cannot decode must fail loudly: measurement data that is
    /// quietly incorrect is worse than data that is missing.
    #[error("Unsupported feature: {feature} ({detail})")]
    Unsupported {
        /// The feature involved, e.g. `"variable-length signal data (VLSD)"`.
        feature: String,
        /// What was being read when it came up.
        detail: String,
    },
}

/// A specialized Result type for MF4 operations.
pub type Result<T> = std::result::Result<T, Mf4Error>;

impl Mf4Error {
    /// Creates a new ParseError with the given message.
    pub fn parse_error(message: impl Into<String>) -> Self {
        Mf4Error::ParseError {
            message: message.into(),
        }
    }

    /// Creates a new InvalidBlockSize error.
    pub fn invalid_block_size(block_type: impl Into<String>, size: u64, min_size: u64) -> Self {
        Mf4Error::InvalidBlockSize {
            block_type: block_type.into(),
            size,
            min_size,
        }
    }

    /// Creates a new MissingBlock error.
    pub fn missing_block(block_type: impl Into<String>, offset: u64) -> Self {
        Mf4Error::MissingBlock {
            block_type: block_type.into(),
            offset,
        }
    }

    /// Creates a new InvalidBlockId error.
    pub fn invalid_block_id(
        offset: u64,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Mf4Error::InvalidBlockId {
            offset,
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// Creates a new TruncatedFile error.
    pub fn truncated(offset: u64, expected: usize, actual: usize) -> Self {
        Mf4Error::TruncatedFile {
            offset,
            expected,
            actual,
        }
    }

    /// Creates a new DataTypeConversion error.
    pub fn data_conversion(message: impl Into<String>) -> Self {
        Mf4Error::DataTypeConversion {
            message: message.into(),
        }
    }

    /// Creates a new InvalidDataBlock error.
    pub fn invalid_data_block(message: impl Into<String>) -> Self {
        Mf4Error::InvalidDataBlock {
            message: message.into(),
        }
    }

    /// Creates a new InvalidConversion error.
    pub fn invalid_conversion(message: impl Into<String>) -> Self {
        Mf4Error::InvalidConversion {
            message: message.into(),
        }
    }

    /// Creates a new Unsupported error.
    pub fn unsupported(feature: impl Into<String>, detail: impl Into<String>) -> Self {
        Mf4Error::Unsupported {
            feature: feature.into(),
            detail: detail.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = Mf4Error::UnsupportedVersion { major: 4, minor: 3 };
        assert_eq!(err.to_string(), "Unsupported MF4 version: 4.3");

        let err = Mf4Error::InvalidSignature("BAD     ".to_string());
        assert!(err.to_string().contains("BAD     "));

        let err = Mf4Error::missing_block("HD", 0x40);
        assert!(err.to_string().contains("HD"));
    }

    #[test]
    fn test_error_construction() {
        let err = Mf4Error::parse_error("test error");
        assert!(matches!(err, Mf4Error::ParseError { .. }));

        let err = Mf4Error::invalid_block_size("DG", 100, 200);
        assert!(matches!(err, Mf4Error::InvalidBlockSize { .. }));
    }
}
