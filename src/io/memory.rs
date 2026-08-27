//! In-memory byte buffer I/O backend.
//!
//! This module provides in-memory access to MF4 file data stored in a `Vec<u8>`.
//! This is useful in environments without a filesystem (such as WebAssembly in a
//! browser) or when MF4 file bytes have already been loaded into memory.

use crate::error::{Mf4Error, Result};
use crate::io::{ByteSlice, ByteSource};

/// An in-memory byte source.
///
/// This struct wraps a `Vec<u8>` and provides safe access to its contents
/// as byte slices.
#[derive(Debug, Clone)]
pub struct MemorySource {
    /// The underlying byte buffer.
    data: Vec<u8>,
}

impl MemorySource {
    /// Creates a new `MemorySource` from a vector of bytes.
    ///
    /// # Arguments
    /// * `data` - The byte vector containing the MF4 file data
    ///
    /// # Example
    /// ```
    /// use falcon_mdf::io::memory::MemorySource;
    /// use falcon_mdf::io::ByteSource;
    ///
    /// let data = vec![0u8; 64];
    /// let source = MemorySource::new(data);
    /// assert_eq!(source.len(), 64);
    /// ```
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Returns a reference to the underlying byte slice.
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }
}

impl ByteSource for MemorySource {
    fn len(&self) -> u64 {
        self.data.len() as u64
    }

    fn read_bytes(&self, offset: u64, len: usize) -> Result<ByteSlice<'_>> {
        let total_len = self.data.len() as u64;
        if offset >= total_len {
            return Err(Mf4Error::truncated(offset, len, 0));
        }

        let start = offset as usize;
        let available = self.data.len().saturating_sub(start);
        if len > available {
            return Err(Mf4Error::truncated(offset, len, available));
        }

        let end = match start.checked_add(len) {
            Some(end) if end <= self.data.len() => end,
            _ => return Err(Mf4Error::truncated(offset, len, available)),
        };

        Ok(ByteSlice::borrowed(&self.data[start..end]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_source_basic() {
        let data = b"Hello, MF4 World!".to_vec();
        let source = MemorySource::new(data);
        assert_eq!(source.len(), 17);
        assert_eq!(source.as_slice(), b"Hello, MF4 World!");

        let slice = source.read_bytes(0, 5).unwrap();
        assert_eq!(&*slice, b"Hello");

        let slice = source.read_bytes(7, 3).unwrap();
        assert_eq!(&*slice, b"MF4");
    }

    #[test]
    fn test_memory_source_out_of_bounds() {
        let data = b"Short".to_vec();
        let source = MemorySource::new(data);

        // Reading beyond data length should fail
        let result = source.read_bytes(0, 100);
        assert!(result.is_err());

        // Reading from beyond data length should fail
        let result = source.read_bytes(100, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_memory_source_zero_copy() {
        let data = b"Test data for zero-copy verification".to_vec();
        let source = MemorySource::new(data);

        let slice = source.read_bytes(0, 4).unwrap();
        assert!(matches!(slice, ByteSlice::Borrowed(_)));
        assert_eq!(&*slice, b"Test");
    }

    #[test]
    fn test_memory_source_empty() {
        let source = MemorySource::new(Vec::new());
        assert_eq!(source.len(), 0);
        assert!(source.is_empty());
        let result = source.read_bytes(0, 10);
        assert!(result.is_err());
    }
}
