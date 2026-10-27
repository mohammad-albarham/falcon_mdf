//! The plot's visible time window, and the arithmetic that moves it.
//!
//! Kept apart from the panel so the rules can be tested without a window:
//! zooming keeps the instant under the pointer where it is, a step never
//! leaves the recording, and a window cannot shrink to nothing.

/// A visible time range on the plot's x axis, `lo < hi`, in plot seconds.
pub type Window = (f64, f64);

/// The narrowest window a zoom may reach, as a fraction of the recording.
/// Past this the decimator has single samples to draw and the tick labels
/// run out of digits; zooming further only makes the view harder to leave.
const MIN_FRACTION: f64 = 1e-7;

/// The narrowest window over `full`, never below a nanosecond.
fn min_span(full: Window) -> f64 {
    ((full.1 - full.0) * MIN_FRACTION).max(1e-9)
}

/// `window` scaled by `factor` around `anchor`: below 1 zooms in, above 1
/// zooms out. The anchor keeps its position in the window, so the instant
/// under the pointer stays under the pointer.
pub fn zoom(window: Window, factor: f64, anchor: f64, full: Window) -> Window {
    let span = window.1 - window.0;
    let new_span = (span * factor).max(min_span(full));
    let at = if span > 0.0 {
        ((anchor - window.0) / span).clamp(0.0, 1.0)
    } else {
        0.5
    };
    let lo = anchor - at * new_span;
    clamp((lo, lo + new_span), full)
}

/// `window` moved by `fraction` of its own width; positive is later.
pub fn pan(window: Window, fraction: f64, full: Window) -> Window {
    let shift = (window.1 - window.0) * fraction;
    clamp((window.0 + shift, window.1 + shift), full)
}

/// `window` kept inside the recording. A window at least as wide as the
/// recording becomes the recording; a narrower one keeps its width and
/// slides back in, so stepping past the end stops there rather than
/// shrinking the view.
pub fn clamp(window: Window, full: Window) -> Window {
    let (mut lo, mut hi) = if window.0 <= window.1 {
        window
    } else {
        (window.1, window.0)
    };
    let has_extent = full.0 < full.1;
    if !has_extent {
        return (lo, hi);
    }
    let span = hi - lo;
    if span >= full.1 - full.0 {
        return full;
    }
    if lo < full.0 {
        lo = full.0;
        hi = lo + span;
    }
    if hi > full.1 {
        hi = full.1;
        lo = hi - span;
    }
    (lo, hi)
}

/// The window two instants span — a dragged range or the two cursors —
/// with a margin of `pad` of its width on each side, so the edges of what
/// was picked are not drawn on the frame. `None` when the two are too close
/// together to be a window.
pub fn between(a: f64, b: f64, pad: f64, full: Window) -> Option<Window> {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let span = hi - lo;
    let wide_enough = span > min_span(full);
    if !wide_enough {
        return None;
    }
    Some(clamp((lo - span * pad, hi + span * pad), full))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: Window = (0.0, 100.0);

    fn close(a: Window, b: Window) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    #[test]
    fn zoom_keeps_the_anchor_in_place() {
        let w = zoom((20.0, 60.0), 0.5, 30.0, FULL);
        assert!(close(w, (25.0, 45.0)), "{w:?}");
        // 30 was a quarter of the way in, and still is.
        assert!(((30.0 - w.0) / (w.1 - w.0) - 0.25).abs() < 1e-12);
    }

    #[test]
    fn zooming_out_past_the_recording_shows_all_of_it() {
        assert_eq!(zoom((40.0, 60.0), 50.0, 50.0, FULL), FULL);
    }

    #[test]
    fn zoom_stops_at_a_minimum_width() {
        let w = zoom((50.0, 50.000_01), 1e-9, 50.0, FULL);
        assert!(w.1 - w.0 >= 100.0 * MIN_FRACTION * 0.999, "{w:?}");
    }

    #[test]
    fn pan_stops_at_the_ends_without_shrinking() {
        assert!(close(pan((80.0, 95.0), 1.0, FULL), (85.0, 100.0)));
        assert!(close(pan((2.0, 12.0), -0.5, FULL), (0.0, 10.0)));
        assert!(close(pan((10.0, 20.0), 0.25, FULL), (12.5, 22.5)));
    }

    #[test]
    fn clamp_orders_a_reversed_window() {
        assert!(close(clamp((30.0, 10.0), FULL), (10.0, 30.0)));
    }

    #[test]
    fn between_pads_and_orders_the_two_instants() {
        let w = between(60.0, 40.0, 0.05, FULL).unwrap();
        assert!(close(w, (39.0, 61.0)), "{w:?}");
        assert_eq!(between(5.0, 5.0, 0.05, FULL), None);
    }

    #[test]
    fn an_empty_recording_leaves_the_window_alone() {
        assert_eq!(clamp((1.0, 2.0), (5.0, 5.0)), (1.0, 2.0));
    }
}
