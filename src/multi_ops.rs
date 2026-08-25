//! Operations that span several channels or several measurements:
//! [`Mf4File::filter`](crate::Mf4File::filter), [`concatenate`] and [`stack`].
//!
//! All three hand back decoded [`SignalSeries`] in memory. None of them writes
//! a file: a concatenated or stacked measurement has a structure our writer
//! cannot express, and producing a half-correct file would be worse than
//! producing none.
//!
//! # How the time axes are lined up
//!
//! An MF4 file records absolute time in its header and relative time in its
//! master channel, so two files recorded an hour apart both start their master
//! at (or near) zero. Combining them without looking at the headers would pile
//! both recordings on top of each other at t = 0. [`TimeAlignment::StartTime`],
//! the default, therefore shifts every file by its header start time relative
//! to the earliest file's — the same rule asammdf's `sync=True` applies in
//! `MDF.concatenate` and `MDF.stack`. [`TimeAlignment::AsRecorded`] is the
//! escape hatch for files whose headers are not trustworthy.

use crate::error::{Mf4Error, Result};
use crate::file::Mf4File;
use crate::model::{Channel, SignalValues};
use crate::time_ops::SignalSeries;

/// How a channel is picked out of a file by [`Mf4File::filter`].
///
/// `ChannelSelector::from("Speed")` is the common case; the other two variants
/// exist for names that several channels share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelSelector {
    /// The channel with this name, which must be unique in the file.
    ///
    /// A name carried by more than one channel is an error rather than a
    /// silent pick of the first: a master channel called `t` exists in every
    /// group, and quietly filtering the wrong group's copy of it is precisely
    /// the kind of plausible-but-wrong answer this crate tries not to give.
    /// Disambiguate with [`ChannelSelector::NameInGroup`] or
    /// [`ChannelSelector::Position`].
    Name(String),
    /// The channel with this name inside one specific channel group.
    NameInGroup {
        /// Channel name.
        name: String,
        /// Index of the data group.
        data_group: usize,
        /// Index of the channel group within that data group.
        channel_group: usize,
    },
    /// The channel at an exact position, regardless of its name.
    Position {
        /// Index of the data group.
        data_group: usize,
        /// Index of the channel group within that data group.
        channel_group: usize,
        /// Index of the channel within that channel group.
        index: usize,
    },
}

impl From<&str> for ChannelSelector {
    fn from(name: &str) -> Self {
        ChannelSelector::Name(name.to_string())
    }
}

impl From<String> for ChannelSelector {
    fn from(name: String) -> Self {
        ChannelSelector::Name(name)
    }
}

impl ChannelSelector {
    /// Resolves this selector against a file.
    pub(crate) fn resolve<'a>(&self, file: &'a Mf4File) -> Result<&'a Channel> {
        match self {
            ChannelSelector::Name(name) => {
                let found = file.find_channels(name);
                match found.len() {
                    0 => Err(Mf4Error::ChannelNotFound { name: name.clone() }),
                    1 => Ok(found[0]),
                    n => Err(Mf4Error::parse_error(format!(
                        "channel name '{name}' is carried by {n} channels; select it with \
                         ChannelSelector::NameInGroup or ChannelSelector::Position"
                    ))),
                }
            }
            ChannelSelector::NameInGroup {
                name,
                data_group,
                channel_group,
            } => {
                let group = group_at(file, *data_group, *channel_group)?;
                group
                    .find_channel(name)
                    .ok_or_else(|| Mf4Error::parse_error(format!(
                        "channel '{name}' not found in data group {data_group}, channel group \
                         {channel_group}"
                    )))
            }
            ChannelSelector::Position {
                data_group,
                channel_group,
                index,
            } => {
                let group = group_at(file, *data_group, *channel_group)?;
                group.channels.get(*index).ok_or_else(|| {
                    Mf4Error::parse_error(format!(
                        "channel index {index} is out of range for data group {data_group}, \
                         channel group {channel_group}, which has {} channels",
                        group.channels.len()
                    ))
                })
            }
        }
    }
}

