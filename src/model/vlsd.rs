//! Payload lookup for variable-length channels.
//!
//! A variable-length channel stores no value in its record. The record holds a
//! byte offset into a separate stream of length-prefixed payloads:
//!
//! ```text
//! [u32 length][payload bytes][u32 length][payload bytes]...
//!  ^                          ^
//!  offset 0                   offset 4 + first length
//! ```
//!
//! The offsets a record carries address that stream, so reading such a channel
//! means walking the stream once to learn where each payload begins, then
//! resolving each record's offset against it.

/// Length-prefixed payloads a variable-length channel points into.
#[derive(Debug, Clone, Default)]
pub struct VlsdPayloads {
    /// Every payload concatenated, prefixes stripped.
    data: Vec<u8>,
    /// `(stored offset, start in data, length)`, ascending by offset.
    ///
    /// Both construction paths walk their input forwards, so offsets arrive in
    /// order and the table is sorted without sorting it. Binary searching that
    /// beats hashing here: there is one entry per sample, so a hash map spends
    /// more time hashing and chasing pointers than a search over a compact,
    /// cache-friendly array does.
    index: Vec<(u64, u32, u32)>,
}

/// Size of the length prefix in front of each payload.
const PREFIX: usize = 4;

impl VlsdPayloads {
    /// Builds the index from a contiguous stream of length-prefixed payloads.
    ///
    /// This is the layout of a signal-data block, where payloads follow one
    /// another with nothing between them.
    pub fn from_stream(stream: &[u8]) -> Self {
        let mut payloads = VlsdPayloads::default();
        let mut pos = 0usize;

        while pos + PREFIX <= stream.len() {
            let len = u32::from_le_bytes([
                stream[pos],
                stream[pos + 1],
                stream[pos + 2],
                stream[pos + 3],
            ]) as usize;

            let from = pos + PREFIX;
            let Some(payload) = stream.get(from..from + len) else {
                // The stream is truncated; keep what was read rather than
                // discarding the payloads already found.
                break;
            };

            payloads.push(pos as u64, payload);
            pos = from + len;
        }

        payloads
    }

    /// Builds the index from the records of a variable-length channel group.
    ///
    /// Bus loggers store payloads this way: as records of their own group,
    /// interleaved with the records that point at them. Each record is a record
    /// ID followed by the same length-prefixed payload as a signal-data block,
    /// and the offsets the pointing records carry address the payloads as
    /// though they were contiguous — so the running position, not the position
    /// in the file, is what identifies a payload.
    pub fn from_records(raw: &[u8], offsets: &[u64], rec_id_size: usize) -> Self {
        let mut payloads = VlsdPayloads::default();
        let mut virtual_offset = 0u64;

        for &offset in offsets {
            let at = offset as usize + rec_id_size;
            let Some(header) = raw.get(at..at + PREFIX) else {
                break;
            };
            let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;

            let from = at + PREFIX;
            let Some(payload) = raw.get(from..from + len) else {
                break;
            };

            payloads.push(virtual_offset, payload);
            virtual_offset += (PREFIX + len) as u64;
        }

        payloads
    }

    fn push(&mut self, offset: u64, payload: &[u8]) {
        let start = self.data.len() as u32;
        self.data.extend_from_slice(payload);
        self.index.push((offset, start, payload.len() as u32));
    }

    /// Returns the payload a record's stored offset refers to.
    ///
    /// `None` when no payload begins at that offset, which means the file is
    /// inconsistent — the caller decides whether that is fatal.
    pub fn get(&self, offset: u64) -> Option<&[u8]> {
        self.get_from(offset, 0).map(|(payload, _)| payload)
    }

    /// Returns the payload at `offset`, and where it sits in the table.
    ///
    /// Records normally reference payloads in the order they were written, so
    /// the caller passes the position after its last hit as `hint`. Checking
    /// that one first turns the common case into a single comparison instead of
    /// a binary search over every payload in the file; a miss simply falls back
    /// to the search, so out-of-order files stay correct.
    pub fn get_from(&self, offset: u64, hint: usize) -> Option<(&[u8], usize)> {
        let at = match self.index.get(hint) {
            Some(&(o, _, _)) if o == offset => hint,
            _ => self
                .index
                .binary_search_by_key(&offset, |&(o, _, _)| o)
                .ok()?,
        };
        let (_, start, len) = self.index[at];
        let payload = self.data.get(start as usize..(start + len) as usize)?;
        Some((payload, at))
    }

    /// Returns the common payload length when every payload has the same one.
    ///
    /// Bus logs are overwhelmingly of this shape, and knowing it up front lets a
    /// reader size its output exactly and copy fixed-width chunks instead of
    /// tracking where each sample begins.
    pub fn uniform_len(&self) -> Option<usize> {
        let first = self.index.first()?.2 as usize;
        self.index
            .iter()
            .all(|&(_, _, len)| len as usize == first)
            .then_some(first)
    }

    /// Returns the total size of all payloads.
    ///
    /// An upper bound on what decoding a channel that references each payload
    /// once will produce, and free to obtain — useful for sizing the output
    /// buffer without walking the offsets twice.
    pub fn total_bytes(&self) -> usize {
        self.data.len()
    }

    /// Returns the number of payloads found.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Returns true if the offset table is ascending, which binary search
    /// requires. Both constructors guarantee it; this checks the guarantee.
    #[cfg(test)]
    fn is_sorted(&self) -> bool {
        self.index.windows(2).all(|w| w[0].0 < w[1].0)
    }

    /// Returns true if no payloads were found.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a stream of length-prefixed payloads.
    fn stream(payloads: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        for p in payloads {
            out.extend_from_slice(&(p.len() as u32).to_le_bytes());
            out.extend_from_slice(p);
        }
        out
    }

