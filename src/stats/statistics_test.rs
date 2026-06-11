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
        .map(|(i, _)| super::GasObservation::new(i as u64 + 1, (i as u64 + 1) * 10, data[i] as u64))
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
        .map(|(i, _)| super::GasObservation::new(i as u64 + 1, (i as u64 + 1) * 10, f64::max(data[i], 0.0) as u64))
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
        .map(|(i, _)| super::GasObservation::new(i as u64 + 1, (i as u64 + 1) * 10, data[i] as u64))
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
        group.add_observation(super::GasObservation::new(i, i * 10, (i as u64) * 1000));
    }

    let s = compute_statistics(&group);
    assert_eq!(s.count, 5);
    assert!((s.mean - 3000.0).abs() < 1e-10);
    assert!((s.median - 3000.0).abs() < 1e-10);
    assert_eq!(s.min, 1000);
    assert_eq!(s.max, 5000);
    assert!(s.outliers.is_empty());
}

// ---------------------------------------------------------------------------
// Knee Detection
// ---------------------------------------------------------------------------

#[test]
fn test_detect_knee_basic() {
    // L-shaped curve: knee near the bend point.
    let x = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let y = vec![10.0, 9.5, 9.0, 8.0, 5.0, 3.0, 2.0, 1.5, 1.0, 0.5, 0.0];
    let knee = detect_knee(&x, &y);
    assert!(knee.is_some());
    let (kx, ky) = knee.unwrap();
    // Knee is the point with max perpendicular distance from (first, last) chord.
    assert!((kx - 5.0).abs() < 0.1, "knee x={kx} expected ~5.0");
    assert!((ky - 3.0).abs() < 0.1, "knee y={ky} expected ~3.0");
}

#[test]
fn test_detect_knee_insufficient_points() {
    assert!(detect_knee(&[1.0, 2.0], &[1.0, 2.0]).is_none());
}

#[test]
fn test_detect_knee_empty() {
    assert!(detect_knee(&[], &[]).is_none());
}

#[test]
fn test_detect_knee_linear() {
    // Perfectly linear: no strong knee, but function still returns a point.
    let x = vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0];
    let y = vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
    assert!(detect_knee(&x, &y).is_some());
}

// ---------------------------------------------------------------------------
// Pareto Frontier
// ---------------------------------------------------------------------------

#[test]
fn test_pareto_frontier_basic() {
    let obs = vec![
        GasObservation::new(1, 1, 100), // cost=0, tokens=0
        GasObservation::new(2, 2, 200),
        GasObservation::new(3, 3, 150),
    ];
    let frontier = compute_pareto_frontier(&obs, |o| o.cost_of_synthesis_usd, |o| o.gas as f64);
    // All have zero cost/tokens → filtered out → empty frontier.
    assert!(frontier.is_empty());
}

#[test]
fn test_pareto_frontier_with_costs() {
    let mut o1 = GasObservation::new(1, 1, 100);
    o1.cost_of_synthesis_usd = 0.10;
    o1.total_tokens = 1000;
    let mut o2 = GasObservation::new(2, 2, 200);
    o2.cost_of_synthesis_usd = 0.05;
    o2.total_tokens = 500;
    let mut o3 = GasObservation::new(3, 3, 150);
    o3.cost_of_synthesis_usd = 0.08;
    o3.total_tokens = 800;
    let obs = vec![o1, o2, o3];
    let frontier = compute_pareto_frontier(&obs, |o| o.cost_of_synthesis_usd, |o| o.gas as f64);
    // All 3 are non-dominated: each has a unique cost/gas trade-off.
    assert_eq!(frontier.len(), 3);
}

#[test]
fn test_pareto_frontier_dominated() {
    let mut o1 = GasObservation::new(1, 1, 100);
    o1.cost_of_synthesis_usd = 0.10;
    o1.total_tokens = 1000;
    let mut o2 = GasObservation::new(2, 2, 200);
    o2.cost_of_synthesis_usd = 0.05;
    o2.total_tokens = 500;
    let mut o3 = GasObservation::new(3, 3, 300);
    o3.cost_of_synthesis_usd = 0.15;
    o3.total_tokens = 300;
    let obs = vec![o1, o2, o3];
    let frontier = compute_pareto_frontier(&obs, |o| o.cost_of_synthesis_usd, |o| o.gas as f64);
    // obs[2] (cost=0.15, gas=300) dominated by obs[0] (cost=0.10, gas=100).
    assert_eq!(frontier.len(), 2);
    let frontier_ids: Vec<u64> = frontier.iter().map(|o| o.test_run_id).collect();
    assert!(!frontier_ids.contains(&3));
}

#[test]
fn test_pareto_frontier_empty() {
    let obs: Vec<GasObservation> = vec![];
    assert!(compute_pareto_frontier(&obs, |o| o.cost_of_synthesis_usd, |o| o.gas as f64).is_empty());
}

#[test]
fn test_pareto_frontier_tokens() {
    let mut o1 = GasObservation::new(1, 1, 100);
    o1.total_tokens = 1000;
    o1.cost_of_synthesis_usd = 0.10;
    let mut o2 = GasObservation::new(2, 2, 200);
    o2.total_tokens = 500;
    o2.cost_of_synthesis_usd = 0.05;
    let mut o3 = GasObservation::new(3, 3, 150);
    o3.total_tokens = 800;
    o3.cost_of_synthesis_usd = 0.08;
    let obs = vec![o1, o2, o3];
    // Token-based frontier: all 3 have unique token/gas trade-offs.
    let token_frontier = compute_pareto_frontier(&obs, |o| o.total_tokens as f64, |o| o.gas as f64);
    assert_eq!(token_frontier.len(), 3);
    // Cost-based frontier: all 3 still non-dominated.
    let cost_frontier = compute_pareto_frontier(&obs, |o| o.cost_of_synthesis_usd, |o| o.gas as f64);
    assert_eq!(cost_frontier.len(), 3);
}
