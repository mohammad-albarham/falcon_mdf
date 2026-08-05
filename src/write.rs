//! Creation of MF4 files from scratch.
//!
//! The reader side of this crate is complete for 4.11 and was audited block by
//! block against the standard; the writer emits a deliberately simple subset
//! of that same format: one data group per channel group, records sorted by
//! time, raw little-endian 64-bit floats, and — when the caller hands over
//! validity — the invalidation bits the reader decodes. No conversions, no
//! arrays, no VLSD, no compression: raw floats are what this crate exports,
//! and a file of raw floats is one every reader can understand.
//!
//! Every group carries an implicit `Time` master channel (seconds, float64),
//! so a group written with `n` channels reads back with `n + 1`.

use std::io::Write;
use std::path::Path;

use crate::error::{Mf4Error, Result};

const ID_SIZE: u64 = 64;
const HD_SIZE: u64 = 104;
const FH_SIZE: u64 = 56;
const DG_SIZE: u64 = 64;
const CG_SIZE: u64 = 104;
const CN_SIZE: u64 = 160;
const DT_HEADER_SIZE: u64 = 24;
const SAMPLE_BYTES: u64 = 8;

/// Flag bit 1 of `cn_flags`: the channel has an invalidation bit.
const CN_FLAG_INVALIDATION_BIT: u32 = 0x0002;

/// An MF4 file under construction.
///
/// Groups are written in the order they were added; each becomes its own data
/// group with one channel group, its records sorted by time.
#[derive(Debug, Default)]
pub struct Mf4Writer {
    start_time_ns: i64,
    groups: Vec<WriteGroup>,
}

/// One channel group: a shared time axis and the channels sampled on it.
#[derive(Debug, Default)]
pub struct WriteGroup {
    times: Vec<f64>,
    channels: Vec<WriteChannel>,
}

#[derive(Debug)]
struct WriteChannel {
    name: String,
    unit: String,
    values: Vec<f64>,
    valid: Option<Vec<bool>>,
}

impl Mf4Writer {
    /// A new, empty file, stamped with the current time as its start time.
    pub fn new() -> Self {
        let start_time_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        Self {
            start_time_ns,
            groups: Vec::new(),
        }
    }

    /// A new, empty file with an explicit start time, in nanoseconds since the
    /// Unix epoch. Deterministic output for tests, and for re-exporting a file
    /// under its original timestamp.
    pub fn with_start_time_ns(start_time_ns: i64) -> Self {
        Self {
            start_time_ns,
            groups: Vec::new(),
        }
    }

    /// Adds a channel group sampled at `times` (seconds). The records are
    /// sorted by time on write, so `times` need not be ordered — but every
    /// timestamp must be orderable, and NaN is not.
    pub fn add_group(&mut self, times: &[f64]) -> Result<&mut WriteGroup> {
        if times.iter().any(|t| t.is_nan()) {
            return Err(Mf4Error::write_error(
                "a group's time axis contains NaN, which has no place in the order records are sorted by",
            ));
        }
        self.groups.push(WriteGroup {
            times: times.to_vec(),
            channels: Vec::new(),
        });
        Ok(self.groups.last_mut().expect("just pushed"))
    }