    #[test]
    fn the_offset_table_is_built_in_ascending_order() {
        let raw = stream(&[&[1, 2, 3], &[4, 5], &[6], &[7, 8, 9, 10]]);
        assert!(VlsdPayloads::from_stream(&raw).is_sorted());

        let mut rec = vec![0u8; 60];
        let mut pos = 0;
        let mut offsets = Vec::new();
        for payload in [&[1u8, 2][..], &[3][..], &[4, 5, 6][..]] {
            offsets.push(pos as u64);
            rec[pos] = 9;
            rec[pos + 1..pos + 5].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            rec[pos + 5..pos + 5 + payload.len()].copy_from_slice(payload);
            pos += 5 + payload.len() + 2;
        }
        assert!(VlsdPayloads::from_records(&rec, &offsets, 1).is_sorted());
    }

    #[test]
    fn reports_a_uniform_payload_length_only_when_all_agree() {
        let same = VlsdPayloads::from_stream(&stream(&[&[1, 2], &[3, 4], &[5, 6]]));
        assert_eq!(same.uniform_len(), Some(2));

        let mixed = VlsdPayloads::from_stream(&stream(&[&[1, 2], &[3]]));
        assert_eq!(mixed.uniform_len(), None);

        assert_eq!(VlsdPayloads::default().uniform_len(), None, "no payloads");
    }

    #[test]
    fn resolves_payloads_by_their_offset_in_the_stream() {
        let raw = stream(&[&[1, 2, 3], &[4, 5], &[6]]);
        let p = VlsdPayloads::from_stream(&raw);

        assert_eq!(p.len(), 3);
        assert_eq!(p.get(0), Some(&[1, 2, 3][..]));
        assert_eq!(p.get(7), Some(&[4, 5][..]), "4 + 3 bytes in");
        assert_eq!(p.get(13), Some(&[6][..]), "7 + 4 + 2 bytes in");
    }

    #[test]
    fn the_sequential_hint_agrees_with_the_search() {
        let raw = stream(&[&[1, 2, 3], &[4, 5], &[6], &[7, 8]]);
        let p = VlsdPayloads::from_stream(&raw);

        // Walking forwards, each hit should be found at the hinted position.
        let offsets = [0u64, 7, 13, 18];
        let mut hint = 0;
        for (i, &o) in offsets.iter().enumerate() {
            let (payload, at) = p.get_from(o, hint).expect("payload should resolve");
            assert_eq!(at, i, "hint should land on the next payload");
            assert_eq!(Some(payload), p.get(o), "hinted and searched agree");
            hint = at + 1;
        }
    }

    #[test]
    fn a_wrong_hint_still_finds_the_payload() {
        let raw = stream(&[&[1], &[2], &[3]]);
        let p = VlsdPayloads::from_stream(&raw);
        // Deliberately misleading hints, including one past the end.
        assert_eq!(p.get_from(0, 2).map(|(b, _)| b), Some(&[1u8][..]));
        assert_eq!(p.get_from(10, 0).map(|(b, _)| b), Some(&[3u8][..]));
        assert_eq!(p.get_from(5, 99).map(|(b, _)| b), Some(&[2u8][..]));
    }

    #[test]
    fn an_offset_no_payload_starts_at_is_not_resolved() {
        let raw = stream(&[&[1, 2, 3]]);
        let p = VlsdPayloads::from_stream(&raw);
        assert_eq!(p.get(1), None, "mid-payload offsets are not valid");
        assert_eq!(p.get(999), None);
    }

    #[test]
    fn handles_zero_length_payloads() {
        let raw = stream(&[&[], &[9]]);
        let p = VlsdPayloads::from_stream(&raw);
        assert_eq!(p.get(0), Some(&[][..]));
        assert_eq!(p.get(4), Some(&[9][..]));
    }

    #[test]
    fn a_truncated_stream_keeps_what_was_read() {
        let mut raw = stream(&[&[1, 2], &[3, 4]]);
        raw.truncate(9); // second payload's bytes are cut short
        let p = VlsdPayloads::from_stream(&raw);
        assert_eq!(p.len(), 1, "the complete first payload survives");
        assert_eq!(p.get(0), Some(&[1, 2][..]));
    }

    #[test]
    fn a_length_beyond_the_stream_does_not_panic() {
        let raw = vec![0xFF, 0xFF, 0xFF, 0xFF, 1, 2, 3];
        let p = VlsdPayloads::from_stream(&raw);
        assert!(p.is_empty());
    }

    #[test]
    fn record_form_addresses_payloads_as_though_contiguous() {
        // Two records, each [rec_id][len][payload], not adjacent in the buffer.
        let mut raw = vec![0u8; 40];
        // record at 4: id, len=3, payload
        raw[4] = 2;
        raw[5..9].copy_from_slice(&3u32.to_le_bytes());
        raw[9..12].copy_from_slice(&[0xAA, 0xBB, 0xCC]);
        // record at 20: id, len=2, payload
        raw[20] = 2;
        raw[21..25].copy_from_slice(&2u32.to_le_bytes());
        raw[25..27].copy_from_slice(&[0xDD, 0xEE]);

        let p = VlsdPayloads::from_records(&raw, &[4, 20], 1);

        assert_eq!(p.len(), 2);
        // Offsets count the virtual stream, not positions in the buffer.
        assert_eq!(p.get(0), Some(&[0xAA, 0xBB, 0xCC][..]));
        assert_eq!(p.get(7), Some(&[0xDD, 0xEE][..]), "4 + 3 into the stream");
        assert_eq!(p.get(20), None, "buffer positions are not stream offsets");
    }
}
