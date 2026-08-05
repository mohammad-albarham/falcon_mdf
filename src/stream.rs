//! Block-by-block signal reading, for files too large to materialise.
//!
//! [`Mf4File::signal`](crate::Mf4File::signal) assembles a channel group's
//! entire record stream into one buffer before a value can be read. That is the
//! right trade for a measurement file and the wrong one for a bus log, which can
//! be hours of traffic from several buses: peak memory scales with the largest
//! data group, so the files this crate most wants to read are the ones it cannot
//! open.
//!
//! [`SignalChunks`] reads the same channel a bounded window of stream at a time.
//! Each chunk is an ordinary [`Signal`] over that window's records, so every
//! decoding path — bit extraction, conversions, invalidation bits,
//! maximum-length sizing, variable-length payloads — is the one `signal()` uses,
//! not a second implementation of it.
//!
//! Windows are bounded by bytes rather than by data block, because block size is
//! the writer's choice: the bus logs this exists for are a single large `DT`
//! block each, and a block-granular reader would hold one entire and bound
//! nothing.
//!
//! ```no_run
//! use falcon_mdf::Mf4File;
//!
//! let file = Mf4File::open("large.mf4")?;
//! let channel = file.find_channel("EngineSpeed").unwrap();
//!
//! let mut peak = f64::MIN;
//! for chunk in file.signal_chunks(channel)? {
//!     for value in chunk?.values_f64()? {
//!         peak = peak.max(value);
//!     }
//! }
//! # Ok::<(), falcon_mdf::error::Mf4Error>(())
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use crate::blocks::ChannelType;
use crate::data_index::DataBlockInfo;
use crate::error::{Mf4Error, Result};
use crate::model::signal::RecordLayout;
use crate::model::vlsd::VlsdPayloads;
use crate::model::{Channel, Signal};
use crate::Mf4File;

/// How a block's bytes relate to the target group's records.
enum Mode {
    /// The data group holds one channel group, so the block payload is a stride
    /// of whole records and needs only slicing.
    Sorted,
    /// Several channel groups share the stream, each record tagged with its
    /// group's ID. The target group's records have to be picked out one at a
    /// time, because a record's size is keyed off its ID and there is no way to
    /// skip a record without having read that ID.
    Unsorted(Demux),
}

/// What the demultiplexer needs to walk an interleaved record stream.
struct Demux {
    rec_id_size: u8,
    /// Record ID to that group's record size and whether it is a VLSD group,
    /// for every group in the data group — not just the target, since every
    /// record has to be measured to find the next one.
    sizes: HashMap<u64, (usize, bool)>,
    /// The target group's record ID.
    target: u64,
    /// Bytes of the target group's record after its ID: channel data plus
    /// invalidation bytes.
    payload: usize,
    /// Record ID of the group holding a variable-length channel's payloads,
    /// when the channel being read is one. Bus loggers write those payloads as
    /// records of their own group, interleaved with the records pointing at
    /// them, so one pass over a block finds both.
    vlsd: Option<u64>,
}

/// Bytes of stream a single chunk reads at most.
///
/// Chunking by data block alone is at the mercy of the writer: the bus logs this
/// exists for are written as one large `DT` block each, so a block-granular
/// reader would hold the whole thing and bound nothing. Uncompressed blocks are
/// therefore read in windows of this size. A compressed block still costs its
/// full inflated size, since deflate cannot be entered part-way.
const CHUNK_BUDGET: usize = 4 << 20;

/// One read: a window of a data block's payload.
struct Segment {
    block: DataBlockInfo,
    /// Offset of this window within the block's payload.
    start: usize,
    len: usize,
    /// Whether this window ends its block, which is where an interleaved
    /// stream's record walk has to stop — see [`Demux::collect`].
    ends_block: bool,
}

/// What one block of an interleaved stream contributed.
struct Demuxed {
    /// The target group's record payloads, concatenated.
    records: Vec<u8>,
    /// Where each variable-length payload sits within the block buffer, as
    /// `(start, length)`. Ranges rather than copies, so the bytes are copied
    /// once when the chunk's index is built rather than twice.
    payloads: Vec<(usize, usize)>,
    /// Bytes the walk got through. What is left is a record cut in half by the
    /// end of the window, to be completed by the next window's bytes.
    consumed: usize,
}