    /// Writes the file to `out`.
    pub fn write<W: Write>(&self, out: &mut W) -> Result<()> {
        let fh_text = format!("created by falcon_mdf {}", env!("CARGO_PKG_VERSION"));

        // Pass 1: every block's offset, so links can point forward. Blocks are
        // laid out in emission order: ID, HD, FH, its text, then per group the
        // DG, the CG, one CN (plus name and unit texts) per channel, and the
        // DT. The master channel is implicit and comes first in every group.
        // The HD block is found by position (offset 64), so nothing links to
        // it and it needs no offset variable.
        let mut next = ID_SIZE + HD_SIZE;
        let fh_off = next;
        next += FH_SIZE;
        let fh_tx_off = next;
        next += tx_size(&fh_text);

        let layouts: Vec<GroupLayout> = self
            .groups
            .iter()
            .map(|group| {
                let dg_off = next;
                next += DG_SIZE;
                let cg_off = next;
                next += CG_SIZE;

                let inval_bits = inval_bit_indices(group);
                let mut channels = Vec::with_capacity(group.channels.len() + 1);
                channels.push(ChannelLayout::new(next, 0, "Time", "s", 0, 0));
                next += channels[0].size;
                for (index, channel) in group.channels.iter().enumerate() {
                    let (flags, bit) = match inval_bits[index] {
                        Some(bit) => (CN_FLAG_INVALIDATION_BIT, bit),
                        None => (0, 0),
                    };
                    channels.push(ChannelLayout::new(
                        next,
                        SAMPLE_BYTES as u32 * (index as u32 + 1),
                        &channel.name,
                        &channel.unit,
                        flags,
                        bit,
                    ));
                    next += channels.last().expect("just pushed").size;
                }

                let dt_off = next;
                next += DT_HEADER_SIZE + group.times.len() as u64 * record_size(group);
                GroupLayout {
                    dg_off,
                    cg_off,
                    channels,
                    dt_off,
                }
            })
            .collect();

        // Pass 2: emit, in the same order the offsets were assigned.
        write_id(out)?;
        write_hd(
            out,
            layouts.first().map(|l| l.dg_off).unwrap_or(0),
            fh_off,
            self.start_time_ns,
        )?;
        write_fh(out, fh_tx_off, self.start_time_ns)?;
        write_tx(out, &fh_text)?;
        for (index, (group, layout)) in self.groups.iter().zip(&layouts).enumerate() {
            let dg_next = layouts.get(index + 1).map(|l| l.dg_off).unwrap_or(0);
            write_dg(out, dg_next, layout.cg_off, layout.dt_off)?;
            write_cg(out, group, layout.channels[0].cn_off)?;
            for (channel_index, channel_layout) in layout.channels.iter().enumerate() {
                let cn_next = layout
                    .channels
                    .get(channel_index + 1)
                    .map(|c| c.cn_off)
                    .unwrap_or(0);
                write_cn(out, channel_index == 0, channel_layout, cn_next)?;
                write_tx(out, &channel_layout.name)?;
                if !channel_layout.unit.is_empty() {
                    write_tx(out, &channel_layout.unit)?;
                }
            }
            write_dt(out, group)?;
        }
        Ok(())
    }

    /// Writes the file to a path, creating or truncating it.
    pub fn write_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let file = std::fs::File::create(path)?;
        let mut out = std::io::BufWriter::new(file);
        self.write(&mut out)?;
        out.flush()?;
        Ok(())
    }
}

impl WriteGroup {
    /// Adds a channel whose samples are all valid. `values` must have one
    /// entry per timestamp of the group.
    pub fn add_channel(&mut self, name: &str, unit: &str, values: &[f64]) -> Result<()> {
        self.add_channel_with_validity(name, unit, values, None)
    }

    /// Adds a channel together with the per-sample validity the file should
    /// carry: `valid[i] == false` writes the invalidation bit for that sample,
    /// and readers gap it out exactly as they would for a logged file.
    pub fn add_channel_with_validity(
        &mut self,
        name: &str,
        unit: &str,
        values: &[f64],
        valid: Option<&[bool]>,
    ) -> Result<()> {
        if values.len() != self.times.len() {
            return Err(Mf4Error::write_error(format!(
                "channel '{name}' has {} values but the group's time axis has {}",
                values.len(),
                self.times.len()
            )));
        }
        let valid = match valid {
            None => None,
            Some(valid) => {
                if valid.len() != self.times.len() {
                    return Err(Mf4Error::write_error(format!(
                        "channel '{name}' has {} validity flags but the group's time axis has {}",
                        valid.len(),
                        self.times.len()
                    )));
                }
                Some(valid.to_vec())
            }
        };
        self.channels.push(WriteChannel {
            name: name.to_string(),
            unit: unit.to_string(),
            values: values.to_vec(),
            valid,
        });
        Ok(())
    }
}

/// Where one channel's blocks land, and what its CN block must say about the
/// record layout.
#[derive(Debug)]
struct ChannelLayout {
    cn_off: u64,
    size: u64,
    name: String,
    unit: String,
    byte_offset: u32,
    flags: u32,
    inval_bit_pos: u32,
}

impl ChannelLayout {
    fn new(
        offset: u64,
        byte_offset: u32,
        name: &str,
        unit: &str,
        flags: u32,
        inval_bit_pos: u32,
    ) -> Self {
        let mut size = CN_SIZE + tx_size(name);
        if !unit.is_empty() {
            size += tx_size(unit);
        }
        ChannelLayout {
            cn_off: offset,
            size,
            name: name.to_string(),
            unit: unit.to_string(),
            byte_offset,
            flags,
            inval_bit_pos,
        }
    }
}

#[derive(Debug)]
struct GroupLayout {
    dg_off: u64,
    cg_off: u64,
    channels: Vec<ChannelLayout>,
    dt_off: u64,
}

