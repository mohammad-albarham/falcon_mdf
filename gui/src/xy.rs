//! Putting two channels on one pair of axes as X against Y.
//!
//! A time plot asks "what was this signal doing"; an X-Y plot asks "how did
//! these two move together" — steering against lateral acceleration, torque
//! against speed. The whole difficulty is the pairing: a point `(x, y)` is a
//! claim that both channels held those values *at the same instant*, and two
//! channels in an MF4 file need not share a raster, a range, or even a clock.
//!
//! So this module's job is to say how the pairing was made, or to refuse.
//! [`pair_xy`] returns a [`XySeries`] carrying its own [`XyPairing`] — exact
//! or resampled, and over what — or an [`XyRefusal`] that names what stopped
//! it. Nothing here guesses: a curve drawn from a pairing nobody can justify
//! looks exactly like a curve drawn from a real one, and the reader has no
//! way to tell them apart afterwards.
//!
//! Pure functions over decoded signals, so the rules are pinned in
//! `gui/tests/xy_plot.rs` without a window.

use crate::computed::resample_linear;
use crate::signal_loader::ChannelSignal;

/// Two timestamps closer than this are the same instant. Masters are written
/// as `f64` seconds and arrive through conversions, so two channels sharing
/// one master can differ in the last bit or two.
const SAME_TIME_EPSILON: f64 = 1e-9;

/// How the points in an [`XySeries`] were paired up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XyPairing {
    /// Both channels are on the same timestamps, so every point is two real
    /// samples measured at one instant. Nothing was interpolated.
    Exact,
    /// The channels are on different rasters. Y was interpolated onto X's
    /// timestamps, over the span where both channels have samples.
    Resampled,
}

impl XyPairing {
    /// What the panel says under the plot, so the pairing is never something
    /// the reader has to infer from the shape of the curve.
    pub fn describe(self) -> &'static str {
        match self {
            XyPairing::Exact => {
                "Both channels share a master, so every point is two samples measured together."
            }
            XyPairing::Resampled => {
                "The channels are on different rasters: Y is linearly interpolated onto X's \
                 timestamps, over the span where both have samples."
            }
        }
    }
}

/// Why two channels could not be put on a common time base.
///
/// Each variant is a refusal, not a warning: the panel draws nothing and
/// shows [`XyRefusal::message`] instead. Every one of these is a case where
/// some pairing could have been invented — clamping, extrapolating, pairing
/// by sample index, pairing two files' timestamps because both start at
/// zero — and where the invented curve would be indistinguishable from a
/// real one.
#[derive(Debug, Clone, PartialEq)]
pub enum XyRefusal {
    /// One of the two channels decoded to nothing.
    NoSamples { axis: Axis },
    /// The two channels' time spans do not overlap: they were never
    /// recording at the same time, so no instant has both values.
    NoOverlap {
        x_span: (f64, f64),
        y_span: (f64, f64),
    },
    /// X and Y come from different files while the plot is aligned on each
    /// file's own zero. See the module docs on [`crate::panels::plot`]: under
    /// that alignment `t = 5` in one file and `t = 5` in the other are
    /// different instants that merely share a number.
    CrossFileNeedsAbsoluteTime,
    /// The spans overlap, but every pair in the overlap was dropped — each
    /// one had a sample the file marks invalid, or a NaN, on one axis or the
    /// other.
    NothingValid { dropped: usize },
}

/// Which axis a refusal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

impl Axis {
    pub fn label(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
        }
    }
}

