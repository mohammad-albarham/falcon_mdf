//! Turning numbers into the text a readout shows.
//!
//! Both of these exist because a column in a table is narrower than a
//! measurement is precise. `0.00042` in a cell three characters wide is a
//! lie by truncation; `420 uV` is the same number, said in a width that
//! fits. Neither function is used for exported values — a CSV carries the
//! full precision — only for what is on screen.

/// The SI prefixes this renders, from pico to tera, with the exponent each
/// stands for. `u` rather than the Greek letter: a measurement file's units
/// are ASCII, and mixing the two in one column looks like a bug.
const PREFIXES: [(&str, i32); 9] = [
    ("p", -12),
    ("n", -9),
    ("u", -6),
    ("m", -3),
    ("", 0),
    ("k", 3),
    ("M", 6),
    ("G", 9),
    ("T", 12),
];

/// A measured value with an SI prefix and three significant digits.
///
/// Zero has no prefix. A value past the ends of the prefix table falls back
/// to scientific notation rather than being scaled by a prefix nobody reads.
/// A non-finite value is named as itself, with no unit: `NaN` volts is not a
/// measurement, and dressing it as one would hide that.
pub fn engineering(value: f64, unit: &str) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_string()
        } else {
            "inf".to_string()
        };
    }
    if value == 0.0 {
        return with_unit("0", unit);
    }

    let exponent = value.abs().log10().floor() as i32;
    // Prefixes step every three decades, and the one that applies is the
    // largest whose exponent the value has reached.
    let prefix = PREFIXES
        .iter()
        .rev()
        .find(|(_, power)| exponent >= *power)
        .filter(|(_, power)| exponent < power + 3 || *power == 12);

    let Some((symbol, power)) = prefix else {
        // Below pico, or above what the last prefix covers.
        return with_unit(&format!("{value:.2e}"), unit);
    };
    if exponent >= 15 {
        return with_unit(&format!("{value:.2e}"), unit);
    }

    let scaled = value / 10f64.powi(*power);
    // Three significant digits: the decimals left over after the integer
    // part has taken its share.
    let decimals = match scaled.abs().log10().floor() as i32 {
        0 => 2,
        1 => 1,
        _ => 0,
    };
    // The prefix belongs to the unit, not to the number: "1.50 mV", never
    // "1.50 m V". With no unit the prefix stands alone as the unit does.
    format!("{scaled:.decimals$}{}", suffix(symbol, unit))
}

/// The space-and-suffix that follows the number: the prefix joined to the
/// unit, or nothing at all when there is neither.
fn suffix(symbol: &str, unit: &str) -> String {
    if symbol.is_empty() && unit.is_empty() {
        String::new()
    } else {
        format!(" {symbol}{unit}")
    }
}

/// A duration in seconds as a person says it out loud.
///
/// Under a minute the seconds carry three decimals, because a measurement
/// interval is often milliseconds; past that the decimals stop earning their
/// width and the larger unit is what the reader wanted.
pub fn duration(seconds: f64) -> String {
    if seconds.is_nan() {
        return "NaN".to_string();
    }
    if seconds.is_infinite() {
        return if seconds.is_sign_negative() {
            "-inf".to_string()
        } else {
            "inf".to_string()
        };
    }
    let sign = if seconds < 0.0 { "-" } else { "" };
    let seconds = seconds.abs();

    if seconds < 60.0 {
        return format!("{sign}{seconds:.3} s");
    }
    if seconds < 3600.0 {
        let minutes = (seconds / 60.0).floor();
        let rest = seconds - minutes * 60.0;
        return format!("{sign}{minutes:.0} min {rest:02.0} s");
    }
    let hours = (seconds / 3600.0).floor();
    let minutes = ((seconds - hours * 3600.0) / 60.0).floor();
    format!("{sign}{hours:.0} h {minutes:02.0} min")
}

/// Joins a number and a unit, leaving no trailing space when the channel has
/// no unit — which is most bus-log channels.
fn with_unit(number: &str, unit: &str) -> String {
    if unit.is_empty() {
        number.to_string()
    } else {
        format!("{number} {unit}")
    }
}
