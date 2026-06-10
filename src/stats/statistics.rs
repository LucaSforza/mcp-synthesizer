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

/// Detect the knee point in a curve using maximum perpendicular distance.
///
/// Sorts points by X ascending, then finds the point with the greatest
/// perpendicular distance from the chord connecting the first and last points.
/// Returns `Some((knee_x, knee_y))` for ≥3 points, `None` otherwise.
pub fn detect_knee(x: &[f64], y: &[f64]) -> Option<(f64, f64)> {
    if x.len() < 3 || y.len() < 3 || x.len() != y.len() {
        return None;
    }

    let mut points: Vec<(f64, f64)> = x.iter().copied().zip(y.iter().copied()).collect();
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let first = *points.first()?;
    let last = *points.last()?;

    let dx = last.0 - first.0;
    let dy = last.1 - first.1;
    let line_len_sq = dx * dx + dy * dy;

    if line_len_sq < f64::EPSILON {
        return None;
    }

    let mut max_dist = -1.0_f64;
    let mut knee = None;

    for &(px, py) in &points {
        // Perpendicular distance from point (px, py) to line (first → last).
        let num = ((last.0 - first.0) * (first.1 - py) - (first.0 - px) * (last.1 - first.1)).abs();
        let dist = num / line_len_sq.sqrt();

        if dist > max_dist {
            max_dist = dist;
            knee = Some((px, py));
        }
    }

    knee
}

/// Compute the Pareto frontier for cost vs gas (both to be minimized).
///
/// An observation is Pareto-optimal if no other observation has
/// lower-or-equal cost AND lower-or-equal gas, with at least one strict improvement.
/// Observations with zero tokens AND zero cost are excluded from the analysis.
pub fn compute_pareto_frontier(observations: &[GasObservation]) -> Vec<&GasObservation> {
    let candidates: Vec<&GasObservation> = observations
        .iter()
        .filter(|o| o.total_tokens > 0 || o.cost_of_synthesis_usd > 0.0)
        .collect();

    if candidates.is_empty() {
        return Vec::new();
    }

    let mut frontier: Vec<&GasObservation> = Vec::new();

    'outer: for obs in &candidates {
        for other in &candidates {
            let same = std::ptr::eq(*other, *obs);
            if same {
                continue;
            }
            let cost_leq = other.cost_of_synthesis_usd <= obs.cost_of_synthesis_usd;
            let gas_leq = (other.gas as f64) <= (obs.gas as f64);
            let strictly_better = other.cost_of_synthesis_usd < obs.cost_of_synthesis_usd
                || (other.gas as f64) < (obs.gas as f64);
            if cost_leq && gas_leq && strictly_better {
                continue 'outer;
            }
        }
        frontier.push(obs);
    }

    frontier.sort_by(|a, b| {
        a.cost_of_synthesis_usd
            .partial_cmp(&b.cost_of_synthesis_usd)
            .unwrap()
    });

    frontier
}

#[cfg(test)]
#[path = "statistics_test.rs"]
mod tests;
