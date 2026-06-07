use super::*;

// ---------------------------------------------------------------------------
// Mean
// ---------------------------------------------------------------------------

#[test]
fn test_mean_basic() {
    let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let result = compute_mean(&data);
    assert!((result - 3.0).abs() < 1e-10);
}

#[test]
fn test_mean_single() {
    let data = vec![42.0];
    let result = compute_mean(&data);
    assert!((result - 42.0).abs() < 1e-10);
}

#[test]
fn test_mean_empty() {
    let data: Vec<f64> = vec![];
    let result = compute_mean(&data);
    assert!((result - 0.0).abs() < 1e-10);
}

#[test]
fn test_mean_all_same() {
    let data = vec![5.0, 5.0, 5.0];
    let result = compute_mean(&data);
    assert!((result - 5.0).abs() < 1e-10);
}

// ---------------------------------------------------------------------------
// Variance & Std Dev
// ---------------------------------------------------------------------------

#[test]
fn test_variance_known() {
    // Population variance of [2,4,4,4,5,5,7,9] = 4.0
    let data = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
    let mean = compute_mean(&data);
    let var = compute_variance(&data, mean);
    assert!((var - 4.0).abs() < 1e-10, "variance={var} expected=4.0");
}

#[test]
fn test_variance_zero() {
    let data = vec![5.0, 5.0, 5.0];
    let mean = compute_mean(&data);
    let var = compute_variance(&data, mean);
    assert!((var - 0.0).abs() < 1e-10);
}

#[test]
fn test_std_dev_known() {
    let var = 4.0;
    let std_dev = compute_std_dev(var);
    assert!((std_dev - 2.0).abs() < 1e-10);
}

#[test]
fn test_variance_single_value() {
    let data = vec![10.0];
    let mean = compute_mean(&data);
    let var = compute_variance(&data, mean);
    assert!((var - 0.0).abs() < 1e-10);
}

// ---------------------------------------------------------------------------
// Median
// ---------------------------------------------------------------------------

#[test]
fn test_median_odd() {
    let data = vec![1.0, 3.0, 5.0];
    let result = compute_median(&data);
    assert!((result - 3.0).abs() < 1e-10);
}

#[test]
fn test_median_even() {
    let data = vec![1.0, 2.0, 3.0, 4.0];
    let result = compute_median(&data);
    assert!((result - 2.5).abs() < 1e-10);
}

#[test]
fn test_median_single() {
    let data = vec![7.0];
    let result = compute_median(&data);
    assert!((result - 7.0).abs() < 1e-10);
}

#[test]
fn test_median_empty() {
    let data: Vec<f64> = vec![];
    let result = compute_median(&data);
    assert!((result - 0.0).abs() < 1e-10);
}

#[test]
fn test_median_two_values() {
    let data = vec![10.0, 20.0];
    let result = compute_median(&data);
    assert!((result - 15.0).abs() < 1e-10);
}

// ---------------------------------------------------------------------------
// Quartiles
// ---------------------------------------------------------------------------

#[test]
fn test_quartiles_basic() {
    // Odd number of elements.
    let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let (q1, q3) = compute_quartiles(&data);
    assert!((q1 - 2.5).abs() < 1e-10, "q1={q1} expected=2.5");
    assert!((q3 - 5.5).abs() < 1e-10, "q3={q3} expected=5.5");
}

#[test]
fn test_quartiles_single() {
    let data = vec![42.0];
    let (q1, q3) = compute_quartiles(&data);
    assert!((q1 - 42.0).abs() < 1e-10);
    assert!((q3 - 42.0).abs() < 1e-10);
}

#[test]
fn test_quartiles_two_values() {
    let data = vec![10.0, 20.0];
    let (q1, q3) = compute_quartiles(&data);
    assert!((q1 - 12.5).abs() < 1e-10, "q1={q1} expected=12.5");
    assert!((q3 - 17.5).abs() < 1e-10, "q3={q3} expected=17.5");
}

#[test]
fn test_quartiles_empty() {
    let data: Vec<f64> = vec![];
    let (q1, q3) = compute_quartiles(&data);
    assert!((q1 - 0.0).abs() < 1e-10);
    assert!((q3 - 0.0).abs() < 1e-10);
}

// ---------------------------------------------------------------------------
// IQR
// ---------------------------------------------------------------------------

