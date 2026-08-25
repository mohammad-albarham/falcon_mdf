//! Creation of MF4 files from scratch.
//!
//! The reader side of this crate is complete for 4.11 and was audited block by
//! block against the standard; the writer emits a subset of that same format:
//! one data group per channel group, records sorted by time, and — when the
//! caller hands over validity — the invalidation bits the reader decodes.
//!
//! Within a record a channel is written **in its own type**: an integer of its
//! own width and signedness, a 32- or 64-bit float, a fixed-length string, or a
//! fixed-width byte run. Pushing everything through `f64` would quietly lose
//! the low bits of any integer past 2^53 and would misdescribe the channel to
//! every reader that looks at `cn_data_type`.
//!
//! A channel may also carry a **conversion** — a `##CC` block — so that raw
//! counts read back as the physical quantity they stand for. The types this
//! writer emits are listed on [`WriteGroup::add_channel_typed_with`]; one it
//! cannot emit is refused by name rather than dropped, since a channel written
//! without the conversion it was given would read back as raw counts labelled
//! with a physical unit.
//!
//! A file may be written **compressed**: with
//! [`Mf4Writer::set_compression`] each group's records go out as a `##DZ`
//! deflate block behind the `##HL`/`##DL` pair the standard puts in front of
//! one. The reader side already decodes six zip types; this writes the one
//! every MDF tool understands.
//!
//! Still not written: arrays, VLSD, more than one channel group per data
//! group, and modifying an existing file.
//!
//! Every group carries an implicit `Time` master channel (seconds, float64),
//! so a group written with `n` channels reads back with `n + 1`.

use std::io::Write;
use std::path::Path;

use crate::blocks::conversion::{Conversion, TableEntry};
use crate::error::{Mf4Error, Result};
use crate::model::SignalValues;

const ID_SIZE: u64 = 64;
const HD_SIZE: u64 = 104;
const FH_SIZE: u64 = 56;
const DG_SIZE: u64 = 64;
const CG_SIZE: u64 = 104;
const CN_SIZE: u64 = 160;
const DT_HEADER_SIZE: u64 = 24;
const CC_HEADER_SIZE: u64 = 24;
const HL_SIZE: u64 = 40;
/// One `##DL` with the equal-length flag set and a single data link.
const DL_SIZE: u64 = 56;
const DZ_HEADER_SIZE: u64 = 48;
/// Width of the implicit time master, which is always a float64.
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
    compress: bool,
    groups: Vec<WriteGroup>,
}

/// One channel group: a shared time axis and the channels sampled on it.
#[derive(Debug, Default)]
pub struct WriteGroup {
    times: Vec<f64>,
    channels: Vec<WriteChannel>,
}

/// One channel within a [`WriteGroup`].
#[derive(Debug, Clone)]
pub struct WriteChannel {
    pub(crate) name: String,
    pub(crate) unit: String,
    pub(crate) comment: String,
    pub(crate) values: SignalValues,
    pub(crate) valid: Option<Vec<bool>>,
    pub(crate) conversion: Option<Conversion>,
    pub(crate) format: SampleFormat,
}

impl WriteChannel {
    /// Returns the channel name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sets the channel name.
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    /// Returns the channel engineering unit.
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// Sets the channel engineering unit.
    pub fn set_unit(&mut self, unit: impl Into<String>) {
        self.unit = unit.into();
    }

    /// Returns the channel comment / description.
    pub fn comment(&self) -> &str {
        &self.comment
    }

    /// Sets the channel comment / description.
    pub fn set_comment(&mut self, comment: impl Into<String>) {
        self.comment = comment.into();
    }

    /// Returns the channel's conversion rule, if any.
    pub fn conversion(&self) -> Option<&Conversion> {
        self.conversion.as_ref()
    }

    /// Sets or removes the channel's conversion rule.
    pub fn set_conversion(&mut self, conversion: Option<Conversion>) -> Result<()> {
        if let Some(c) = &conversion {
            if !c.is_identity() {
                CcPlan::of(c, &self.name)?;
            }
        }
        self.conversion = conversion;
        Ok(())
    }

    /// Returns the channel's sample values.
    pub fn values(&self) -> &SignalValues {
        &self.values
    }

    /// Sets the channel's sample values.
    pub fn set_values(&mut self, values: SignalValues) -> Result<()> {
        if values.len() != self.values.len() {
            return Err(Mf4Error::write_error(format!(
                "new values length ({}) does not match current channel length ({})",
                values.len(),
                self.values.len()
            )));
        }
        self.format = SampleFormat::of(&values, &self.name)?;
        self.values = values;
        Ok(())
    }