/// A channel's samples, a bounded window of stream at a time.
///
/// Yields a [`Signal`] per window, holding only that window's records. The chunks
/// together cover the same samples, in the same order, as
/// [`Mf4File::signal`](crate::Mf4File::signal) returns in one piece.
///
/// Created by [`Mf4File::signal_chunks`](crate::Mf4File::signal_chunks).
pub struct SignalChunks<'a> {
    file: &'a Mf4File,
    channel: Channel,
    layout: RecordLayout,
    mode: Mode,
    segments: std::vec::IntoIter<Segment>,
    /// Bytes of a record left over from the previous window.
    ///
    /// A window ends where the byte budget runs out, not where a record does,
    /// and the format lets a record straddle a block boundary too. The remainder
    /// is carried forward and completed by the bytes the next window starts with;
    /// dropping it would lose one sample per boundary and shift every sample
    /// after it.
    carry: Vec<u8>,
    /// Samples not yet handed out, so that padding at the end of the stream is
    /// not reported as extra samples.
    remaining: usize,
    /// Running position in the variable-length payload stream, which is what the
    /// records' stored offsets address. Carried across chunks so that a payload
    /// found in the tenth block is numbered as the stream numbers it.
    payload_base: u64,
}

impl Demux {
    /// Copies the target group's record payloads out of one block.
    ///
    /// The walk mirrors the one that built the file's record index when it was
    /// opened, down to where it stops: on an unrecognised record ID, or on a
    /// record that would run past the end of the block. Both mean the stream can
    /// no longer be measured — a record's size is keyed off its ID — and the
    /// index made the same judgement, so stopping here is what keeps a streamed
    /// read and a whole read over the same file in agreement.
    fn collect(&self, data: &[u8]) -> Demuxed {
        let id_size = self.rec_id_size as usize;
        let mut out = Demuxed {
            records: Vec::new(),
            payloads: Vec::new(),
            consumed: 0,
        };
        let mut pos = 0usize;

        while pos < data.len() {
            let Some(rec_id) = read_record_id(data, pos, self.rec_id_size) else {
                break;
            };
            let Some(&(record_size, is_vlsd)) = self.sizes.get(&rec_id) else {
                break;
            };

            let next = if is_vlsd {
                // A VLSD record carries its own length: the ID, then four
                // little-endian bytes, then that many payload bytes.
                let len_at = pos + id_size;
                let Some(bytes) = data.get(len_at..len_at + 4) else {
                    break;
                };
                let payload_len = u32::from_le_bytes(bytes.try_into().unwrap_or_default()) as usize;
                let from = len_at + 4;
                if from + payload_len > data.len() {
                    break;
                }
                if self.vlsd == Some(rec_id) {
                    out.payloads.push((from, payload_len));
                }
                from + payload_len
            } else {
                if record_size == 0 || pos + record_size > data.len() {
                    break;
                }
                pos + record_size
            };

            if rec_id == self.target {
                let start = pos + id_size;
                let Some(slice) = data.get(start..start + self.payload) else {
                    break;
                };
                out.records.extend_from_slice(slice);
            }
            pos = next;
            out.consumed = pos;
        }

        out
    }
}

/// Splits the data blocks into the windows a reader takes them in.
///
/// An uncompressed block is cut into [`CHUNK_BUDGET`]-sized windows, so a single
/// enormous block still reads in bounded memory. A compressed block is one
/// window whatever its size, because inflating part of a deflate stream means
/// inflating everything before it, and windowing would repeat that per window.
fn segments(blocks: &[DataBlockInfo]) -> Vec<Segment> {
    let mut out = Vec::with_capacity(blocks.len());

    for block in blocks {
        let size = block.original_size as usize;
        if block.compression.is_some() || size <= CHUNK_BUDGET {
            out.push(Segment {
                block: block.clone(),
                start: 0,
                len: size.max(1),
                ends_block: true,
            });
            continue;
        }

        let mut start = 0usize;
        while start < size {
            let len = CHUNK_BUDGET.min(size - start);
            out.push(Segment {
                block: block.clone(),
                start,
                len,
                ends_block: start + len >= size,
            });
            start += len;
        }
    }

    out
}

