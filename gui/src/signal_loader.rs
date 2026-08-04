//! Decodes one channel's signal, and its master, on a worker thread — so
//! selecting a large channel does not freeze the UI.
//!
//! Mirrors `loader.rs`: `Signal` is `Send + Sync` and owns its data (pinned
//! in `tests/api_surface.rs`), so the read, decompress and per-sample decode
//! all happen off the UI thread; only the resulting `Vec<f64>`s cross back,
//! which is what the plot panel and the decimator need anyway.

use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use falcon_mdf::Mf4File;

use crate::model::ChannelLoc;

/// A channel's decoded samples, paired with its master (time) channel.
pub struct ChannelSignal {
    pub loc: ChannelLoc,
    pub name: String,
    pub unit: String,
    pub time_name: String,
    pub time_unit: String,
    /// Same length as `values`, ascending. From the channel's master when it
    /// has one; otherwise the sample index, so a channel with no master still
    /// plots against something meaningful to hover.
    pub times: Vec<f64>,
    pub values: Vec<f64>,
}

pub enum SignalLoadResult {
    Ok(ChannelSignal),
    Err { loc: ChannelLoc, message: String },
}

/// Starts decoding `loc` on a new thread and returns a receiver for the
/// result. `ctx` wakes the UI once decoding finishes, same as `loader.rs`.
pub fn spawn_signal_load(
    file: Arc<Mf4File>,
    loc: ChannelLoc,
    ctx: egui::Context,
) -> Receiver<SignalLoadResult> {
    let (tx, rx) = channel();

    std::thread::spawn(move || {
        let result = load(&file, loc);
        let _ = tx.send(result);
        ctx.request_repaint();
    });

    rx
}

fn load(file: &Mf4File, loc: ChannelLoc) -> SignalLoadResult {
    let channel = &file.data_groups()[loc.data_group_index].channel_groups[loc.channel_group_index]
        .channels[loc.channel_index];

    let values = match file.signal(channel).and_then(|s| s.values_f64()) {
        Ok(v) => v,
        Err(e) => {
            return SignalLoadResult::Err {
                loc,
                message: e.to_string(),
            }
        }
    };

    let (times, time_name, time_unit) =
        match file.master_channel(loc.data_group_index, loc.channel_group_index) {
            Some(master) => match file.signal(master).and_then(|s| s.values_f64()) {
                Ok(t) => (t, master.name.clone(), master.unit.clone()),
                Err(e) => {
                    return SignalLoadResult::Err {
                        loc,
                        message: format!("master channel: {e}"),
                    }
                }
            },
            None => (
                (0..values.len()).map(|i| i as f64).collect(),
                "Sample index".to_string(),
                String::new(),
            ),
        };

    SignalLoadResult::Ok(ChannelSignal {
        loc,
        name: channel.name.clone(),
        unit: channel.unit.clone(),
        time_name,
        time_unit,
        times,
        values,
    })
}
