//! A byte source that fetches ranges on demand.
//!
//! [`RangeSource`] reads a file it does not hold: every read is answered by a
//! caller-supplied `fetch(offset, buf)` that fills `buf` with the bytes at
//! `offset`. That is the shape of an HTTP range request, a browser `File`
//! slice read in a worker, an object-store GET, or any `Read + Seek`.
//!
//! Opening an MDF file is a walk over many small blocks, most of them near
//! each other, so reads are rounded out to fixed-size windows and the last
//! few windows are kept. A walk then costs roughly one fetch per window it
//! touches rather than one per block. A read larger than the whole cache — a
//! data block's payload — bypasses it, so one big read cannot evict every
//! window the block walk still needs.

use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use crate::error::{Mf4Error, Result};
use crate::io::{ByteSlice, ByteSource};

/// Default window: 64 KiB, the granularity the wasm streaming design settled
/// on (`docs/wasm-streaming-design.md`).
pub const DEFAULT_WINDOW: usize = 64 * 1024;
/// Default number of windows kept: 4 MiB of cache at the default window.
pub const DEFAULT_WINDOWS: usize = 64;

/// The range fetch behind a [`RangeSource`]: fill `buf` with the bytes at
/// `offset`, or fail. Implemented for every matching closure.
pub trait RangeFetch: Fn(u64, &mut [u8]) -> Result<()> + Send + Sync {}

impl<F: Fn(u64, &mut [u8]) -> Result<()> + Send + Sync> RangeFetch for F {}

/// A [`ByteSource`] backed by an on-demand range fetch, with a small LRU of
/// fixed-size windows in front of it.
///
/// `fetch(offset, buf)` must fill all of `buf` with the bytes starting at
/// `offset`, or return an error; it is never asked for bytes past `len`.
///
/// # Example
/// ```
/// use falcon_mdf::io::range::RangeSource;
/// use falcon_mdf::io::ByteSource;
///
/// let file: Vec<u8> = (0..=255).collect();
/// let source = RangeSource::new(file.len() as u64, move |offset, buf: &mut [u8]| {
///     let start = offset as usize;
///     buf.copy_from_slice(&file[start..start + buf.len()]);
///     Ok(())
/// });
/// assert_eq!(&*source.read_bytes(10, 3)?, &[10, 11, 12]);
/// # Ok::<(), falcon_mdf::error::Mf4Error>(())
/// ```
pub struct RangeSource<F> {
    len: u64,
    window: usize,
    max_windows: usize,
    fetch: F,
    cache: Mutex<WindowCache>,
}

#[derive(Default)]
struct WindowCache {
    /// (window index, bytes, last use). A linear scan: the cache holds tens of
    /// windows, and each read touches one or two.
    windows: Vec<(u64, Arc<Vec<u8>>, u64)>,
    clock: u64,
    fetches: u64,
}

impl<F: RangeFetch> RangeSource<F> {
    /// A source of `len` bytes read through `fetch`, with the default
    /// 64 KiB × 64 window cache.
    pub fn new(len: u64, fetch: F) -> Self {
        Self::with_cache(len, DEFAULT_WINDOW, DEFAULT_WINDOWS, fetch)
    }

    /// As [`RangeSource::new`], with an explicit window size and window count.
    /// Both are clamped to at least 1.
    pub fn with_cache(len: u64, window: usize, max_windows: usize, fetch: F) -> Self {
        Self {
            len,
            window: window.max(1),
            max_windows: max_windows.max(1),
            fetch,
            cache: Mutex::new(WindowCache::default()),
        }
    }

    /// How many times `fetch` has been called, for callers tuning the window.
    pub fn fetch_count(&self) -> u64 {
        self.cache.lock().map(|c| c.fetches).unwrap_or(0)
    }

    fn fetch_exact(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        if let Ok(mut c) = self.cache.lock() {
            c.fetches += 1;
        }
        (self.fetch)(offset, buf)
    }

    /// The bytes of window `index`, from the cache or freshly fetched. The
    /// last window of the file is short.
    fn window_bytes(&self, index: u64) -> Result<Arc<Vec<u8>>> {
        {
            let mut c = self
                .cache
                .lock()
                .map_err(|_| Mf4Error::parse_error("range source cache lock poisoned"))?;
            c.clock += 1;
            let now = c.clock;
            if let Some(entry) = c.windows.iter_mut().find(|w| w.0 == index) {
                entry.2 = now;
                return Ok(Arc::clone(&entry.1));
            }
        }
        // Fetched outside the lock: a slow fetch must not block reads that
        // the cache can already answer.
        let start = index * self.window as u64;
        let size = (self.len - start).min(self.window as u64) as usize;
        let mut bytes = vec![0u8; size];
        self.fetch_exact(start, &mut bytes)?;
        let bytes = Arc::new(bytes);

        let mut c = self
            .cache
            .lock()
            .map_err(|_| Mf4Error::parse_error("range source cache lock poisoned"))?;
        c.clock += 1;
        let now = c.clock;
        if !c.windows.iter().any(|w| w.0 == index) {
            if c.windows.len() >= self.max_windows {
                if let Some(oldest) = c
                    .windows
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, w)| w.2)
                    .map(|(i, _)| i)
                {
                    c.windows.swap_remove(oldest);
                }
            }
            c.windows.push((index, Arc::clone(&bytes), now));
        }
        Ok(bytes)
    }
}

impl<F: RangeFetch> ByteSource for RangeSource<F> {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_bytes(&self, offset: u64, len: usize) -> Result<ByteSlice<'_>> {
        let available = self.len.saturating_sub(offset);
        if offset >= self.len || len as u64 > available {
            return Err(Mf4Error::truncated(
                offset,
                len,
                available.min(usize::MAX as u64) as usize,
            ));
        }
        if len == 0 {
            return Ok(ByteSlice::owned(Vec::new()));
        }

