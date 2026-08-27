//! I/O abstraction layer for reading MF4 files.
//!
//! This module provides unified access to file data through either
//! memory-mapped files or buffered readers, allowing the rest of
//! the crate to work with byte slices without caring about the
//! underlying I/O strategy.

#[cfg(feature = "mmap")]
pub mod mmap;
pub mod memory;
pub mod reader;

use crate::error::Result;
use std::ops::Deref;
use std::path::Path;

/// A trait for byte sources that can provide slices of file data.
///
/// This abstraction allows the parser to work with both memory-mapped
/// files and buffered readers through a uniform interface.
pub trait ByteSource: Send + Sync {
    /// Returns the total length of the data source in bytes.
    fn len(&self) -> u64;

    /// Returns true if the data source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a slice of bytes from the given range.
    ///
    /// # Arguments
    /// * `offset` - The starting offset in bytes
    /// * `len` - The number of bytes to read
    ///
    /// # Returns
    /// A `ByteSlice` containing the requested data, or an error if
    /// the range is out of bounds or I/O fails.
    fn read_bytes(&self, offset: u64, len: usize) -> Result<ByteSlice<'_>>;

    /// Returns all bytes from offset to end of file.
    fn read_bytes_to_end(&self, offset: u64) -> Result<ByteSlice<'_>> {
        let remaining = self.len().saturating_sub(offset) as usize;
        self.read_bytes(offset, remaining)
    }
}

/// A slice of bytes that may be borrowed from a memory map or owned.
///
/// This type provides zero-copy access when using memory-mapped files,
/// while still supporting buffered reads by owning the data when necessary.
#[derive(Debug)]
pub enum ByteSlice<'a> {
    /// A borrowed slice from a memory-mapped file.
    Borrowed(&'a [u8]),
    /// An owned buffer from a buffered read.
    Owned(Vec<u8>),
}

impl<'a> Deref for ByteSlice<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
            ByteSlice::Borrowed(slice) => slice,
            ByteSlice::Owned(vec) => vec,
        }
    }
}

impl<'a> AsRef<[u8]> for ByteSlice<'a> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl<'a> ByteSlice<'a> {
    /// Creates a new borrowed byte slice.
    pub fn borrowed(slice: &'a [u8]) -> Self {
        ByteSlice::Borrowed(slice)
    }

    /// Creates a new owned byte slice.
    pub fn owned(data: Vec<u8>) -> Self {
        ByteSlice::Owned(data)
    }

    /// Converts to an owned `Vec<u8>`, cloning if necessary.
    pub fn into_owned(self) -> Vec<u8> {
        match self {
            ByteSlice::Borrowed(slice) => slice.to_vec(),
            ByteSlice::Owned(vec) => vec,
        }
    }
}

/// The I/O backend type used for reading MF4 files.
#[non_exhaustive]
pub enum IoBackend {
    /// Memory-mapped file access (zero-copy, best for large files).
    #[cfg(feature = "mmap")]
    Mmap(mmap::MmapSource),
    /// Buffered reader (works on all platforms, lower memory usage for partial reads).
    Buffered(reader::BufferedSource),
    /// In-memory byte buffer (zero-copy, works without a filesystem).
    Memory(memory::MemorySource),
}

impl IoBackend {
    /// Opens a file using memory-mapped I/O.
    ///
    /// This is the recommended approach for large files as it provides
    /// zero-copy access to file data and lets the OS handle caching.
    ///
    /// # Example
    /// ```no_run
    /// use falcon_mdf::io::IoBackend;
    ///
    /// let backend = IoBackend::open_mmap("data.mf4")?;
    /// # Ok::<(), falcon_mdf::error::Mf4Error>(())
    /// ```
    #[cfg(feature = "mmap")]
    pub fn open_mmap<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(IoBackend::Mmap(mmap::MmapSource::open(path)?))
    }

    /// Opens a file using buffered I/O.
    ///
    /// This approach reads data into owned buffers. It's useful when
    /// memory-mapped I/O is not available or when working with files
    /// on network drives where mmap may not work well.
    ///
    /// # Example
    /// ```no_run
    /// use falcon_mdf::io::IoBackend;
    ///
    /// let backend = IoBackend::open_buffered("data.mf4")?;
    /// # Ok::<(), falcon_mdf::error::Mf4Error>(())
    /// ```
    pub fn open_buffered<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(IoBackend::Buffered(reader::BufferedSource::open(path)?))
    }

    /// Creates an I/O backend from in-memory bytes.
    ///
    /// # Example
    /// ```
    /// use falcon_mdf::io::IoBackend;
    ///
    /// let backend = IoBackend::from_bytes(vec![0u8; 64]);
    /// ```
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        IoBackend::Memory(memory::MemorySource::new(bytes))
    }

    /// Opens a file using the best available I/O strategy.
    ///
    /// This will use memory-mapped I/O if the `mmap` feature is enabled,
    /// otherwise falls back to buffered I/O.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        #[cfg(feature = "mmap")]
        {
            Self::open_mmap(path)
        }
        #[cfg(not(feature = "mmap"))]
        {
            Self::open_buffered(path)
        }
    }
}

impl ByteSource for IoBackend {
    fn len(&self) -> u64 {
        match self {
            #[cfg(feature = "mmap")]
            IoBackend::Mmap(source) => source.len(),
            IoBackend::Buffered(source) => source.len(),
            IoBackend::Memory(source) => source.len(),
        }
    }

    fn read_bytes(&self, offset: u64, len: usize) -> Result<ByteSlice<'_>> {
        if len == 0 && offset <= self.len() {
            return Ok(ByteSlice::borrowed(&[]));
        }
        match self {
            #[cfg(feature = "mmap")]
            IoBackend::Mmap(source) => source.read_bytes(offset, len),
            IoBackend::Buffered(source) => source.read_bytes(offset, len),
            IoBackend::Memory(source) => source.read_bytes(offset, len),
        }
    }
}

/// Lets an `Arc<IoBackend>` be used wherever a byte source is expected, so the
/// backend can be held behind a shared pointer without every call site having
/// to dereference it.
impl<T: ByteSource + ?Sized> ByteSource for std::sync::Arc<T> {
    fn len(&self) -> u64 {
        (**self).len()
    }

    fn read_bytes(&self, offset: u64, len: usize) -> Result<ByteSlice<'_>> {
        (**self).read_bytes(offset, len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_slice_deref() {
        let data = vec![1, 2, 3, 4, 5];
        let slice = ByteSlice::owned(data.clone());
        assert_eq!(&*slice, &[1, 2, 3, 4, 5]);

        let borrowed = ByteSlice::borrowed(&data);
        assert_eq!(&*borrowed, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_byte_slice_into_owned() {
        let data = vec![1, 2, 3];
        let slice = ByteSlice::borrowed(&data);
        let owned = slice.into_owned();
        assert_eq!(owned, vec![1, 2, 3]);
    }
}