#[test]
fn test_iqr_basic() {
    let result = compute_iqr(2.5, 5.5);
    assert!((result - 3.0).abs() < 1e-10);
}

#[test]
fn test_iqr_zero() {
    let result = compute_iqr(3.0, 3.0);
    assert!((result - 0.0).abs() < 1e-10);
}

// ---------------------------------------------------------------------------
// Coefficient of Variation
// ---------------------------------------------------------------------------

#[test]
fn test_cv_basic() {
    let result = compute_cv(2.0, 10.0);
    assert!((result - 0.2).abs() < 1e-10);
}

#[test]
fn test_cv_zero_mean() {
    let result = compute_cv(1.0, 0.0);
    assert!((result - 0.0).abs() < 1e-10);
}

// ---------------------------------------------------------------------------
// Outlier Detection
// ---------------------------------------------------------------------------

#[test]
fn test_outliers_above() {
    // Data with an obvious outlier: [1, 2, 2, 3, 100].
    let data = vec![1.0, 2.0, 2.0, 3.0, 100.0];
    let obs: Vec<super::GasObservation> = data
        .iter()
        .enumerate()
        .map(|(i, _)| super::GasObservation {
            test_run_id: i as u64 + 1,
            trial_id: (i as u64 + 1) * 10,
            gas: data[i] as u64,
        })
        .collect();

    let (q1, q3) = compute_quartiles(&data);
    let iqr = compute_iqr(q1, q3);
    let outliers = detect_outliers(&data, &obs, q1, q3, iqr);

    // 100 should be an outlier (Q3 + 1.5*IQR).
    assert!(!outliers.is_empty(), "expected at least one outlier");
    let outlier_gases: Vec<u64> = outliers.iter().map(|o| o.gas).collect();
    assert!(outlier_gases.contains(&100), "expected 100 as outlier");
}

#[test]
fn test_outliers_below() {
    // Data with a low outlier: [-50, 10, 10, 10, 20].
    let data = vec![-50.0, 10.0, 10.0, 10.0, 20.0];
    let obs: Vec<super::GasObservation> = data
        .iter()
        .enumerate()
        .map(|(i, _)| super::GasObservation {
            test_run_id: i as u64 + 1,
            trial_id: (i as u64 + 1) * 10,
            gas: f64::max(data[i], 0.0) as u64,
        })
        .collect();

    let (q1, q3) = compute_quartiles(&data);
    let iqr = compute_iqr(q1, q3);
    let outliers = detect_outliers(&data, &obs, q1, q3, iqr);

    // -50 should be an outlier (Q1 - 1.5*IQR).
    assert!(!outliers.is_empty(), "expected at least one outlier");
}

#[test]
fn test_no_outliers() {
    // Contiguous data with no outliers.
    let data = vec![10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0];
    let obs: Vec<super::GasObservation> = data
        .iter()
        .enumerate()
        .map(|(i, _)| super::GasObservation {
            test_run_id: i as u64 + 1,
            trial_id: (i as u64 + 1) * 10,
            gas: data[i] as u64,
        })
        .collect();

    let (q1, q3) = compute_quartiles(&data);
    let iqr = compute_iqr(q1, q3);
    let outliers = detect_outliers(&data, &obs, q1, q3, iqr);

    assert!(outliers.is_empty(), "expected no outliers, got {}", outliers.len());
}

#[test]
fn test_outliers_empty_data() {
    let data: Vec<f64> = vec![];
    let obs: Vec<super::GasObservation> = vec![];
    let outliers = detect_outliers(&data, &obs, 0.0, 0.0, 0.0);
    assert!(outliers.is_empty());
}

// ---------------------------------------------------------------------------
// compute_statistics (integration)
// ---------------------------------------------------------------------------

#[test]
fn test_compute_statistics_integration() {
    use super::ExperimentGroup;

    let mut group = ExperimentGroup::new("test".to_string(), 1, 5);
    for i in 1..=5 {
        group.add_observation(super::GasObservation {
            test_run_id: i,
            trial_id: i * 10,
            gas: (i as u64) * 1000, // 1000, 2000, 3000, 4000, 5000
        });
    }

    let s = compute_statistics(&group);
    assert_eq!(s.count, 5);
    assert!((s.mean - 3000.0).abs() < 1e-10);
    assert!((s.median - 3000.0).abs() < 1e-10);
    assert_eq!(s.min, 1000);
    assert_eq!(s.max, 5000);
    assert!(s.outliers.is_empty());
}
