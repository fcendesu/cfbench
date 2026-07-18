/// Returns the mean absolute difference between consecutive finite points.
pub fn jitter(values: &[f64]) -> Option<f64> {
    if values.len() < 2 || values.iter().any(|value| !value.is_finite()) {
        return None;
    }

    let total_difference = values
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).abs())
        .sum::<f64>();

    let result = total_difference / (values.len() - 1) as f64;
    result.is_finite().then_some(result)
}