impl XyRefusal {
    /// The sentence shown in place of the plot. It says what is wrong and,
    /// where there is one, what the user can do about it — a refusal that
    /// leaves the reader guessing is only marginally better than a wrong
    /// curve.
    pub fn message(&self) -> String {
        match self {
            XyRefusal::NoSamples { axis } => format!(
                "The {} channel has no samples, so there is nothing to pair against.",
                axis.label()
            ),
            XyRefusal::NoOverlap { x_span, y_span } => format!(
                "These channels were never recording at the same time \u{2014} X covers \
                 {:.6}\u{2026}{:.6} s and Y covers {:.6}\u{2026}{:.6} s, which do not overlap. \
                 No instant has a value on both axes, so there is no X-Y curve to draw.",
                x_span.0, x_span.1, y_span.0, y_span.1
            ),
            XyRefusal::CrossFileNeedsAbsoluteTime => {
                "X and Y come from different files, and the plot is aligned on each file's own \
                 zero \u{2014} so the same t means a different instant in each. Pairing them \
                 would draw a relationship that is an artefact of where each recording was \
                 triggered. Switch \"Align B to A\" to absolute time in the Plot tab, or pick \
                 both channels from one file."
                    .to_string()
            }
            XyRefusal::NothingValid { dropped } => format!(
                "The channels overlap in time, but all {dropped} paired sample(s) in the \
                 overlap are marked invalid or are NaN on one axis or the other."
            ),
        }
    }
}

/// Two channels paired into points, with the story of how they were paired.
#[derive(Debug, Clone, PartialEq)]
pub struct XySeries {
    /// The curve, as `[x_value, y_value]`, in ascending time order.
    pub points: Vec<[f64; 2]>,
    /// The instant each point was taken at, on the shared (plot-space) axis.
    /// Same length as `points`. This is what lets the measurement cursors
    /// mark a point: a cursor is a time, and an X-Y plot has no time axis to
    /// put it on.
    pub times: Vec<f64>,
    pub pairing: XyPairing,
    /// Pairs inside the overlap that were dropped because a sample was
    /// invalid or NaN on one axis. Reported rather than silently absent.
    pub dropped: usize,
}

/// A point on the curve, with the instant it was actually measured at.
///
/// The two are reported together because they need not be the same: a cursor
/// dropped into a gap in the recording matches the nearest sample, which may
/// be a long way off. Callers show `time`, not the time they asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XyMatch {
    pub point: [f64; 2],
    /// The paired sample's own instant on the shared axis.
    pub time: f64,
}

impl XySeries {
    /// The point the curve was at time `t`, or `None` when `t` is outside the
    /// span the pairing covers.
    ///
    /// `None` rather than the nearest endpoint: a cursor parked past the end
    /// of the overlap would otherwise pin a marker to the last point and read
    /// as though the curve were there at that time.
    ///
    /// Inside the span the nearest sample is matched, but the match carries
    /// that sample's own time — the curve may have a gap an hour wide, and
    /// reporting the cursor's time against a sample from the far side of it
    /// would be the same lie in a smaller place. (R3 finding 5.2.)
    pub fn point_at(&self, t: f64) -> Option<XyMatch> {
        let first = *self.times.first()?;
        let last = *self.times.last()?;
        if t < first - SAME_TIME_EPSILON || t > last + SAME_TIME_EPSILON {
            return None;
        }
        let i = crate::panels::plot::nearest_index(&self.times, t);
        Some(XyMatch {
            point: *self.points.get(i)?,
            time: *self.times.get(i)?,
        })
    }

    /// The time span the pairing covers, on the shared axis.
    pub fn span(&self) -> Option<(f64, f64)> {
        Some((*self.times.first()?, *self.times.last()?))
    }
}