    /// Returns the channel's validity flags, if any.
    pub fn valid(&self) -> Option<&[bool]> {
        self.valid.as_deref()
    }

    /// Sets or removes the channel's validity flags.
    pub fn set_valid(&mut self, valid: Option<Vec<bool>>) -> Result<()> {
        if let Some(v) = &valid {
            if v.len() != self.values.len() {
                return Err(Mf4Error::write_error(format!(
                    "validity flags length ({}) does not match channel values length ({})",
                    v.len(),
                    self.values.len()
                )));
            }
        }
        self.valid = valid;
        Ok(())
    }

    /// Returns the number of samples in this channel.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns true if this channel has no samples.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// How one channel's samples sit in a record.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SampleFormat {
    /// The `cn_data_type` code the CN block declares.
    pub(crate) data_type: u8,
    /// Bytes each sample occupies.
    pub(crate) width: u32,
}

impl SampleFormat {
    /// Works out how a channel's samples are stored, or refuses a kind this
    /// writer has no record layout for.
    ///
    /// Refusing is the point: writing a complex number or a ragged array as
    /// something else would produce a file that reads back as a measurement it
    /// never was.
    fn of(values: &SignalValues, name: &str) -> Result<Self> {
        // cn_data_type codes: 0 unsigned LE, 2 signed LE, 4 float LE,
        // 7 UTF-8 string, 10 byte array.
        let (data_type, width) = match values {
            SignalValues::U8(_) => (0, 1),
            SignalValues::U16(_) => (0, 2),
            SignalValues::U32(_) => (0, 4),
            SignalValues::U64(_) => (0, 8),
            SignalValues::I8(_) => (2, 1),
            SignalValues::I16(_) => (2, 2),
            SignalValues::I32(_) => (2, 4),
            SignalValues::I64(_) => (2, 8),
            SignalValues::F32(_) => (4, 4),
            SignalValues::F64(_) => (4, 8),
            SignalValues::Str(v) => {
                // A fixed-length string channel is as wide as its longest
                // sample; the rest are padded with NUL, which is how every
                // reader recovers the shorter ones.
                let width = v.iter().map(|s| s.len()).max().unwrap_or(0).max(1);
                (7, u32::try_from(width).map_err(|_| too_wide(name, width))?)
            }
            SignalValues::Bytes { width, .. } => (
                10,
                u32::try_from((*width).max(1)).map_err(|_| too_wide(name, *width))?,
            ),
            other => {
                return Err(Mf4Error::write_error(format!(
                    "channel '{name}' holds {} samples, which this writer has no record \
                     layout for; it writes integers, floats, fixed-length strings and \
                     fixed-width byte runs",
                    other.kind()
                )))
            }
        };
        Ok(SampleFormat { data_type, width })
    }

    /// Appends sample `index`, padded or truncated to exactly `width` bytes.
    fn encode(&self, values: &SignalValues, index: usize, out: &mut Vec<u8>) {
        let width = self.width as usize;
        let before = out.len();
        match values {
            SignalValues::U8(v) => out.push(v[index]),
            SignalValues::U16(v) => out.extend_from_slice(&v[index].to_le_bytes()),
            SignalValues::U32(v) => out.extend_from_slice(&v[index].to_le_bytes()),
            SignalValues::U64(v) => out.extend_from_slice(&v[index].to_le_bytes()),
            SignalValues::I8(v) => out.extend_from_slice(&v[index].to_le_bytes()),
            SignalValues::I16(v) => out.extend_from_slice(&v[index].to_le_bytes()),
            SignalValues::I32(v) => out.extend_from_slice(&v[index].to_le_bytes()),
            SignalValues::I64(v) => out.extend_from_slice(&v[index].to_le_bytes()),
            SignalValues::F32(v) => out.extend_from_slice(&v[index].to_le_bytes()),
            SignalValues::F64(v) => out.extend_from_slice(&v[index].to_le_bytes()),
            SignalValues::Str(v) => out.extend_from_slice(v[index].as_bytes()),
            SignalValues::Bytes { data, width: w } => {
                out.extend_from_slice(&data[index * w..(index + 1) * w])
            }
            // `SampleFormat::of` refused every other kind before this ran.
            _ => {}
        }
        // Strings are the only kind whose samples differ in length; the rest
        // already wrote exactly `width` bytes.
        out.resize(before + width, 0);
    }
}

