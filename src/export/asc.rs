//! Vector CANoe ASCII trace (.asc) export.
//!
//! # What an ASC file contains
//!
//! Vector's CANoe ASCII logging format (.asc) records CAN and LIN bus traffic
//! chronologically. It begins with a standard header:
//!
//! ```text
//! date Thu Feb 13 08:27:45.000000 AM 2020
//! base hex  timestamps absolute
//! no internal events logged
//! ```
//!
//! followed by one line per frame in chronological order:
//!
//! ```text
//!  71.664350 1  123             Rx   d 4 12 34 56 78
//!  71.664350 1  18f00400x       Rx   d 8 11 22 33 44 55 66 77 88
//! ```
//!
//! # Multi-bus logs are sorted
//!
//! If the file carries multiple CAN frame groups (e.g. multi-bus loggers),
//! frames from all groups are merged and sorted chronologically so the resulting
//! ASC trace replays in strict time order.

use std::io::Write;

use crate::bus::CanFrame;
use crate::error::Result;
use crate::Mf4File;

/// Writes all CAN frames from `file` to `out` in Vector CANoe ASCII format (.asc).
///
/// If the file carries multiple CAN frame groups, their frames are merged and
/// written in chronological timestamp order.
///
/// # Errors
///
/// Returns whatever error reading the file's CAN frame groups returns.
pub fn write_asc<W: Write>(file: &Mf4File, out: &mut W) -> Result<()> {
    let mut groups_frames = Vec::new();
    for group in file.can_frame_groups() {
        groups_frames.push(file.can_frames(group)?);
    }

    let mut all_frames = Vec::new();
    for frames in &groups_frames {
        for frame in frames.iter() {
            all_frames.push(frame);
        }
    }

    // Sort stably by timestamp
    all_frames.sort_by(|a, b| a.timestamp.total_cmp(&b.timestamp));

    let start_time_ns = file.start_time().timestamp_ns;
    write_asc_frames(&all_frames, Some(start_time_ns), out)
}

/// Formats start timestamp into CANoe ASCII date header string.
///
/// Standard format: `%a %b %d %I:%M:%S.%6f %p %Y`
/// Example: `Thu Feb 13 08:27:45.000000 AM 2020`
fn format_date_header(start_time_ns: Option<i64>) -> String {
    let Some(ns) = start_time_ns else {
        return "date Thu Jan 01 12:00:00.000000 AM 1970".to_string();
    };

    if ns <= 0 {
        return "date Thu Jan 01 12:00:00.000000 AM 1970".to_string();
    }

    let secs = ns / 1_000_000_000;
    let subsec_ns = (ns % 1_000_000_000) as u32;

    let dt = chrono::DateTime::from_timestamp(secs, subsec_ns);
    match dt {
        Some(d) => format!("date {}", d.format("%a %b %d %I:%M:%S.%6f %p %Y")),
        None => "date Thu Jan 01 12:00:00.000000 AM 1970".to_string(),
    }
}

/// Formats a float timestamp matching Python/asammdf `f"{t: 9.6f}"` into
/// `buf`, which is cleared first.
///
/// The padding is done in place: a `format!` per leading space — the obvious
/// way to write this — reallocates up to nine times per frame.
fn write_timestamp(buf: &mut String, t: f64) {
    use std::fmt::Write as _;

    buf.clear();
    write!(buf, "{t:.6}").expect("writing to a String cannot fail");
    if !buf.starts_with('-') {
        buf.insert(0, ' ');
    }
    while buf.len() < 9 {
        buf.insert(0, ' ');
    }
}

/// Writes `frames` to `out` in Vector CANoe ASCII format (.asc).
pub fn write_asc_frames<'a, W: Write>(
    frames: &[CanFrame<'a>],
    start_time_ns: Option<i64>,
    out: &mut W,
) -> Result<()> {
    use std::fmt::Write as _;

    writeln!(out, "{}", format_date_header(start_time_ns))?;
    writeln!(out, "base hex  timestamps absolute")?;
    writeln!(out, "no internal events logged")?;

    // One row buffer reused across frames; the per-frame strings this
    // replaces were the bulk of the allocator traffic on multi-million-frame
    // bus logs.
    let mut row = String::with_capacity(96);
    for frame in frames {
        row.clear();
        write_timestamp(&mut row, frame.timestamp);
        let bus = if frame.bus_channel == 0 {
            1
        } else {
            frame.bus_channel
        };
        let is_ext = frame.extended.unwrap_or(frame.id > 0x7FF);
        let id_str = if is_ext {
            format!("{:x}x", frame.id)
        } else {
            format!("{:x}", frame.id)
        };

        // Format: "{t: 9.6f} {bus}  {id:<15} {dir:<4} d {dlc:x} {data}" —
        // `{dir:<4}` pads "Rx" to four characters before the separator.
        write!(row, " {bus}  {id_str:<15} Rx   d {:x} ", frame.data.len())
            .expect("writing to a String cannot fail");
        for (i, byte) in frame.data.iter().enumerate() {
            if i > 0 {
                row.push(' ');
            }
            write!(row, "{byte:02X}").expect("writing to a String cannot fail");
        }
        row.push('\n');
        out.write_all(row.as_bytes())?;
    }

    Ok(())
}
