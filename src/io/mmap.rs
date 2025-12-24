//! Memory-mapped file I/O backend.
//!
//! This module provides zero-copy access to MF4 file data through
//! memory mapping. This is the most efficient approach for large files
//! as it allows the operating system to manage caching and only loads
//! pages into memory as they are accessed.

use crate::error::{Mf4Error, Result};
use crate::io::{ByteSlice, ByteSource};
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;

/// A memory-mapped file source.
///
/// This struct wraps a memory-mapped file and provides safe access
/// to its contents as byte slices.
pub struct MmapSource {
    /// The memory-mapped file.
    mmap: Mmap,
    /// The file length in bytes.
    len: u64,
}

impl MmapSource {
    /// Opens a file and creates a memory mapping.
    ///
    /// # Arguments
    /// * `path` - Path to the file to open
    ///
    /// # Returns
    /// A new `MmapSource` or an error if the file cannot be opened
    /// or mapped.
    ///
    /// # Safety
    /// The file must not be modified while the mapping is active.
    /// This is generally safe for MF4 files which are typically
    /// read-only measurement data.
    ///
    /// # Example
    /// ```no_run
    /// use falcon_mdf::io::mmap::MmapSource;
    ///
    /// let source = MmapSource::open("data.mf4")?;
    /// println!("File size: {} bytes", source.len());
    /// # Ok::<(), falcon_mdf::error::Mf4Error>(())
    /// ```
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        let metadata = file.metadata()?;
        let len = metadata.len();

        // Handle empty files
        if len == 0 {
            return Err(Mf4Error::MmapFailed("File is empty".to_string()));
        }

        // SAFETY: We're opening the file read-only, and MF4 files are
        // typically not modified during reading. The mapping is safe
        // as long as the file isn't modified by external processes.
        let mmap = unsafe {
            Mmap::map(&file).map_err(|e| Mf4Error::MmapFailed(e.to_string()))?
        };

        Ok(MmapSource { mmap, len })
    }

    /// Returns a direct reference to the underlying byte slice.
    ///
    /// This is useful when you need the entire file contents and
    /// want to avoid the `ByteSlice` wrapper overhead.
    pub fn as_slice(&self) -> &[u8] {
        &self.mmap
    }
}

impl ByteSource for MmapSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_bytes(&self, offset: u64, len: usize) -> Result<ByteSlice<'_>> {
        let start = offset as usize;
        let end = start.checked_add(len).ok_or_else(|| {
            Mf4Error::truncated(offset, len, 0)
        })?;

        if end > self.mmap.len() {
            return Err(Mf4Error::truncated(
                offset,
                len,
                self.mmap.len().saturating_sub(start),
            ));
        }

        // Zero-copy: return a borrowed slice directly from the mmap
        Ok(ByteSlice::borrowed(&self.mmap[start..end]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_mmap_source_basic() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Hello, MF4 World!").unwrap();
        file.flush().unwrap();

        let source = MmapSource::open(file.path()).unwrap();
        assert_eq!(source.len(), 17);

        let slice = source.read_bytes(0, 5).unwrap();
        assert_eq!(&*slice, b"Hello");

        let slice = source.read_bytes(7, 3).unwrap();
        assert_eq!(&*slice, b"MF4");
    }

    #[test]
    fn test_mmap_source_out_of_bounds() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Short").unwrap();
        file.flush().unwrap();

        let source = MmapSource::open(file.path()).unwrap();

        // Reading beyond file length should fail
        let result = source.read_bytes(0, 100);
        assert!(result.is_err());

        // Reading from beyond file length should fail
        let result = source.read_bytes(100, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_mmap_source_empty_file() {
        let file = NamedTempFile::new().unwrap();
        // Don't write anything - file is empty

        let result = MmapSource::open(file.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_mmap_zero_copy() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Test data for zero-copy verification")
            .unwrap();
        file.flush().unwrap();

        let source = MmapSource::open(file.path()).unwrap();

        // Get two overlapping slices
        let slice1 = source.read_bytes(0, 10).unwrap();
        let slice2 = source.read_bytes(5, 10).unwrap();

        // Verify the data is correct
        assert_eq!(&*slice1, b"Test data ");
        assert_eq!(&*slice2, b"data for z");

        // Both should be borrowed (zero-copy)
        assert!(matches!(slice1, ByteSlice::Borrowed(_)));
        assert!(matches!(slice2, ByteSlice::Borrowed(_)));
    }
}