fn too_wide(name: &str, width: usize) -> Mf4Error {
    Mf4Error::write_error(format!(
        "channel '{name}' has samples of {width} bytes, more than a record can describe"
    ))
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
            compress: false,
            groups: Vec::new(),
        }
    }

    /// A new, empty file with an explicit start time, in nanoseconds since the
    /// Unix epoch. Deterministic output for tests, and for re-exporting a file
    /// under its original timestamp.
    pub fn with_start_time_ns(start_time_ns: i64) -> Self {
        Self {
            start_time_ns,
            compress: false,
            groups: Vec::new(),
        }
    }

    /// Whether to deflate each group's records into a `##DZ` block.
    ///
    /// Off by default. Compression costs write time and makes the file opaque
    /// to anything that reads bytes rather than blocks; it is worth it for the
    /// long, slowly-varying channels a logger produces, and rarely worth it for
    /// a handful of samples.
    pub fn set_compression(&mut self, on: bool) {
        self.compress = on;
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

    /// Returns the start time of the measurement in nanoseconds since the Unix epoch.
    pub fn start_time_ns(&self) -> i64 {
        self.start_time_ns
    }

    /// Sets the start time of the measurement in nanoseconds since the Unix epoch.
    pub fn set_start_time_ns(&mut self, start_time_ns: i64) {
        self.start_time_ns = start_time_ns;
    }

    /// Returns true if data blocks are configured to be deflated.
    pub fn is_compressed(&self) -> bool {
        self.compress
    }

    /// Returns an immutable slice of channel groups in the file.
    pub fn groups(&self) -> &[WriteGroup] {
        &self.groups
    }

    /// Returns a mutable slice of channel groups in the file.
    pub fn groups_mut(&mut self) -> &mut [WriteGroup] {
        &mut self.groups
    }

    /// Returns a reference to the group at `index`, or `None` if out of bounds.
    pub fn group(&self, index: usize) -> Option<&WriteGroup> {
        self.groups.get(index)
    }

    /// Returns a mutable reference to the group at `index`, or `None` if out of bounds.
    pub fn group_mut(&mut self, index: usize) -> Option<&mut WriteGroup> {
        self.groups.get_mut(index)
    }

    /// Removes and returns the group at `index`.
    pub fn remove_group(&mut self, index: usize) -> WriteGroup {
        self.groups.remove(index)
    }

    /// Retains only the groups specified by the predicate.
    pub fn retain_groups<F: FnMut(&WriteGroup) -> bool>(&mut self, mut f: F) {
        self.groups.retain(|g| f(g));
    }

    /// Loads an existing MF4 file into an editable [`Mf4Writer`].
    ///
    /// Every data group and channel group is converted into a [`WriteGroup`].
    /// Channels that can be expressed by the writer (integers, floats, fixed-length strings,
    /// byte arrays, conversions, and invalidation bits) are preserved in their original typed representation.
    ///
    /// Unsupported channel layouts (e.g. CA array compositions, variable-length streams)
    /// or unreadable channels are skipped.
    pub fn from_file(file: &crate::Mf4File) -> Result<Self> {
        let start_time_ns = file.start_time().timestamp_ns;
        let mut writer = Mf4Writer::with_start_time_ns(start_time_ns);

        for dg in file.data_groups() {
            for cg in &dg.channel_groups {
                let master_ch = cg.channels.iter().find(|c| c.is_master());
                let times = if let Some(master) = master_ch {
                    if let Ok(sig) = file.signal(master) {
                        sig.values_f64().unwrap_or_else(|_| {
                            (0..cg.sample_count).map(|i| i as f64).collect()
                        })
                    } else {
                        (0..cg.sample_count).map(|i| i as f64).collect()
                    }
                } else {
                    (0..cg.sample_count).map(|i| i as f64).collect()
                };

                let group = writer.add_group(&times)?;

                for ch in &cg.channels {
                    if ch.is_master() {
                        continue;
                    }
                    if ch.unreadable().is_some() || ch.is_array() {
                        continue;
                    }
                    let Ok(sig) = file.signal(ch) else {
                        continue;
                    };
                    let Ok(raw_vals) = sig.raw_values() else {
                        continue;
                    };
                    if raw_vals.len() != times.len() {
                        continue;
                    }
                    if SampleFormat::of(&raw_vals, &ch.name).is_err() {
                        continue;
                    }
                    let conv = if !ch.conversion.is_identity() {
                        if CcPlan::of(&ch.conversion, &ch.name).is_ok() {
                            Some(ch.conversion.clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let validity = sig.validity();
                    let _ = group.add_channel_full(
                        &ch.name,
                        &ch.unit,
                        &ch.comment,
                        raw_vals,
                        validity.as_deref(),
                        conv,
                    );
                }
            }
        }

        Ok(writer)
    }

    /// Writes the file to `out`.
    pub fn write<W: Write>(&self, out: &mut W) -> Result<()> {
        let fh_text = format!("created by falcon_mdf {}", env!("CARGO_PKG_VERSION"));

        // Built before any offset is assigned: a compressed block's size is
        // not known until it has been compressed, and every link after it
        // depends on that size.
        let payloads: Vec<Payload> = self
            .groups
            .iter()
            .map(|g| Payload::build(g, self.compress))
            .collect::<Result<_>>()?;

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
            .zip(&payloads)
            .map(|(group, payload)| {
                let dg_off = next;
                next += DG_SIZE;
                let cg_off = next;
                next += CG_SIZE;

                let inval_bits = inval_bit_indices(group);
                let offsets = byte_offsets(group);
                let mut channels = Vec::with_capacity(group.channels.len() + 1);
                channels.push(ChannelLayout::master(next));
                next += channels[0].size;
                for (index, channel) in group.channels.iter().enumerate() {
                    let (flags, bit) = match inval_bits[index] {
                        Some(bit) => (CN_FLAG_INVALIDATION_BIT, bit),
                        None => (0, 0),
                    };
                    let layout = ChannelLayout::new(next, offsets[index], channel, flags, bit)?;
                    next += layout.size;
                    channels.push(layout);
                }

                let dt_off = next;
                next += payload.size();
                Ok(GroupLayout {
                    dg_off,
                    cg_off,
                    channels,
                    dt_off,
                })
            })
            .collect::<Result<Vec<_>>>()?;

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
        for (index, ((group, layout), payload)) in self
            .groups
            .iter()
            .zip(&layouts)
            .zip(&payloads)
            .enumerate()
        {
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
                if !channel_layout.comment.is_empty() {
                    write_tx(out, &channel_layout.comment)?;
                }
                if let Some((cc_off, plan)) = &channel_layout.cc {
                    write_cc(out, *cc_off, plan)?;
                    for text in plan.refs.iter().flatten() {
                        write_tx(out, text)?;
                    }
                }
            }
            write_payload(out, layout.dt_off, payload)?;
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
    /// Returns the shared time axis for this group.
    pub fn times(&self) -> &[f64] {
        &self.times
    }

    /// Sets the time axis for this group.
    pub fn set_times(&mut self, times: &[f64]) -> Result<()> {
        if times.iter().any(|t| t.is_nan()) {
            return Err(Mf4Error::write_error(
                "time axis contains NaN, which cannot be ordered",
            ));
        }
        for ch in &self.channels {
            if ch.values.len() != times.len() {
                return Err(Mf4Error::write_error(format!(
                    "channel '{}' has {} values, but new time axis has {}",
                    ch.name,
                    ch.values.len(),
                    times.len()
                )));
            }
        }
        self.times = times.to_vec();
        Ok(())
    }

    /// Returns an immutable slice of channels in this group.
    pub fn channels(&self) -> &[WriteChannel] {
        &self.channels
    }

    /// Returns a mutable slice of channels in this group.
    pub fn channels_mut(&mut self) -> &mut [WriteChannel] {
        &mut self.channels
    }

    /// Finds a channel by name.
    pub fn find_channel(&self, name: &str) -> Option<&WriteChannel> {
        self.channels.iter().find(|c| c.name == name)
    }

    /// Finds a channel by name mutably.
    pub fn find_channel_mut(&mut self, name: &str) -> Option<&mut WriteChannel> {
        self.channels.iter_mut().find(|c| c.name == name)
    }

    /// Removes a channel by index.
    pub fn remove_channel(&mut self, index: usize) -> WriteChannel {
        self.channels.remove(index)
    }

    /// Removes a channel by name, returning it if found.
    pub fn remove_channel_by_name(&mut self, name: &str) -> Option<WriteChannel> {
        let idx = self.channels.iter().position(|c| c.name == name)?;
        Some(self.channels.remove(idx))
    }

    /// Retains only the channels specified by the predicate.
    pub fn retain_channels<F: FnMut(&WriteChannel) -> bool>(&mut self, mut f: F) {
        self.channels.retain(|c| f(c));
    }

    /// Adds a `f64` channel whose samples are all valid. `values` must have one
    /// entry per timestamp of the group.
    pub fn add_channel(&mut self, name: &str, unit: &str, values: &[f64]) -> Result<()> {
        self.add_channel_with_validity(name, unit, values, None)
    }

    /// Adds a `f64` channel together with the per-sample validity the file
    /// should carry: `valid[i] == false` writes the invalidation bit for that
    /// sample, and readers gap it out exactly as they would for a logged file.
    pub fn add_channel_with_validity(
        &mut self,
        name: &str,
        unit: &str,
        values: &[f64],
        valid: Option<&[bool]>,
    ) -> Result<()> {
        self.add_channel_typed_with(
            name,
            unit,
            SignalValues::F64(values.to_vec()),
            valid,
            None,
        )
    }

    /// Adds a channel written in its own type.
    ///
    /// An integer keeps its width and signedness, a float keeps its precision,
    /// and text is written as a fixed-length string as wide as its longest
    /// sample. Nothing is routed through `f64` on the way out.
    pub fn add_channel_typed(
        &mut self,
        name: &str,
        unit: &str,
        values: SignalValues,
    ) -> Result<()> {
        self.add_channel_typed_with(name, unit, values, None, None)
    }

    /// Adds a channel with everything the writer can attach to one: its own
    /// sample type, per-sample validity, and a conversion.
    ///
    /// # Conversions
    ///
    /// A `##CC` block is written for [`Conversion::Linear`],
    /// [`Conversion::Rational`], [`Conversion::Algebraic`],
    /// [`Conversion::TableInterpolated`], [`Conversion::TableLookup`] and
    /// [`Conversion::ValueToText`] whose entries are plain labels. Passing
    /// [`Conversion::None`], or an identity linear rule, writes no block at
    /// all, which is what a channel whose raw values are already physical
    /// should say.
    ///
    /// # Errors
    ///
    /// Refuses, by name, a conversion this writer cannot express — the
    /// remaining table types, a nested conversion inside a text table, and any
    /// rule the reader itself could not evaluate. Writing the channel without
    /// its conversion instead would produce raw counts labelled with a physical
    /// unit, which reads as a plausible wrong measurement rather than a missing
    /// one.
    ///
    /// Refuses a sample kind that has no fixed-width record layout: arrays,
    /// variable-length byte runs, complex numbers and CANopen times.
    pub fn add_channel_typed_with(
        &mut self,
        name: &str,
        unit: &str,
        values: SignalValues,
        valid: Option<&[bool]>,
        conversion: Option<Conversion>,
    ) -> Result<()> {
        self.add_channel_full(name, unit, "", values, valid, conversion)
    }

    /// Adds a channel with all configurable fields: name, unit, comment, values,
    /// validity mask, and conversion rule.
    pub fn add_channel_full(
        &mut self,
        name: &str,
        unit: &str,
        comment: &str,
        values: SignalValues,
        valid: Option<&[bool]>,
        conversion: Option<Conversion>,
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
        let format = SampleFormat::of(&values, name)?;
        let conversion = match conversion {
            Some(c) if !matches!(c, Conversion::None) => {
                CcPlan::of(&c, name)?;
                Some(c)
            }
            _ => None,
        };
        self.channels.push(WriteChannel {
            name: name.to_string(),
            unit: unit.to_string(),
            comment: comment.to_string(),
            values,
            valid,
            conversion,
            format,
        });
        Ok(())
    }
}

/// The `##CC` block a conversion needs: its type code, its `cc_val` numbers,
/// and the texts its `cc_ref` links point at.
#[derive(Debug)]
struct CcPlan {
    cc_type: u8,
    values: Vec<f64>,
    /// One entry per `cc_ref` link; `None` writes a null link.
    refs: Vec<Option<String>>,
}

impl CcPlan {
    /// Plans the block for a conversion, or refuses one this writer cannot
    /// express.
    ///
    /// Returns `None` for a rule that needs no block: a channel whose raw
    /// values are already physical carries a null `cc_conversion` link rather
    /// than an identity block.
    fn of(conversion: &Conversion, name: &str) -> Result<Option<Self>> {
        let refused = |what: &str| {
            Err(Mf4Error::write_error(format!(
                "channel '{name}' carries {what}, which this writer cannot express; \
                 writing the channel without it would return raw counts labelled as \
                 physical values"
            )))
        };
        Ok(match conversion {
            Conversion::None => None,
            Conversion::Linear { offset, factor } if *offset == 0.0 && *factor == 1.0 => None,
            Conversion::Linear { offset, factor } => Some(CcPlan {
                cc_type: 1,
                values: vec![*offset, *factor],
                refs: Vec::new(),
            }),
            Conversion::Rational { coefficients } => Some(CcPlan {
                cc_type: 2,
                values: coefficients.to_vec(),
                refs: Vec::new(),
            }),
            Conversion::Algebraic { formula, .. } => Some(CcPlan {
                cc_type: 3,
                values: Vec::new(),
                refs: vec![Some(formula.clone())],
            }),
            Conversion::TableInterpolated { keys, values }
            | Conversion::TableLookup { keys, values } => {
                if keys.is_empty() || keys.len() != values.len() {
                    return refused("a conversion table with no entries, or with more keys than values");
                }
                let cc_type = if matches!(conversion, Conversion::TableInterpolated { .. }) {
                    4
                } else {
                    5
                };
                // Stored interleaved: key, value, key, value.
                let mut interleaved = Vec::with_capacity(keys.len() * 2);
                for (k, v) in keys.iter().zip(values) {
                    interleaved.push(*k);
                    interleaved.push(*v);
                }
                Some(CcPlan {
                    cc_type,
                    values: interleaved,
                    refs: Vec::new(),
                })
            }
            Conversion::ValueToText {
                keys,
                entries,
                default,
            } => {
                if keys.len() != entries.len() {
                    return refused("a value-to-text table with more keys than entries");
                }
                let mut refs = Vec::with_capacity(entries.len() + 1);
                for entry in entries {
                    match entry {
                        TableEntry::Text(t) => refs.push(Some(t.clone())),
                        TableEntry::Nested(_) => {
                            return refused("a value-to-text table with a nested conversion")
                        }
                    }
                }
                // The trailing reference is the default; a null link means a
                // value that matches no key has no label.
                refs.push(match default {
                    Some(TableEntry::Text(t)) => Some(t.clone()),
                    Some(TableEntry::Nested(_)) => {
                        return refused("a value-to-text table with a nested default")
                    }
                    None => None,
                });
                Some(CcPlan {
                    cc_type: 7,
                    values: keys.clone(),
                    refs,
                })
            }
            Conversion::Unsupported { kind, .. } => {
                return refused(&format!("a conversion of type {kind:?}, which the reader \
                     itself could not evaluate"))
            }
            other => return refused(&format!("a conversion of kind {other:?}")),
        })
    }

    /// Bytes this block occupies, not counting the texts it points at.
    fn size(&self) -> u64 {
        CC_HEADER_SIZE + (4 + self.refs.len() as u64) * 8 + 24 + self.values.len() as u64 * 8
    }
}

/// Where one channel's blocks land, and what its CN block must say about the
/// record layout.
///
/// A channel's blocks are laid out as one contiguous region: the CN, its name
/// and unit texts, then — where there is one — the conversion block and the
/// texts it references. Keeping them together is what lets `size` be the whole
/// of a channel's footprint, so the next channel's offset is just the running
/// total.
#[derive(Debug)]
struct ChannelLayout {
    cn_off: u64,
    size: u64,
    name: String,
    unit: String,
    comment: String,
    byte_offset: u32,
    flags: u32,
    inval_bit_pos: u32,
    data_type: u8,
    bit_count: u32,
    /// The conversion block to emit, and where it lands.
    cc: Option<(u64, CcPlan)>,
}

impl ChannelLayout {
    /// The implicit time master: a float64 at the front of every record.
    fn master(offset: u64) -> Self {
        ChannelLayout {
            cn_off: offset,
            size: CN_SIZE + tx_size("Time") + tx_size("s"),
            name: "Time".to_string(),
            unit: "s".to_string(),
            comment: String::new(),
            byte_offset: 0,
            flags: 0,
            inval_bit_pos: 0,
            data_type: 4,
            bit_count: 64,
            cc: None,
        }
    }

    fn new(
        offset: u64,
        byte_offset: u32,
        channel: &WriteChannel,
        flags: u32,
        inval_bit_pos: u32,
    ) -> Result<Self> {
        let mut size = CN_SIZE + tx_size(&channel.name);
        if !channel.unit.is_empty() {
            size += tx_size(&channel.unit);
        }
        if !channel.comment.is_empty() {
            size += tx_size(&channel.comment);
        }
        let cc = match &channel.conversion {
            None => None,
            Some(c) => CcPlan::of(c, &channel.name)?.map(|plan| {
                let cc_off = offset + size;
                size += plan.size();
                for text in plan.refs.iter().flatten() {
                    size += tx_size(text);
                }
                (cc_off, plan)
            }),
        };
        Ok(ChannelLayout {
            cn_off: offset,
            size,
            name: channel.name.clone(),
            unit: channel.unit.clone(),
            comment: channel.comment.clone(),
            byte_offset,
            flags,
            inval_bit_pos,
            data_type: channel.format.data_type,
            bit_count: channel.format.width * 8,
            cc,
        })
    }

    /// Where this channel's unit text lands, or 0 when it has none.
    fn unit_off(&self) -> u64 {
        if self.unit.is_empty() {
            0
        } else {
            self.cn_off + CN_SIZE + tx_size(&self.name)
        }
    }

    /// Where this channel's comment text lands, or 0 when it has none.
    fn comment_off(&self) -> u64 {
        if self.comment.is_empty() {
            0
        } else {
            let mut off = self.cn_off + CN_SIZE + tx_size(&self.name);
            if !self.unit.is_empty() {
                off += tx_size(&self.unit);
            }
            off
        }
    }

    /// Where this channel's conversion block lands, or 0 when it has none.
    fn cc_off(&self) -> u64 {
        self.cc.as_ref().map(|(off, _)| *off).unwrap_or(0)
    }
}

/// A group's records, laid out and ready to write.
#[derive(Debug)]
enum Payload {
    /// A plain `##DT` block.
    Plain(Vec<u8>),
    /// A `##DZ` deflate block, behind the `##HL`/`##DL` pair.
    Deflated {
        /// Length of the records before compression, which the DZ block must
        /// state so a reader can size its output buffer.
        original_len: u64,
        /// The zlib stream.
        data: Vec<u8>,
    },
}

impl Payload {
    /// Builds a group's records, compressing them when asked.
    fn build(group: &WriteGroup, compress: bool) -> Result<Self> {
        let raw = record_bytes(group);
        if !compress {
            return Ok(Payload::Plain(raw));
        }
        use flate2::write::ZlibEncoder;
        use flate2::Compression;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&raw)?;
        let data = encoder
            .finish()
            .map_err(|e| Mf4Error::Compression(e.to_string()))?;
        Ok(Payload::Deflated {
            original_len: raw.len() as u64,
            data,
        })
    }

    /// Bytes this payload occupies in the file, blocks included.
    fn size(&self) -> u64 {
        match self {
            Payload::Plain(data) => DT_HEADER_SIZE + data.len() as u64,
            Payload::Deflated { data, .. } => {
                HL_SIZE + DL_SIZE + DZ_HEADER_SIZE + data.len() as u64
            }
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

/// Bytes of samples per record: the float64 master, then each channel at its
/// own width. Not every channel is eight bytes any more, so this is a sum
/// rather than a product — reading it as one would place every channel after
/// the first at the wrong offset.
fn data_bytes(group: &WriteGroup) -> u64 {
    SAMPLE_BYTES
        + group
            .channels
            .iter()
            .map(|c| u64::from(c.format.width))
            .sum::<u64>()
}

/// Bytes per record: the samples, then the invalidation area — one bit per
/// channel that carries validity, packed.
fn record_size(group: &WriteGroup) -> u64 {
    data_bytes(group) + u64::from(inval_bytes(group))
}

/// The byte offset of each channel within a record, in channel order, after
/// the master.
fn byte_offsets(group: &WriteGroup) -> Vec<u32> {
    let mut next = SAMPLE_BYTES as u32;
    group
        .channels
        .iter()
        .map(|c| {
            let at = next;
            next += c.format.width;
            at
        })
        .collect()
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
    buf.extend_from_slice(&(data_bytes(group) as u32).to_le_bytes());
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
    push_link(&mut buf, layout.cc_off()); // cc_conversion, 0 when already physical
    push_link(&mut buf, 0); // cn_data
    push_link(&mut buf, layout.unit_off());
    push_link(&mut buf, layout.comment_off());

    if is_master {
        buf.push(2); // channel_type: master
        buf.push(1); // sync_type: time in seconds
    } else {
        buf.push(0); // channel_type: fixed length
        buf.push(0); // sync_type: none
    }
    buf.push(layout.data_type);
    buf.push(0); // bit offset within the byte: samples are byte-aligned
    buf.extend_from_slice(&layout.byte_offset.to_le_bytes());
    buf.extend_from_slice(&layout.bit_count.to_le_bytes());
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

/// Writes one CC block, followed by nothing — the texts it links to are
/// emitted by the caller, directly after it, in `refs` order.
fn write_cc(out: &mut impl Write, cc_off: u64, plan: &CcPlan) -> Result<()> {
    let length = plan.size();
    let mut buf = Vec::with_capacity(length as usize);
    block_header(&mut buf, b"##CC", length, 4 + plan.refs.len() as u64);
    push_link(&mut buf, 0); // tx_name
    push_link(&mut buf, 0); // md_unit: the channel carries the unit
    push_link(&mut buf, 0); // md_comment
    push_link(&mut buf, 0); // cc_inverse

    // Each referenced text sits after the block, one after another, so its
    // offset is the running total of the ones before it.
    let mut text_off = cc_off + CC_HEADER_SIZE + (4 + plan.refs.len() as u64) * 8
        + 24
        + plan.values.len() as u64 * 8;
    for text in &plan.refs {
        match text {
            Some(t) => {
                push_link(&mut buf, text_off);
                text_off += tx_size(t);
            }
            None => push_link(&mut buf, 0),
        }
    }

    buf.push(plan.cc_type);
    buf.push(0); // precision, not stated
    buf.extend_from_slice(&0u16.to_le_bytes()); // flags: no physical range
    buf.extend_from_slice(&(plan.refs.len() as u16).to_le_bytes());
    buf.extend_from_slice(&(plan.values.len() as u16).to_le_bytes());
    buf.extend_from_slice(&0f64.to_le_bytes()); // phy_range_min
    buf.extend_from_slice(&0f64.to_le_bytes()); // phy_range_max
    for v in &plan.values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    debug_assert_eq!(buf.len() as u64, length, "CC block size must match its header");
    out.write_all(&buf)?;
    Ok(())
}

/// One group's records, sorted by time, each channel in its own type, with the
/// invalidation area appended.
fn record_bytes(group: &WriteGroup) -> Vec<u8> {
    let size = record_size(group);
    let mut buf = Vec::with_capacity(group.times.len() * size as usize);

    let order = sorted_order(&group.times);
    let inval_bits = inval_bit_indices(group);
    let inval_len = inval_bytes(group) as usize;
    let mut inval = vec![0u8; inval_len];
    for index in order {
        buf.extend_from_slice(&group.times[index].to_le_bytes());
        for channel in &group.channels {
            channel.format.encode(&channel.values, index, &mut buf);
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
    buf
}

/// Writes a group's records, as a plain `##DT` or as `##HL`/`##DL`/`##DZ`.
///
/// `at` is where the first of those blocks lands, which the DL needs in order
/// to link to the DZ that follows it.
fn write_payload(out: &mut impl Write, at: u64, payload: &Payload) -> Result<()> {
    match payload {
        Payload::Plain(data) => {
            let mut buf = Vec::with_capacity(DT_HEADER_SIZE as usize);
            block_header(&mut buf, b"##DT", DT_HEADER_SIZE + data.len() as u64, 0);
            out.write_all(&buf)?;
            out.write_all(data)?;
        }
        Payload::Deflated { original_len, data } => {
            let dl_off = at + HL_SIZE;
            let dz_off = dl_off + DL_SIZE;

            // ##HL: says up front which zip type the list below uses, so a
            // reader need not open a DZ to find out.
            let mut buf = Vec::with_capacity(HL_SIZE as usize);
            block_header(&mut buf, b"##HL", HL_SIZE, 1);
            push_link(&mut buf, dl_off);
            buf.extend_from_slice(&0u16.to_le_bytes()); // hl_flags
            buf.push(0); // hl_zip_type: deflate
            buf.extend_from_slice(&[0u8; 5]); // reserved
            out.write_all(&buf)?;

            // ##DL: one entry, so the equal-length form is the shorter one and
            // its length is the whole of the uncompressed data.
            let mut buf = Vec::with_capacity(DL_SIZE as usize);
            block_header(&mut buf, b"##DL", DL_SIZE, 2);
            push_link(&mut buf, 0); // dl_dl_next
            push_link(&mut buf, dz_off);
            buf.push(0x01); // dl_flags: equal length
            buf.extend_from_slice(&[0u8; 3]); // reserved
            buf.extend_from_slice(&1u32.to_le_bytes()); // dl_count
            buf.extend_from_slice(&original_len.to_le_bytes());
            out.write_all(&buf)?;

            // ##DZ.
            let mut buf = Vec::with_capacity(DZ_HEADER_SIZE as usize);
            block_header(
                &mut buf,
                b"##DZ",
                DZ_HEADER_SIZE + data.len() as u64,
                0,
            );
            buf.extend_from_slice(b"DT"); // the block type this stands in for
            buf.push(0); // dz_zip_type: deflate
            buf.push(0); // reserved
            buf.extend_from_slice(&0u32.to_le_bytes()); // dz_zip_parameter
            buf.extend_from_slice(&original_len.to_le_bytes());
            buf.extend_from_slice(&(data.len() as u64).to_le_bytes());
            out.write_all(&buf)?;
            out.write_all(data)?;
        }
    }
    Ok(())
}