        // Larger than the whole cache: read straight through rather than
        // evicting every window the block walk still wants.
        if len >= self.window.saturating_mul(self.max_windows) {
            let mut out = vec![0u8; len];
            self.fetch_exact(offset, &mut out)?;
            return Ok(ByteSlice::owned(out));
        }

        let window = self.window as u64;
        let end = offset + len as u64;
        let mut out = Vec::with_capacity(len);
        let mut index = offset / window;
        while index * window < end {
            let bytes = self.window_bytes(index)?;
            let start = index * window;
            let from = offset.max(start) - start;
            let to = end.min(start + bytes.len() as u64) - start;
            out.extend_from_slice(&bytes[from as usize..to as usize]);
            index += 1;
        }
        Ok(ByteSlice::owned(out))
    }
}

/// A [`RangeSource`] over anything that reads and seeks — a file, a network
/// stream with range support, a cursor. The reader is locked for each fetch.
///
/// # Example
/// ```
/// use std::io::Cursor;
/// use falcon_mdf::io::range::read_seek_source;
/// use falcon_mdf::io::ByteSource;
///
/// let source = read_seek_source(Cursor::new(vec![7u8; 100]))?;
/// assert_eq!(source.len(), 100);
/// assert_eq!(&*source.read_bytes(98, 2)?, &[7, 7]);
/// # Ok::<(), falcon_mdf::error::Mf4Error>(())
/// ```
pub fn read_seek_source<R>(mut reader: R) -> Result<RangeSource<impl RangeFetch>>
where
    R: Read + Seek + Send + 'static,
{
    let len = reader.seek(SeekFrom::End(0))?;
    let reader = Mutex::new(reader);
    Ok(RangeSource::new(len, move |offset, buf: &mut [u8]| {
        let mut r = reader
            .lock()
            .map_err(|_| Mf4Error::parse_error("reader lock poisoned"))?;
        r.seek(SeekFrom::Start(offset))?;
        r.read_exact(buf)?;
        Ok(())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn source_over(data: Vec<u8>, window: usize, windows: usize) -> RangeSource<impl RangeFetch> {
        let len = data.len() as u64;
        RangeSource::with_cache(len, window, windows, move |offset, buf: &mut [u8]| {
            let start = offset as usize;
            assert!(start + buf.len() <= data.len(), "fetch past the end");
            buf.copy_from_slice(&data[start..start + buf.len()]);
            Ok(())
        })
    }

    #[test]
    fn every_range_matches_the_underlying_bytes() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
        let source = source_over(data.clone(), 64, 3);
        for offset in [0usize, 1, 63, 64, 65, 127, 500, 936, 999] {
            for len in [1usize, 2, 63, 64, 65, 129, 191, 192] {
                if offset + len > data.len() {
                    continue;
                }
                let got = source.read_bytes(offset as u64, len).unwrap();
                assert_eq!(
                    &*got,
                    &data[offset..offset + len],
                    "offset {offset} len {len}"
                );
            }
        }
    }

    #[test]
    fn reads_past_the_end_are_truncation_errors() {
        let source = source_over(vec![0u8; 100], 64, 2);
        assert!(source.read_bytes(100, 1).is_err());
        assert!(source.read_bytes(90, 11).is_err());
        assert!(source.read_bytes(u64::MAX, 1).is_err());
        assert!(source.read_bytes(99, 1).is_ok());
    }

    #[test]
    fn repeated_reads_inside_one_window_fetch_once() {
        let source = source_over(vec![1u8; 4096], 1024, 4);
        for i in 0..50 {
            source.read_bytes(i * 10, 8).unwrap();
        }
        assert_eq!(source.fetch_count(), 1);
    }

    #[test]
    fn the_least_recently_used_window_is_the_one_evicted() {
        let fetched = Arc::new(AtomicUsize::new(0));
        let f = Arc::clone(&fetched);
        let source = RangeSource::with_cache(400, 100, 2, move |_, buf: &mut [u8]| {
            f.fetch_add(1, Ordering::SeqCst);
            buf.fill(0);
            Ok(())
        });
        source.read_bytes(0, 1).unwrap(); // window 0
        source.read_bytes(100, 1).unwrap(); // window 1
        source.read_bytes(0, 1).unwrap(); // window 0 again: now most recent
        source.read_bytes(200, 1).unwrap(); // window 2 evicts window 1
        assert_eq!(fetched.load(Ordering::SeqCst), 3);
        source.read_bytes(0, 1).unwrap(); // still cached
        assert_eq!(fetched.load(Ordering::SeqCst), 3);
        source.read_bytes(100, 1).unwrap(); // was evicted
        assert_eq!(fetched.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn a_read_larger_than_the_cache_goes_straight_through() {
        let data: Vec<u8> = (0..=255).cycle().take(1000).collect();
        let source = source_over(data.clone(), 64, 2);
        let got = source.read_bytes(3, 500).unwrap();
        assert_eq!(&*got, &data[3..503]);
        assert_eq!(source.fetch_count(), 1, "one direct fetch, no windows");
    }

    #[test]
    fn a_failing_fetch_is_an_error_not_a_cached_window() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);
        let source = RangeSource::with_cache(100, 50, 2, move |_, buf: &mut [u8]| {
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(Mf4Error::parse_error("network down"));
            }
            buf.fill(9);
            Ok(())
        });
        assert!(source.read_bytes(0, 4).is_err());
        assert_eq!(&*source.read_bytes(0, 4).unwrap(), &[9, 9, 9, 9]);
    }
}
