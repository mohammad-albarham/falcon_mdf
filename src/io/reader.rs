//! Buffered file I/O backend.
//!
//! This module provides a buffered reader approach for reading MF4 files.
//! While not as efficient as memory mapping for large files, it works
//! reliably on all platforms and may be preferred for smaller files or
//! when memory-mapped I/O is not available.

use crate::error::{Mf4Error, Result};
use crate::io::{ByteSlice, ByteSource};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

/// A buffered file reader source.
///
/// This struct wraps a file handle with a mutex to allow thread-safe
/// concurrent reads from different offsets.
pub struct BufferedSource {
    /// The underlying file, wrapped in a mutex for thread safety.
    file: Mutex<File>,
    /// The file length in bytes (cached to avoid repeated metadata calls).
    len: u64,
}

impl BufferedSource {
    /// Opens a file for buffered reading.
    ///
    /// # Arguments
    /// * `path` - Path to the file to open
    ///
    /// # Returns
    /// A new `BufferedSource` or an error if the file cannot be opened.
    ///
    /// # Example
    /// ```no_run
    /// use falcon_mdf::io::reader::BufferedSource;
    /// use falcon_mdf::io::ByteSource;  // Required for len() method
    ///
    /// let source = BufferedSource::open("data.mf4")?;
    /// println!("File size: {} bytes", source.len());
    /// # Ok::<(), falcon_mdf::error::Mf4Error>(())
    /// ```
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        let metadata = file.metadata()?;
        let len = metadata.len();

        Ok(BufferedSource {
            file: Mutex::new(file),
            len,
        })
    }
}

impl ByteSource for BufferedSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_bytes(&self, offset: u64, len: usize) -> Result<ByteSlice<'_>> {
        // Check bounds before acquiring the lock
        if offset >= self.len {
            return Err(Mf4Error::truncated(offset, len, 0));
        }

        let available = (self.len - offset) as usize;
        if len > available {
            return Err(Mf4Error::truncated(offset, len, available));
        }

        // Acquire the file lock and perform the read
        let mut file = self.file.lock().map_err(|_| {
            Mf4Error::parse_error("Failed to acquire file lock")
        })?;

        file.seek(SeekFrom::Start(offset))?;

        let mut buffer = vec![0u8; len];
        file.read_exact(&mut buffer)?;

        Ok(ByteSlice::owned(buffer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_buffered_source_basic() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Hello, MF4 World!").unwrap();
        file.flush().unwrap();

        let source = BufferedSource::open(file.path()).unwrap();
        assert_eq!(source.len(), 17);

        let slice = source.read_bytes(0, 5).unwrap();
        assert_eq!(&*slice, b"Hello");

        let slice = source.read_bytes(7, 3).unwrap();
        assert_eq!(&*slice, b"MF4");
    }

    #[test]
    fn test_buffered_source_out_of_bounds() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Short").unwrap();
        file.flush().unwrap();

        let source = BufferedSource::open(file.path()).unwrap();

        // Reading beyond file length should fail
        let result = source.read_bytes(0, 100);
        assert!(result.is_err());

        // Reading from beyond file length should fail
        let result = source.read_bytes(100, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_buffered_source_owned_data() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"Test data").unwrap();
        file.flush().unwrap();

        let source = BufferedSource::open(file.path()).unwrap();
        let slice = source.read_bytes(0, 4).unwrap();

        // Buffered source always returns owned data
        assert!(matches!(slice, ByteSlice::Owned(_)));
        assert_eq!(&*slice, b"Test");
    }

    #[test]
    fn test_buffered_source_multiple_reads() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ").unwrap();
        file.flush().unwrap();

        let source = BufferedSource::open(file.path()).unwrap();

        // Multiple reads from different offsets
        let slice1 = source.read_bytes(0, 5).unwrap();
        let slice2 = source.read_bytes(10, 5).unwrap();
        let slice3 = source.read_bytes(20, 6).unwrap();

        assert_eq!(&*slice1, b"ABCDE");
        assert_eq!(&*slice2, b"KLMNO");
        assert_eq!(&*slice3, b"UVWXYZ");
    }
}