fn group_at(
    file: &Mf4File,
    data_group: usize,
    channel_group: usize,
) -> Result<&crate::model::ChannelGroup> {
    let dg = file.data_groups().get(data_group).ok_or_else(|| {
        Mf4Error::parse_error(format!(
            "data group {data_group} is out of range; the file has {}",
            file.data_groups().len()
        ))
    })?;
    dg.channel_groups.get(channel_group).ok_or_else(|| {
        Mf4Error::parse_error(format!(
            "channel group {channel_group} is out of range for data group {data_group}, which \
             has {}",
            dg.channel_groups.len()
        ))
    })
}

/// How the time axes of several measurements are lined up before combining
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeAlignment {
    /// Shift every file by its header start time relative to the earliest
    /// file's, so a later recording lands after an earlier one.
    ///
    /// Equivalent to asammdf's `sync=True`, which is its default.
    #[default]
    StartTime,
    /// Use each file's master channel exactly as recorded, applying no shift.
    ///
    /// Equivalent to asammdf's `sync=False`.
    AsRecorded,
}

/// One series produced by [`stack`], tagged with the file it came from.
///
/// Stacking keeps same-named channels from different files apart, so the name
/// alone does not identify a series; `file_index` does.
#[derive(Debug, Clone, PartialEq)]
pub struct StackedSeries {
    /// Position of the source file in the slice handed to [`stack`].
    pub file_index: usize,
    /// The decoded series, with its timestamps already shifted.
    pub series: SignalSeries,
}