/// The sample order for a group: indices into `times`, sorted. `total_cmp`
/// needs no NaN special case — `add_group` already refused NaN timestamps.
fn sorted_order(times: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..times.len()).collect();
    order.sort_by(|&a, &b| times[a].total_cmp(&times[b]));
    order
}

/// Bytes per record: one float64 per channel (master included), then the
/// invalidation area — one bit per channel that carries validity, packed.
fn record_size(group: &WriteGroup) -> u64 {
    SAMPLE_BYTES * (1 + group.channels.len() as u64) + u64::from(inval_bytes(group))
}

fn inval_bytes(group: &WriteGroup) -> u32 {
    let bits = group.channels.iter().filter(|c| c.valid.is_some()).count();
    bits.div_ceil(8) as u32
}

/// The invalidation bit index of each value channel, in channel order; `None`
/// for channels written as wholly valid.
fn inval_bit_indices(group: &WriteGroup) -> Vec<Option<u32>> {
    let mut next_bit = 0u32;
    group
        .channels
        .iter()
        .map(|c| {
            if c.valid.is_some() {
                let bit = next_bit;
                next_bit += 1;
                Some(bit)
            } else {
                None
            }
        })
        .collect()
}

fn tx_size(text: &str) -> u64 {
    24 + text.len() as u64 + 1
}

fn block_header(buf: &mut Vec<u8>, id: &[u8; 4], length: u64, link_count: u64) {
    buf.extend_from_slice(id);
    buf.extend_from_slice(&[0u8; 4]);
    buf.extend_from_slice(&length.to_le_bytes());
    buf.extend_from_slice(&link_count.to_le_bytes());
}

fn push_link(buf: &mut Vec<u8>, target: u64) {
    buf.extend_from_slice(&target.to_le_bytes());
}

fn write_id(out: &mut impl Write) -> Result<()> {
    let mut buf = vec![0u8; ID_SIZE as usize];
    buf[0..8].copy_from_slice(b"MDF     ");
    buf[8..16].copy_from_slice(b"4.11    ");
    buf[16..24].copy_from_slice(b"falcon  ");
    buf[28..30].copy_from_slice(&411u16.to_le_bytes());
    out.write_all(&buf)?;
    Ok(())
}

fn write_hd(out: &mut impl Write, dg_first: u64, fh_first: u64, start_time_ns: i64) -> Result<()> {
    let mut buf = Vec::with_capacity(HD_SIZE as usize);
    block_header(&mut buf, b"##HD", HD_SIZE, 6);
    push_link(&mut buf, dg_first);
    push_link(&mut buf, fh_first);
    push_link(&mut buf, 0); // ch_first
    push_link(&mut buf, 0); // at_first
    push_link(&mut buf, 0); // ev_first
    push_link(&mut buf, 0); // md_comment
    buf.extend_from_slice(&start_time_ns.to_le_bytes());
    buf.extend_from_slice(&0i16.to_le_bytes()); // tz offset
    buf.extend_from_slice(&0i16.to_le_bytes()); // dst offset
    buf.push(0); // time class: local PC time
    buf.push(0); // flags
    buf.push(0); // reserved
    buf.push(0); // reserved
    buf.extend_from_slice(&0f64.to_le_bytes()); // start angle
    buf.extend_from_slice(&0f64.to_le_bytes()); // start distance
    out.write_all(&buf)?;
    Ok(())
}

fn write_fh(out: &mut impl Write, md_comment: u64, time_ns: i64) -> Result<()> {
    let mut buf = Vec::with_capacity(FH_SIZE as usize);
    block_header(&mut buf, b"##FH", FH_SIZE, 2);
    push_link(&mut buf, 0); // fh_next
    push_link(&mut buf, md_comment);
    buf.extend_from_slice(&(time_ns.max(0) as u64).to_le_bytes());
    buf.extend_from_slice(&0i16.to_le_bytes());
    buf.extend_from_slice(&0i16.to_le_bytes());
    buf.push(0); // time flags
    buf.extend_from_slice(&[0u8; 3]); // reserved
    out.write_all(&buf)?;
    Ok(())
}

fn write_tx(out: &mut impl Write, text: &str) -> Result<()> {
    let length = tx_size(text);
    let mut buf = Vec::with_capacity(length as usize);
    block_header(&mut buf, b"##TX", length, 0);
    buf.extend_from_slice(text.as_bytes());
    buf.push(0);
    out.write_all(&buf)?;
    Ok(())
}

