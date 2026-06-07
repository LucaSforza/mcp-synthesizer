/// A single gas observation from a synthesis trial.
#[derive(Debug, Clone)]
pub struct GasObservation {
    pub test_run_id: u64,
    pub trial_id: u64,
    pub gas: u64,
}

/// An experiment group: a set of test runs identified by label + range.
#[derive(Debug, Clone)]
pub struct ExperimentGroup {
    pub label: String,
    pub test_run_start: u64,
    pub test_run_end: u64,
    pub observations: Vec<GasObservation>,
}

impl ExperimentGroup {
    /// Create a new empty group with the given label and range.
    pub fn new(label: String, start: u64, end: u64) -> Self {
        Self {
            label,
            test_run_start: start,
            test_run_end: end,
            observations: Vec::new(),
        }
    }

    /// Add a single observation to this group.
    pub fn add_observation(&mut self, obs: GasObservation) {
        self.observations.push(obs);
    }

    /// Return the gas values as a sorted `Vec<f64>` for statistical computation.
    #[allow(dead_code)]
    pub fn sorted_gas_values(&self) -> Vec<f64> {
        let mut values: Vec<f64> = self.observations.iter().map(|o| o.gas as f64).collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap());
        values
    }

    /// Return true if this group has no observations.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }

    /// Number of observations in this group.
    pub fn count(&self) -> usize {
        self.observations.len()
    }
}

/// Statistics computed for a single experiment group.
#[derive(Debug, Clone)]
pub struct GroupStatistics {
    pub label: String,
    pub count: usize,
    pub mean: f64,
    pub median: f64,
    pub variance: f64,
    pub std_dev: f64,
    pub min: u64,
    pub max: u64,
    pub q1: f64,
    pub q3: f64,
    pub iqr: f64,
    pub coefficient_of_variation: f64,
    pub outliers: Vec<Outlier>,
}

/// A single outlier observation.
#[derive(Debug, Clone)]
pub struct Outlier {
    pub test_run_id: u64,
    pub trial_id: u64,
    pub gas: u64,
}
