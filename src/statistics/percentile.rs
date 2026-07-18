/// Reduces finite values using the upstream sorted linear-interpolation rule.
pub fn percentile(values: &[f64], fraction: f64) -> Option<f64> {
    if values.is_empty()
        || !(0.0..=1.0).contains(&fraction)
        || values.iter().any(|value| !value.is_finite())
    {
        return None;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);

    let index = (sorted.len() - 1) as f64 * fraction;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;

    Some(sorted[lower] + (sorted[upper] - sorted[lower]) * index.fract())
}
