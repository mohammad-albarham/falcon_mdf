//! Putting two channels on one pair of axes as a GPS track (longitude vs latitude).
//!
//! A GPS track asks "where did the vehicle travel in space". A point `(lon, lat)`
//! is a claim that both coordinates were measured at *the same instant*. Two channels
//! in an MF4 file need not share a raster, a range, or even a clock.
//!
//! So this module pairs the latitude and longitude channels, resamples onto a common
//! time base when needed, or refuses. It also validates coordinate ranges
//! ([-90..90] lat, [-180..180] lon) and provides degenerate input detection (single
//! points, all-identical coordinates).
//!
//! Pure functions over decoded signals, so the rules are pinned in `gui/tests/gps_track.rs`
//! without an `egui::Context` or window.

use crate::signal_loader::ChannelSignal;
use crate::xy::{pair_xy, Axis, XyPairing, XyRefusal};

/// Two timestamps closer than this are the same instant.
const SAME_TIME_EPSILON: f64 = 1e-9;

/// Two coordinates closer than this are the same coordinate.
const SAME_COORDINATE_EPSILON: f64 = 1e-9;

/// Which coordinate a refusal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coordinate {
    Latitude,
    Longitude,
}

impl Coordinate {
    pub fn label(self) -> &'static str {
        match self {
            Coordinate::Latitude => "latitude",
            Coordinate::Longitude => "longitude",
        }
    }
}

/// Why two channels could not be drawn as a GPS track.
#[derive(Debug, Clone, PartialEq)]
pub enum GpsRefusal {
    /// One of the two channels decoded to nothing.
    NoSamples { coordinate: Coordinate },
    /// The two channels' time spans do not overlap: they were never
    /// recording at the same time, so no instant has both values.
    NoOverlap {
        lat_span: (f64, f64),
        lon_span: (f64, f64),
    },
    /// Latitude and longitude come from different files while the plot is
    /// aligned on each file's own zero.
    CrossFileNeedsAbsoluteTime,
    /// The spans overlap, but every pair in the overlap was dropped — each
    /// one had a sample marked invalid or NaN.
    NothingValid { dropped: usize },
    /// Latitude values outside the valid range [-90.0, 90.0] degrees.
    LatitudeOutOfRange { min: f64, max: f64 },
    /// Longitude values outside the valid range [-180.0, 180.0] degrees.
    LongitudeOutOfRange { min: f64, max: f64 },
}

impl GpsRefusal {
    /// The message shown in place of the plot.
    pub fn message(&self) -> String {
        match self {
            GpsRefusal::NoSamples { coordinate } => format!(
                "The {} channel has no samples, so there is nothing to pair against.",
                coordinate.label()
            ),
            GpsRefusal::NoOverlap { lat_span, lon_span } => format!(
                "These channels were never recording at the same time \u{2014} latitude covers \
                 {:.6}\u{2026}{:.6} s and longitude covers {:.6}\u{2026}{:.6} s, which do not overlap. \
                 No instant has a value on both axes, so there is no GPS track to draw.",
                lat_span.0, lat_span.1, lon_span.0, lon_span.1
            ),
            GpsRefusal::CrossFileNeedsAbsoluteTime => {
                "Latitude and longitude come from different files, and the plot is aligned on \
                 each file's own zero \u{2014} so the same t means a different instant in each. \
                 Switch \"Align B to A\" to absolute time in the Plot tab, or pick both channels \
                 from one file."
                    .to_string()
            }
            GpsRefusal::NothingValid { dropped } => format!(
                "The channels overlap in time, but all {dropped} paired sample(s) in the \
                 overlap are marked invalid or are NaN on one axis or the other."
            ),
            GpsRefusal::LatitudeOutOfRange { min, max } => format!(
                "Latitude values must be between \u{2013}90\u{00b0} and +90\u{00b0}, but the channel \
                 ranges from {min:.6}\u{00b0} to {max:.6}\u{00b0}. Check that the latitude channel \
                 is in degrees."
            ),
            GpsRefusal::LongitudeOutOfRange { min, max } => format!(
                "Longitude values must be between \u{2013}180\u{00b0} and +180\u{00b0}, but the channel \
                 ranges from {min:.6}\u{00b0} to {max:.6}\u{00b0}. Check that the longitude channel \
                 is in degrees."
            ),
        }
    }
}

/// A paired GPS track, with points in `[lon, lat]` order for plotting.
#[derive(Debug, Clone, PartialEq)]
pub struct GpsSeries {
    /// The track as `[lon_value, lat_value]`, in ascending time order.
    pub points: Vec<[f64; 2]>,
    /// The instant each point was taken at, on the shared (plot-space) axis.
    pub times: Vec<f64>,
    pub pairing: XyPairing,
    /// Pairs inside the overlap that were dropped because a sample was
    /// invalid or NaN on one axis.
    pub dropped: usize,
}