/// Joins several measurements end to end along time.
///
/// The files must have the same internal structure: the same number of channel
/// groups, and the same set of channel names in each. A channel present in one
/// file and absent from another is an error, not a gap to be filled — the two
/// files describe different measurements, and inventing samples for the
/// missing one would fabricate data. This is what asammdf's `MDF.concatenate`
/// does too.
///
/// Channels sharing a name across files are joined into one series: that is
/// the point of concatenating. Their order within a group may differ between
/// files; they are matched by name, and the result follows the first file's
/// order.
///
/// # Time
///
/// Each file is shifted by its header start time relative to the earliest
/// file's (see [`TimeAlignment`]). If, after that shift, a file would still
/// start at or before the previous file's last timestamp, it is pushed forward
/// to begin one sample interval after it — the result is monotonic, and no two
/// files overlap. The sample interval used is the second minus the first
/// timestamp of the file being appended, or 1 ms when it has fewer than two
/// samples. asammdf applies the identical rule.
///
/// # Returns
///
/// One [`SignalSeries`] per non-master channel, in the first file's group and
/// channel order. The channel metadata is the first file's.
///
/// # Example
///
/// ```no_run
/// # use falcon_mdf::{Mf4File, TimeAlignment, multi_ops};
/// let a = Mf4File::open("run1.mf4")?;
/// let b = Mf4File::open("run2.mf4")?;
/// let joined = multi_ops::concatenate(&[&a, &b], TimeAlignment::StartTime)?;
/// println!("{} samples", joined[0].len());
/// # Ok::<(), falcon_mdf::error::Mf4Error>(())
/// ```
pub fn concatenate(files: &[&Mf4File], alignment: TimeAlignment) -> Result<Vec<SignalSeries>> {
    if files.is_empty() {
        return Err(Mf4Error::parse_error("concatenate needs at least one file"));
    }

    let offsets = start_time_offsets(files, alignment);
    let layouts: Vec<Vec<GroupLayout<'_>>> = files.iter().map(|f| group_layouts(f)).collect();
    check_same_structure(&layouts)?;

    let mut out: Vec<SignalSeries> = Vec::new();

    for (group_index, first_group) in layouts[0].iter().enumerate() {
        if first_group.channels.is_empty() {
            continue;
        }

        // One accumulator per channel of this group, in the first file's order.
        let mut timestamps: Vec<Vec<f64>> = vec![Vec::new(); first_group.channels.len()];
        let mut values: Vec<Option<SignalValues>> = vec![None; first_group.channels.len()];
        let mut validity: Vec<Option<Vec<bool>>> = vec![None; first_group.channels.len()];
        let mut counts: Vec<usize> = vec![0; first_group.channels.len()];
        let mut last_timestamp: Option<f64> = None;

        for (file_index, file) in files.iter().enumerate() {
            let group = &layouts[file_index][group_index];
            // Matched by name, so a file that lists the same channels in a
            // different order still lines up. Matches are consumed: a group
            // holding two channels of the same name would otherwise resolve
            // both of the first file's slots to the same channel of the
            // second, silently duplicating one and dropping the other.
            let mut taken = vec![false; group.channels.len()];
            let selectors: Vec<&Channel> = first_group
                .channels
                .iter()
                .map(|ch| {
                    let at = group
                        .channels
                        .iter()
                        .enumerate()
                        .find(|(i, other)| other.name == ch.name && !taken[*i])
                        .map(|(i, _)| i)
                        .expect("the structure check matched the names as multisets");
                    taken[at] = true;
                    group.channels[at]
                })
                .collect();

            let series = file.series_for(&selectors)?;

            // Every channel of a group shares its master, so the shifted time
            // axis is computed once from the first of them and applied to all.
            // A group with no samples contributes an empty axis and leaves
            // `last_timestamp` alone — it is not a recording that ended, so
            // nothing should be continued from it — but its channels are still
            // merged below, so a channel no file has samples for still reports
            // its own sample type rather than a stand-in.
            let master: Vec<f64> = match series.first() {
                Some(first) if !first.timestamps.is_empty() => {
                    let mut master: Vec<f64> = first
                        .timestamps
                        .iter()
                        .map(|t| t + offsets[file_index])
                        .collect();
                    if let Some(last) = last_timestamp {
                        if last >= master[0] {
                            let delta = if master.len() >= 2 {
                                master[1] - master[0]
                            } else {
                                0.001
                            };
                            let shift = last + delta - master[0];
                            for t in &mut master {
                                *t += shift;
                            }
                        }
                    }
                    last_timestamp = master.last().copied();
                    master
                }
                _ => Vec::new(),
            };

            for (slot, s) in series.into_iter().enumerate() {
                timestamps[slot].extend_from_slice(&master);
                let added = s.values.len();
                append_validity(
                    &mut validity[slot],
                    counts[slot],
                    s.validity.as_deref(),
                    added,
                );
                counts[slot] += added;
                match &mut values[slot] {
                    Some(acc) => append_values(acc, &s.values)?,
                    none => *none = Some(s.values),
                }
            }
        }

        for (slot, channel) in first_group.channels.iter().enumerate() {
            // Always `Some` in practice: the file list is non-empty and every
            // file yields one series per slot, even a file with no samples.
            let vals = values[slot].take().unwrap_or(SignalValues::F64(Vec::new()));
            out.push(SignalSeries::new(
                (*channel).clone(),
                std::mem::take(&mut timestamps[slot]),
                vals,
                validity[slot].take(),
            )?);
        }
    }

    Ok(out)
}

/// Combines several measurements side by side: every file's channels present
/// together, rather than one file's samples after another's.
///
/// Nothing is merged and nothing is resampled. A channel name that appears in
/// two files yields two series, each keeping its own file's samples and its own
/// file's sample rate; [`StackedSeries::file_index`] says which file each came
/// from. Files need not have the same structure — a channel present in only one
/// of them simply appears once.
///
/// The "common time base" is a common *origin*, not a common raster: under
/// [`TimeAlignment::StartTime`] each file's timestamps are shifted by its
/// header start time relative to the earliest file's, so t = 0 means the same
/// instant for all of them. asammdf's `MDF.stack` does exactly this. To put
/// stacked series on one raster as well, resample them afterwards with
/// [`SignalSeries::resample`].
///
/// # Example
///
/// ```no_run
/// # use falcon_mdf::{Mf4File, TimeAlignment, multi_ops};
/// let a = Mf4File::open("engine.mf4")?;
/// let b = Mf4File::open("chassis.mf4")?;
/// for s in multi_ops::stack(&[&a, &b], TimeAlignment::StartTime)? {
///     println!("file {}: {}", s.file_index, s.series.name());
/// }
/// # Ok::<(), falcon_mdf::error::Mf4Error>(())
/// ```
pub fn stack(files: &[&Mf4File], alignment: TimeAlignment) -> Result<Vec<StackedSeries>> {
    if files.is_empty() {
        return Err(Mf4Error::parse_error("stack needs at least one file"));
    }

    let offsets = start_time_offsets(files, alignment);
    let mut out = Vec::new();

    for (file_index, file) in files.iter().enumerate() {
        let offset = offsets[file_index];
        for group in group_layouts(file) {
            if group.channels.is_empty() {
                continue;
            }
            for mut series in file.series_for(&group.channels)? {
                for t in &mut series.timestamps {
                    *t += offset;
                }
                out.push(StackedSeries { file_index, series });
            }
        }
    }

    Ok(out)
}

