use cfbench::statistics::{jitter, percentile};

#[test]
fn percentile_uses_upstream_linear_interpolation() {
    assert_eq!(percentile(&[10.0, 0.0, 30.0, 20.0], 0.5), Some(15.0));
    assert_eq!(percentile(&[0.0, 10.0, 20.0, 30.0], 0.9), Some(27.0));
    assert_eq!(percentile(&[], 0.5), None);
}

#[test]
fn percentile_rejects_non_finite_values_and_invalid_fractions() {
    assert_eq!(percentile(&[1.0, f64::NAN], 0.5), None);
    assert_eq!(percentile(&[1.0], -0.1), None);
    assert_eq!(percentile(&[1.0], 1.1), None);
    assert_eq!(percentile(&[1.0], f64::NAN), None);
}

#[test]
fn jitter_requires_two_finite_points() {
    assert_eq!(jitter(&[10.0]), None);
    assert_eq!(jitter(&[10.0, 14.0, 12.0]), Some(3.0));
    assert_eq!(jitter(&[10.0, f64::NAN]), None);
}

#[test]
fn jitter_preserves_measurement_order() {
    assert_eq!(jitter(&[1.0, 5.0, 2.0, 8.0]), Some(13.0 / 3.0));
    assert_eq!(jitter(&[4.0, 4.0]), Some(0.0));
}