/// A point on the GPS track with the instant it was actually measured at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpsMatch {
    /// `[lon, lat]`
    pub point: [f64; 2],
    /// The paired sample's own instant on the shared axis.
    pub time: f64,
}

impl GpsMatch {
    pub fn longitude(&self) -> f64 {
        self.point[0]
    }

    pub fn latitude(&self) -> f64 {
        self.point[1]
    }
}

impl GpsSeries {
    /// The point the track was at time `t`, or `None` when `t` is outside the
    /// span the pairing covers.
    pub fn point_at(&self, t: f64) -> Option<GpsMatch> {
        let first = *self.times.first()?;
        let last = *self.times.last()?;
        if t < first - SAME_TIME_EPSILON || t > last + SAME_TIME_EPSILON {
            return None;
        }
        let i = crate::panels::plot::nearest_index(&self.times, t);
        Some(GpsMatch {
            point: *self.points.get(i)?,
            time: *self.times.get(i)?,
        })
    }

    /// The time span the pairing covers, on the shared axis.
    pub fn span(&self) -> Option<(f64, f64)> {
        Some((*self.times.first()?, *self.times.last()?))
    }

    /// True if all coordinates in the track are identical.
    pub fn is_all_identical(&self) -> bool {
        if self.points.len() <= 1 {
            return false;
        }
        let first = self.points[0];
        self.points.iter().all(|p| {
            (p[0] - first[0]).abs() <= SAME_COORDINATE_EPSILON
                && (p[1] - first[1]).abs() <= SAME_COORDINATE_EPSILON
        })
    }
}

/// Pairs latitude and longitude signals into a GPS track, or refuses.
///
/// Longitude is mapped to X and Latitude to Y. If the signals do not share a raster,
/// Latitude is interpolated onto Longitude's timestamps over their overlapping span.
pub fn pair_gps(
    lat: &ChannelSignal,
    lat_offset: f64,
    lon: &ChannelSignal,
    lon_offset: f64,
    cross_file: bool,
    absolute_alignment: bool,
) -> Result<GpsSeries, GpsRefusal> {
    let xy_result = pair_xy(
        lon,
        lon_offset,
        lat,
        lat_offset,
        cross_file,
        absolute_alignment,
    );

    let xy_series = match xy_result {
        Ok(series) => series,
        Err(XyRefusal::NoSamples { axis: Axis::X }) => {
            return Err(GpsRefusal::NoSamples {
                coordinate: Coordinate::Longitude,
            })
        }
        Err(XyRefusal::NoSamples { axis: Axis::Y }) => {
            return Err(GpsRefusal::NoSamples {
                coordinate: Coordinate::Latitude,
            })
        }
        Err(XyRefusal::NoOverlap { x_span, y_span }) => {
            return Err(GpsRefusal::NoOverlap {
                lon_span: x_span,
                lat_span: y_span,
            })
        }
        Err(XyRefusal::CrossFileNeedsAbsoluteTime) => {
            return Err(GpsRefusal::CrossFileNeedsAbsoluteTime)
        }
        Err(XyRefusal::NothingValid { dropped }) => {
            return Err(GpsRefusal::NothingValid { dropped })
        }
    };

    // Validate coordinate ranges: Latitude in [-90.0, 90.0], Longitude in [-180.0, 180.0].
    let mut min_lat = f64::INFINITY;
    let mut max_lat = f64::NEG_INFINITY;
    let mut min_lon = f64::INFINITY;
    let mut max_lon = f64::NEG_INFINITY;

    for &[lon_val, lat_val] in &xy_series.points {
        if lat_val < min_lat {
            min_lat = lat_val;
        }
        if lat_val > max_lat {
            max_lat = lat_val;
        }
        if lon_val < min_lon {
            min_lon = lon_val;
        }
        if lon_val > max_lon {
            max_lon = lon_val;
        }
    }

    if min_lat < -90.0 || max_lat > 90.0 {
        return Err(GpsRefusal::LatitudeOutOfRange {
            min: min_lat,
            max: max_lat,
        });
    }

    if min_lon < -180.0 || max_lon > 180.0 {
        return Err(GpsRefusal::LongitudeOutOfRange {
            min: min_lon,
            max: max_lon,
        });
    }

    Ok(GpsSeries {
        points: xy_series.points,
        times: xy_series.times,
        pairing: xy_series.pairing,
        dropped: xy_series.dropped,
    })
}