/// The non-master channels of one channel group.
struct GroupLayout<'a> {
    channels: Vec<&'a Channel>,
}

/// Lists every channel group's non-master channels, in file order.
fn group_layouts(file: &Mf4File) -> Vec<GroupLayout<'_>> {
    file.data_groups()
        .iter()
        .flat_map(|dg| dg.channel_groups.iter())
        .map(|cg| GroupLayout {
            // The master is the time axis, carried on every series already;
            // emitting it as a channel of its own would duplicate it.
            channels: cg.channels.iter().filter(|ch| !ch.is_master()).collect(),
        })
        .collect()
}

/// Offsets, in seconds, from the earliest file's header start time.
///
/// Clamped at zero so that a file whose header claims to predate the earliest
/// one cannot pull samples backwards; asammdf clamps the same way.
fn start_time_offsets(files: &[&Mf4File], alignment: TimeAlignment) -> Vec<f64> {
    match alignment {
        TimeAlignment::AsRecorded => vec![0.0; files.len()],
        TimeAlignment::StartTime => {
            let starts: Vec<i64> = files
                .iter()
                .map(|f| f.start_time().timestamp_ns)
                .collect();
            let oldest = starts.iter().copied().min().unwrap_or(0);
            // `saturating_sub`: a header start time is whatever the file says
            // it is, and two of them far enough apart overflow an i64 of
            // nanoseconds. A wrapped difference would place a file at a
            // plausible but wrong offset, which is the failure mode to avoid.
            starts
                .iter()
                .map(|&ns| (ns.saturating_sub(oldest) as f64 / 1e9).max(0.0))
                .collect()
        }
    }
}

