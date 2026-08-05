//! CSV export of decoded channels.
//!
//! Both the `export_to_csv` example and the GUI's export action write through
//! [`write_csv`], so a channel exported from the app is byte for byte what the
//! example produces.

use std::io::Write;

use crate::error::Result;
use crate::model::Channel;
use crate::Mf4File;

/// Writes `channels` to `out` as CSV: one time column taken from the first
/// channel's master, then one value column per channel in the order given.
///
/// With one channel this is exactly the `export_to_csv` example's format —
/// `Time [unit]` (or `Index` when the group has no master), nine-decimal
/// values — so a single-channel export is byte-identical to it. With several,
/// each row carries one cell per column; a channel with fewer samples leaves
/// its trailing cells empty rather than inventing values, and rows past the
/// time column's last timestamp leave the time cell empty.
///
/// An empty slice writes nothing and succeeds: nothing was asked, and nothing
/// is the truthful answer. Callers with a user interface disable the action
/// instead; this keeps the function total.
pub fn write_csv<W: Write>(file: &Mf4File, channels: &[&Channel], out: &mut W) -> Result<()> {
    let Some(first) = channels.first() else {
        return Ok(());
    };

    let time = time_column(file, first);

    let mut headers = Vec::with_capacity(channels.len() + 1);
    headers.push(match &time {
        Some((_, unit)) => format!("Time [{unit}]"),
        None => "Index".to_string(),
    });
    for channel in channels {
        headers.push(column_header(channel));
    }
    writeln!(
        out,
        "{}",
        headers
            .iter()
            .map(|h| csv_field(h))
            .collect::<Vec<_>>()
            .join(",")
    )?;

    let columns: Vec<Vec<f64>> = channels
        .iter()
        .map(|channel| file.signal(channel)?.values_f64())
        .collect::<Result<Vec<_>>>()?;

    let row_count = columns
        .iter()
        .map(Vec::len)
        .chain(time.as_ref().map(|(times, _)| times.len()))
        .max()
        .unwrap_or(0);

    for row in 0..row_count {
        let mut cells = Vec::with_capacity(channels.len() + 1);
        cells.push(match &time {
            Some((times, _)) => times
                .get(row)
                .map(|t| format!("{t:.9}"))
                .unwrap_or_default(),
            None => format!("{row}"),
        });
        for column in &columns {
            cells.push(
                column
                    .get(row)
                    .map(|value| format!("{value:.9}"))
                    .unwrap_or_default(),
            );
        }
        writeln!(out, "{}", cells.join(","))?;
    }

    Ok(())
}

/// The time column for an export: the first channel's master, decoded, with
/// its unit for the header. `None` when the group has no master or the master
/// does not decode, in which case the export falls back to sample indices.
fn time_column(file: &Mf4File, channel: &Channel) -> Option<(Vec<f64>, String)> {
    let dg = &file.data_groups()[channel.data_group_index];
    let cg = &dg.channel_groups[channel.channel_group_index];
    let signal = file.signal(cg.master_channel()?).ok()?;
    let unit = signal.unit().to_string();
    let times = signal.values_f64().ok()?;
    Some((times, unit))
}

/// A value column's header: the channel's name, with its unit in brackets
/// when it has one.
fn column_header(channel: &Channel) -> String {
    if channel.unit.is_empty() {
        channel.name.clone()
    } else {
        format!("{} [{}]", channel.name, channel.unit)
    }
}

/// RFC 4180 escaping: a field containing a comma, a quote or a line break is
/// wrapped in quotes with its quotes doubled. Channel names are file-supplied
/// text, and a bus-signal name like `Boost, psi` would otherwise split into
/// two columns on its way into a spreadsheet. Plain fields pass through
/// untouched, so names without special characters export exactly as before.
fn csv_field(field: &str) -> std::borrow::Cow<'_, str> {
    if field.contains([',', '"', '\n', '\r']) {
        std::borrow::Cow::Owned(format!("\"{}\"", field.replace('"', "\"\"")))
    } else {
        std::borrow::Cow::Borrowed(field)
    }
}
