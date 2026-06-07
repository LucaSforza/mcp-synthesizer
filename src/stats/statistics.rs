use crate::stats::types::{ExperimentGroup, GroupStatistics, GasObservation, Outlier};

/// Compute the arithmetic mean of a sorted slice of values.
pub fn compute_mean(data: &[f64]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let sum: f64 = data.iter().sum();
    sum / data.len() as f64
}

/// Compute population variance.
pub fn compute_variance(data: &[f64], mean: f64) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = data.iter().map(|x| (x - mean).powi(2)).sum();
    sum_sq / data.len() as f64
}

/// Compute population standard deviation from variance.
pub fn compute_std_dev(variance: f64) -> f64 {
    variance.sqrt()
}

/// Compute median from sorted data.
pub fn compute_median(data: &[f64]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let len = data.len();
    if len % 2 == 0 {
        let mid = len / 2;
        (data[mid - 1] + data[mid]) / 2.0
    } else {
        data[len / 2]
    }
}

/// Compute Q1 and Q3 from sorted data using linear interpolation.
pub fn compute_quartiles(data: &[f64]) -> (f64, f64) {
    if data.is_empty() {
        return (0.0, 0.0);
    }
    if data.len() == 1 {
        return (data[0], data[0]);
    }

    let q1 = percentile(data, 0.25);
    let q3 = percentile(data, 0.75);
    (q1, q3)
}

/// Compute the p-th percentile using linear interpolation (C = 1 method).
fn percentile(data: &[f64], p: f64) -> f64 {
    let len = data.len();
    if len == 0 {
        return 0.0;
    }
    if len == 1 {
        return data[0];
    }

    // Use the C = 1 interpolation method (popular for box plots).
    let n = len as f64;
    let rank = p * (n - 1.0);
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    let frac = rank - lower as f64;

    let lower_val = data[lower];
    let upper_val = if upper >= len { lower_val } else { data[upper] };

    lower_val + frac * (upper_val - lower_val)
}

/// Compute IQR from Q1 and Q3.
pub fn compute_iqr(q1: f64, q3: f64) -> f64 {
    q3 - q1
}

/// Compute coefficient of variation (std_dev / mean).
pub fn compute_cv(std_dev: f64, mean: f64) -> f64 {
    if mean == 0.0 {
        return 0.0;
    }
    std_dev / mean
}

/// Detect outliers using the standard box-plot rule.
/// `data` and `observations` must be in the same order (sorted by gas ascending).
pub fn detect_outliers(
    data: &[f64],
    observations: &[GasObservation],
    q1: f64,
    q3: f64,
    iqr: f64,
) -> Vec<Outlier> {
    let lower_bound = q1 - 1.5 * iqr;
    let upper_bound = q3 + 1.5 * iqr;

    data.iter()
        .zip(observations.iter())
        .filter_map(|(val, obs)| {
            if *val < lower_bound || *val > upper_bound {
                Some(Outlier {
                    test_run_id: obs.test_run_id,
                    trial_id: obs.trial_id,
                    gas: obs.gas,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Build a `(sorted_gas_values, sorted_observations)` pair where both are
/// sorted by gas ascending, so outlier detection stays aligned.
fn sorted_observations(group: &ExperimentGroup) -> (Vec<f64>, Vec<GasObservation>) {
    let mut pairs: Vec<(f64, &GasObservation)> = group
        .observations
        .iter()
        .map(|o| (o.gas as f64, o))
        .collect();
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let data: Vec<f64> = pairs.iter().map(|(v, _)| *v).collect();
    let obs: Vec<GasObservation> = pairs.iter().map(|(_, o)| (*o).clone()).collect();
    (data, obs)
}

/// Compute a full statistics summary for a group.
pub fn compute_statistics(group: &ExperimentGroup) -> GroupStatistics {
    let (data, sorted_obs) = sorted_observations(group);
    let count = data.len();
    let min = data.first().copied().unwrap_or(0.0) as u64;
    let max = data.last().copied().unwrap_or(0.0) as u64;
    let mean = compute_mean(&data);
    let median = compute_median(&data);
    let variance = compute_variance(&data, mean);
    let std_dev = compute_std_dev(variance);
    let (q1, q3) = compute_quartiles(&data);
    let iqr_val = compute_iqr(q1, q3);
    let cv = compute_cv(std_dev, mean);
    let outliers = detect_outliers(&data, &sorted_obs, q1, q3, iqr_val);

    GroupStatistics {
        label: group.label.clone(),
        count,
        mean,
        median,
        variance,
        std_dev,
        min,
        max,
        q1,
        q3,
        iqr: iqr_val,
        coefficient_of_variation: cv,
        outliers,
    }
}

#[cfg(test)]
#[path = "statistics_test.rs"]
mod tests;