/// Rejects files that do not describe the same measurement.
fn check_same_structure(layouts: &[Vec<GroupLayout<'_>>]) -> Result<()> {
    let first = &layouts[0];
    for (file_index, layout) in layouts.iter().enumerate().skip(1) {
        if layout.len() != first.len() {
            return Err(Mf4Error::parse_error(format!(
                "cannot concatenate: file 0 has {} channel groups but file {file_index} has {}",
                first.len(),
                layout.len()
            )));
        }
        for (group_index, (a, b)) in first.iter().zip(layout).enumerate() {
            let mut want: Vec<&str> = a.channels.iter().map(|ch| ch.name.as_str()).collect();
            let mut got: Vec<&str> = b.channels.iter().map(|ch| ch.name.as_str()).collect();
            want.sort_unstable();
            got.sort_unstable();
            if want != got {
                return Err(Mf4Error::parse_error(format!(
                    "cannot concatenate: channel group {group_index} holds {want:?} in file 0 \
                     but {got:?} in file {file_index}"
                )));
            }
        }
    }
    Ok(())
}

/// Extends an accumulated validity mask with `added` samples' worth of `next`.
///
/// A file without invalidation bits contributes valid samples, so mixing a file
/// that has a mask with one that does not yields a mask covering both rather
/// than dropping the one that existed.
fn append_validity(
    acc: &mut Option<Vec<bool>>,
    acc_len: usize,
    next: Option<&[bool]>,
    added: usize,
) {
    match (acc.as_mut(), next) {
        (None, None) => {}
        (None, Some(v)) => {
            let mut mask = vec![true; acc_len];
            mask.extend_from_slice(v);
            *acc = Some(mask);
        }
        (Some(mask), Some(v)) => mask.extend_from_slice(v),
        (Some(mask), None) => mask.extend(std::iter::repeat_n(true, added)),
    }
}

/// Appends `next` to `acc` in place, refusing to mix sample representations.
///
/// The refusal matters: two files whose copies of a channel decode to different
/// widths are not the same channel, and gluing their bytes together would
/// produce samples that parse but mean nothing.
fn append_values(acc: &mut SignalValues, next: &SignalValues) -> Result<()> {
    fn mismatch(acc: &SignalValues, next: &SignalValues) -> Mf4Error {
        Mf4Error::parse_error(format!(
            "cannot concatenate {:?} samples onto {:?} samples",
            next.kind(),
            acc.kind()
        ))
    }

    match (acc, next) {
        (SignalValues::U8(a), SignalValues::U8(b)) => a.extend_from_slice(b),
        (SignalValues::U16(a), SignalValues::U16(b)) => a.extend_from_slice(b),
        (SignalValues::U32(a), SignalValues::U32(b)) => a.extend_from_slice(b),
        (SignalValues::U64(a), SignalValues::U64(b)) => a.extend_from_slice(b),
        (SignalValues::I8(a), SignalValues::I8(b)) => a.extend_from_slice(b),
        (SignalValues::I16(a), SignalValues::I16(b)) => a.extend_from_slice(b),
        (SignalValues::I32(a), SignalValues::I32(b)) => a.extend_from_slice(b),
        (SignalValues::I64(a), SignalValues::I64(b)) => a.extend_from_slice(b),
        (SignalValues::F32(a), SignalValues::F32(b)) => a.extend_from_slice(b),
        (SignalValues::F64(a), SignalValues::F64(b)) => a.extend_from_slice(b),
        (SignalValues::Str(a), SignalValues::Str(b)) => a.extend_from_slice(b),
        (SignalValues::CanopenDate(a), SignalValues::CanopenDate(b)) => a.extend_from_slice(b),
        (SignalValues::CanopenTime(a), SignalValues::CanopenTime(b)) => a.extend_from_slice(b),
        (
            SignalValues::Complex { re, im },
            SignalValues::Complex {
                re: re_b,
                im: im_b,
            },
        ) => {
            re.extend_from_slice(re_b);
            im.extend_from_slice(im_b);
        }
        (
            SignalValues::Bytes { data, width },
            SignalValues::Bytes {
                data: data_b,
                width: width_b,
            },
        ) => {
            if width != width_b {
                return Err(Mf4Error::parse_error(format!(
                    "cannot concatenate {width_b}-byte samples onto {width}-byte samples"
                )));
            }
            data.extend_from_slice(data_b);
        }
        (
            SignalValues::VarBytes { data, starts },
            SignalValues::VarBytes {
                data: data_b,
                starts: starts_b,
            },
        ) => {
            let base = data.len();
            data.extend_from_slice(data_b);
            starts.extend(starts_b.iter().skip(1).map(|&s| s + base));
        }
        (
            SignalValues::Array {
                values,
                elements_per_sample,
            },
            SignalValues::Array {
                values: values_b,
                elements_per_sample: per_b,
            },
        ) => {
            if elements_per_sample != per_b {
                return Err(Mf4Error::parse_error(format!(
                    "cannot concatenate {per_b}-element samples onto \
                     {elements_per_sample}-element samples"
                )));
            }
            values.extend_from_slice(values_b);
        }
        (
            SignalValues::ArrayVarLen { values, starts },
            SignalValues::ArrayVarLen {
                values: values_b,
                starts: starts_b,
            },
        ) => {
            let base = values.len();
            values.extend_from_slice(values_b);
            starts.extend(starts_b.iter().skip(1).map(|&s| s + base));
        }
        (acc, next) => return Err(mismatch(acc, next)),
    }
    Ok(())
}