/// Reads a record ID of `rec_id_size` bytes at `pos`.
///
/// A duplicate of the reader `file.rs` uses when it indexes an unsorted group,
/// kept here rather than shared because the two are four lines and making one
/// call the other would mean widening a private helper's visibility across
/// modules for no gain.
fn read_record_id(data: &[u8], pos: usize, rec_id_size: u8) -> Option<u64> {
    let end = pos.checked_add(rec_id_size as usize)?;
    let bytes = data.get(pos..end)?;
    Some(match rec_id_size {
        1 => bytes[0] as u64,
        2 => u16::from_le_bytes(bytes.try_into().ok()?) as u64,
        4 => u32::from_le_bytes(bytes.try_into().ok()?) as u64,
        8 => u64::from_le_bytes(bytes.try_into().ok()?),
        _ => return None,
    })
}

impl std::fmt::Debug for SignalChunks<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignalChunks")
            .field("channel", &self.channel.name)
            .field("reads_left", &self.segments.len())
            .field("samples_left", &self.remaining)
            .finish()
    }
}

impl Iterator for SignalChunks<'_> {
    type Item = Result<Signal>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.remaining > 0 {
            let segment = self.segments.next()?;

            let mut buffer = std::mem::take(&mut self.carry);
            match self
                .file
                .read_block_range(&segment.block, segment.start, segment.len)
            {
                Ok(bytes) => buffer.extend_from_slice(&bytes),
                Err(e) => return Some(Err(e)),
            }

            let (records, whole, payloads) = match &self.mode {
                Mode::Sorted => {
                    let whole = buffer.len() / self.layout.record_size;
                    if whole == 0 {
                        // A block holding less than one record contributes
                        // nothing on its own; its bytes wait for the block that
                        // completes them.
                        self.carry = buffer;
                        continue;
                    }
                    let kept = whole * self.layout.record_size;
                    self.carry = buffer[kept..].to_vec();
                    buffer.truncate(kept);
                    (buffer, whole, None)
                }
                Mode::Unsorted(demux) => {
                    let demuxed = demux.collect(&buffer);
                    let whole = demuxed.records.len() / demux.payload;

                    // A record cut in half by the end of a *window* is completed
                    // by the next window. One cut in half by the end of a
                    // *block* is dropped, because that is what the index built
                    // when the file was opened did, and the two readings of the
                    // same file have to agree.
                    //
                    // The window half of that rule is exercised by the corpus;
                    // the block half is not, because no unsorted file in it
                    // spans more than one block. It mirrors `index_records` by
                    // construction rather than by test — if that walk ever
                    // learns to carry across blocks, this has to follow.
                    self.carry = if segment.ends_block {
                        Vec::new()
                    } else {
                        buffer[demuxed.consumed.min(buffer.len())..].to_vec()
                    };

                    // The payload index has to advance even for a block holding
                    // no record of this group, because the offsets later records
                    // carry are running positions over the whole stream.
                    let payloads = demux.vlsd.map(|_| {
                        let (index, next) = VlsdPayloads::from_located(
                            demuxed
                                .payloads
                                .iter()
                                .filter_map(|&(at, len)| buffer.get(at..at + len)),
                            self.payload_base,
                        );
                        self.payload_base = next;
                        Arc::new(index)
                    });

                    if whole == 0 {
                        // No record of this group in this block. Ordinary in an
                        // interleaved stream, so move on rather than handing
                        // back an empty chunk.
                        continue;
                    }
                    (demuxed.records, whole, payloads)
                }
            };

            let count = whole.min(self.remaining);
            self.remaining -= count;

            return Some(match payloads {
                // A variable-length channel's payloads are this chunk's, not the
                // whole stream's, so they are attached here instead of letting
                // `signal_over` index the entire payload stream.
                Some(payloads) => {
                    let mut signal =
                        Signal::new(self.channel.clone(), Arc::new(records), self.layout, count);
                    signal.attach_payloads(payloads);
                    Ok(signal)
                }
                None => self
                    .file
                    .signal_over(&self.channel, Arc::new(records), self.layout, count),
            });
        }
        None
    }
}