fn write_dg(out: &mut impl Write, dg_next: u64, cg_first: u64, data: u64) -> Result<()> {
    let mut buf = Vec::with_capacity(DG_SIZE as usize);
    block_header(&mut buf, b"##DG", DG_SIZE, 4);
    push_link(&mut buf, dg_next);
    push_link(&mut buf, cg_first);
    push_link(&mut buf, data);
    push_link(&mut buf, 0); // md_comment
    buf.push(0); // rec_id_size: one channel group per data group
    buf.extend_from_slice(&[0u8; 7]); // reserved
    out.write_all(&buf)?;
    Ok(())
}

fn write_cg(out: &mut impl Write, group: &WriteGroup, cn_first: u64) -> Result<()> {
    let mut buf = Vec::with_capacity(CG_SIZE as usize);
    block_header(&mut buf, b"##CG", CG_SIZE, 6);
    push_link(&mut buf, 0); // cg_next
    push_link(&mut buf, cn_first);
    push_link(&mut buf, 0); // tx_acq_name
    push_link(&mut buf, 0); // si_acq_source
    push_link(&mut buf, 0); // sr_first
    push_link(&mut buf, 0); // md_comment
    push_link(&mut buf, 0); // record_id
    buf.extend_from_slice(&(group.times.len() as u64).to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // flags
    buf.extend_from_slice(&0u16.to_le_bytes()); // path separator
    buf.extend_from_slice(&[0u8; 4]); // reserved
    let data_bytes = (group.channels.len() as u64 + 1) * SAMPLE_BYTES;
    buf.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    buf.extend_from_slice(&inval_bytes(group).to_le_bytes());
    out.write_all(&buf)?;
    Ok(())
}

/// Writes one CN block. `is_master` marks the implicit time channel.
fn write_cn(
    out: &mut impl Write,
    is_master: bool,
    layout: &ChannelLayout,
    cn_next: u64,
) -> Result<()> {
    let mut buf = Vec::with_capacity(CN_SIZE as usize);
    block_header(&mut buf, b"##CN", CN_SIZE, 8);
    push_link(&mut buf, cn_next);
    push_link(&mut buf, 0); // composition
    push_link(&mut buf, layout.cn_off + CN_SIZE); // tx_name
    push_link(&mut buf, 0); // si_source
    push_link(&mut buf, 0); // cc_conversion: raw values are the physical values
    push_link(&mut buf, 0); // cn_data
    let unit_off = if layout.unit.is_empty() {
        0
    } else {
        layout.cn_off + CN_SIZE + tx_size(&layout.name)
    };
    push_link(&mut buf, unit_off);
    push_link(&mut buf, 0); // md_comment

    if is_master {
        buf.push(2); // channel_type: master
        buf.push(1); // sync_type: time in seconds
    } else {
        buf.push(0); // channel_type: fixed length
        buf.push(0); // sync_type: none
    }
    buf.push(4); // data_type: float64 little-endian
    buf.push(0); // bit offset within the byte
    buf.extend_from_slice(&layout.byte_offset.to_le_bytes());
    buf.extend_from_slice(&64u32.to_le_bytes()); // bit count
    buf.extend_from_slice(&layout.flags.to_le_bytes());
    buf.extend_from_slice(&layout.inval_bit_pos.to_le_bytes());
    buf.push(0); // precision
    buf.push(0); // reserved
    buf.extend_from_slice(&0u16.to_le_bytes()); // attachment count
    for _ in 0..6 {
        buf.extend_from_slice(&0f64.to_le_bytes());
    }
    out.write_all(&buf)?;
    Ok(())
}

fn write_dt(out: &mut impl Write, group: &WriteGroup) -> Result<()> {
    let size = record_size(group);
    let length = DT_HEADER_SIZE + group.times.len() as u64 * size;
    let mut buf = Vec::with_capacity(length as usize);
    block_header(&mut buf, b"##DT", length, 0);

    let order = sorted_order(&group.times);
    let inval_bits = inval_bit_indices(group);
    let inval_len = inval_bytes(group) as usize;
    let mut inval = vec![0u8; inval_len];
    for index in order {
        buf.extend_from_slice(&group.times[index].to_le_bytes());
        for channel in &group.channels {
            buf.extend_from_slice(&channel.values[index].to_le_bytes());
        }
        if inval_len > 0 {
            inval.fill(0);
            for (channel, bit) in group.channels.iter().zip(&inval_bits) {
                let Some(bit) = bit else { continue };
                let valid = channel.valid.as_ref().expect("a bit implies validity");
                if !valid[index] {
                    // Bit set means invalid; readers report the sample valid
                    // when the bit is clear.
                    inval[(bit / 8) as usize] |= 1 << (bit % 8);
                }
            }
            buf.extend_from_slice(&inval);
        }
    }
    out.write_all(&buf)?;
    Ok(())
}