/// Pairs `x` and `y` into an X-Y curve, or refuses.
///
/// `x_offset` and `y_offset` are the shifts that put each signal on the
/// shared axis — zero for the first file, and the alignment offset for the
/// second (see [`crate::panels::plot::alignment_offset_seconds`]). Everything
/// below works in that shared space.
///
/// `cross_file` says the two channels come from different measurements, and
/// `absolute_alignment` says the plot is currently aligned on the headers'
/// wall clock. The two together are the only case in which a cross-file
/// pairing means anything, which is why both are asked for rather than
/// inferred.
pub fn pair_xy(
    x: &ChannelSignal,
    x_offset: f64,
    y: &ChannelSignal,
    y_offset: f64,
    cross_file: bool,
    absolute_alignment: bool,
) -> Result<XySeries, XyRefusal> {
    // Checked first: it is a refusal about what the axes *mean*, and saying
    // "no overlap" about two files that were never on one clock would be
    // answering a question nobody asked.
    if cross_file && !absolute_alignment {
        return Err(XyRefusal::CrossFileNeedsAbsoluteTime);
    }

    let x_len = x.times.len().min(x.values.len());
    let y_len = y.times.len().min(y.values.len());
    if x_len == 0 {
        return Err(XyRefusal::NoSamples { axis: Axis::X });
    }
    if y_len == 0 {
        return Err(XyRefusal::NoSamples { axis: Axis::Y });
    }

    let x_span = (x.times[0] + x_offset, x.times[x_len - 1] + x_offset);
    let y_span = (y.times[0] + y_offset, y.times[y_len - 1] + y_offset);
    let lo = x_span.0.max(y_span.0);
    let hi = x_span.1.min(y_span.1);
    if lo > hi + SAME_TIME_EPSILON {
        return Err(XyRefusal::NoOverlap { x_span, y_span });
    }

    let same_raster = x_len == y_len
        && (0..x_len)
            .all(|i| ((x.times[i] + x_offset) - (y.times[i] + y_offset)).abs() <= SAME_TIME_EPSILON);

    if same_raster {
        return collect(
            (0..x_len).map(|i| {
                (
                    x.times[i] + x_offset,
                    x.values[i],
                    y.values[i],
                    valid_at(x, i) && valid_at(y, i),
                )
            }),
            XyPairing::Exact,
        );
    }

    // X's own timestamps are the raster, restricted to the overlap: they are
    // real measurements of the X axis, so only Y is invented. Interpolating
    // both onto a synthetic union raster would invent twice as much.
    //
    // The index and the shared-axis time are collected together in one pass,
    // so there is no way for the two to fall out of step and pair an X value
    // with another sample's instant.
    // The bounds are exact, with none of the slack the overlap test above
    // allows itself. `resample_linear` clamps rather than refusing, so a
    // target even a nanosecond outside Y's range would come back holding Y's
    // endpoint value — invented data, which is the one thing this module
    // exists to avoid. A boundary sample that misses by an ulp is dropped
    // instead, which costs one point out of thousands. (R3 finding 2.)
    let kept: Vec<(usize, f64)> = (0..x_len)
        .map(|i| (i, x.times[i] + x_offset))
        .filter(|(_, t)| *t >= lo && *t <= hi)
        .collect();
    if kept.is_empty() {
        return Err(XyRefusal::NoOverlap { x_span, y_span });
    }
    let targets: Vec<f64> = kept.iter().map(|(_, t)| *t).collect();

    // `resample_linear` works in the source signal's own space, so the
    // targets are carried back out of shared space rather than Y's whole
    // master being shifted into it.
    let targets_in_y: Vec<f64> = targets.iter().map(|t| t - y_offset).collect();
    let (y_values, y_valid) = resample_linear(
        &y.times[..y_len],
        &y.values[..y_len],
        y.valid.as_deref(),
        &targets_in_y,
    );

    collect(
        kept.iter().enumerate().map(|(k, &(i, t))| {
            let y_ok = y_valid.as_ref().is_none_or(|v| v[k]);
            (t, x.values[i], y_values[k], valid_at(x, i) && y_ok)
        }),
        XyPairing::Resampled,
    )
}

/// Turns `(time, x, y, valid)` tuples into a series, dropping the pairs that
/// are not usable and counting them.
fn collect(
    pairs: impl Iterator<Item = (f64, f64, f64, bool)>,
    pairing: XyPairing,
) -> Result<XySeries, XyRefusal> {
    let mut points = Vec::new();
    let mut times = Vec::new();
    let mut dropped = 0usize;
    for (t, xv, yv, valid) in pairs {
        // A NaN is dropped for the same reason an invalid sample is: the
        // record held something, but it was not a measurement.
        if !valid || xv.is_nan() || yv.is_nan() {
            dropped += 1;
            continue;
        }
        points.push([xv, yv]);
        times.push(t);
    }
    if points.is_empty() {
        return Err(XyRefusal::NothingValid { dropped });
    }
    Ok(XySeries {
        points,
        times,
        pairing,
        dropped,
    })
}

/// Whether sample `i` of `signal` is one the file vouches for.
fn valid_at(signal: &ChannelSignal, i: usize) -> bool {
    match &signal.valid {
        Some(v) => v.get(i).copied().unwrap_or(true),
        None => true,
    }
}