impl Mf4File {
    /// Reads a channel a bounded window of the record stream at a time,
    /// rather than all at once.
    ///
    /// Use this when a group is too large to hold in memory; use
    /// [`Mf4File::signal`] otherwise, since it is simpler and lets the record
    /// cache serve other channels of the same group.
    ///
    /// Both sorted and unsorted data groups are read; an unsorted group's
    /// records are demultiplexed per window rather than through the
    /// whole-file index [`Mf4File::signal`] uses.
    ///
    /// # Errors
    ///
    /// Returns [`Mf4Error::Unsupported`] for **variable-length channels**, whose
    /// payloads live in a second stream outside the records: only the record
    /// half of such a channel could be chunked, which would not bound its
    /// memory. A channel this build cannot decode at all fails the same way it
    /// does through [`Mf4File::signal`].
    pub fn signal_chunks(&self, channel: &Channel) -> Result<SignalChunks<'_>> {
        if let Some(reason) = channel.unreadable() {
            return Err(Mf4Error::Unsupported {
                feature: reason.feature().to_string(),
                detail: format!("reading channel '{}' block by block", channel.name),
            });
        }
        let dg = &self.data_groups()[channel.data_group_index];
        let cg = &dg.channel_groups[channel.channel_group_index];

        // A variable-length channel's payloads live either in a channel group of
        // this same data group — how bus loggers write them, and the form that
        // can be demultiplexed alongside the records — or in a signal-data block
        // of the channel's own, which is a second block chain this reader would
        // have to walk in lockstep with the records. Only the first is streamed.
        let vlsd = if channel.channel_type == ChannelType::VariableLength {
            let link = channel.data_link();
            let group = dg
                .channel_groups
                .iter()
                .find(|other| other.matches_offset(link));
            match group {
                Some(group) => Some(group.record_id()),
                None => {
                    return Err(Mf4Error::Unsupported {
                        feature: "block-by-block reading of a signal-data block".to_string(),
                        detail: format!(
                            "channel '{}' stores its payloads outside the record stream",
                            channel.name
                        ),
                    })
                }
            }
        } else {
            None
        };

        // An interleaved stream is demultiplexed a block at a time; a sorted one
        // is a plain stride. Either way a chunk ends up holding whole records of
        // this group and nothing else, which is what `Signal` decodes over.
        let (mode, layout) = if dg.record_index.is_some() {
            let payload = cg.payload_size();
            if payload == 0 {
                return Err(Mf4Error::parse_error(format!(
                    "channel group '{}' declares a zero-byte record",
                    cg.acquisition_name
                )));
            }
            let demux = Demux {
                rec_id_size: dg.rec_id_size,
                sizes: dg
                    .channel_groups
                    .iter()
                    .map(|other| {
                        (
                            other.record_id(),
                            (other.record_size(dg.rec_id_size), other.is_vlsd()),
                        )
                    })
                    .collect(),
                target: cg.record_id(),
                payload,
                vlsd,
            };
            (
                Mode::Unsorted(demux),
                RecordLayout {
                    // The record ID is stripped as records are gathered, so a
                    // chunk's stride is the payload alone — the same layout the
                    // eager path reports for an unsorted group.
                    record_size: payload,
                    record_offset: 0,
                    inval_start: cg.data_bytes_len(),
                    inval_bytes: cg.inval_bytes_len(),
                },
            )
        } else {
            if vlsd.is_some() {
                // A sorted group's payload group cannot be demultiplexed out of
                // the record stream, because there is no record ID to tell the
                // two apart.
                return Err(Mf4Error::Unsupported {
                    feature: "block-by-block reading of variable-length signal data".to_string(),
                    detail: format!(
                        "channel '{}' is variable-length in a sorted data group",
                        channel.name
                    ),
                });
            }
            let record_size = cg.record_size(dg.rec_id_size);
            if record_size == 0 {
                return Err(Mf4Error::parse_error(format!(
                    "channel group '{}' declares a zero-byte record",
                    cg.acquisition_name
                )));
            }
            (
                Mode::Sorted,
                RecordLayout {
                    record_size,
                    record_offset: dg.rec_id_size as usize,
                    inval_start: cg.data_bytes_len(),
                    inval_bytes: cg.inval_bytes_len(),
                },
            )
        };

        Ok(SignalChunks {
            file: self,
            channel: channel.clone(),
            layout,
            mode,
            segments: segments(dg.data_block_index.blocks()).into_iter(),
            carry: Vec::new(),
            remaining: cg.sample_count as usize,
            payload_base: 0,
        })
    }
}
