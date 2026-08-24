/// The value below which `fraction` of the samples fall, by linear
/// interpolation between the two neighbouring ranks — the definition
/// numpy and asammdf use, so the number agrees with the tool people
/// check against.
///
/// `fraction` is clamped to 0.0..=1.0. `values` need not be sorted and is
/// not modified. Returns `None` for an empty slice.
pub fn percentile(values: &[f64], fraction: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));

    let fraction = fraction.clamp(0.0, 1.0);
    let rank = fraction * (sorted.len() - 1) as f64;

    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;

    if lower == upper {
        return Some(sorted[lower]);
    }

    let weight = rank - lower as f64;
    Some(sorted[lower] + (sorted[upper] - sorted[lower]) * weight)
}
