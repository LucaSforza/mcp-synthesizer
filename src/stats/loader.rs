use anyhow::{Context, Result};
use std::collections::HashMap;

use crate::stats::types::{ExperimentGroup, GasObservation};

/// Loader that reads synthesis trial data from Redis.
pub struct RedisLoader {
    conn: redis::Connection,
}

impl RedisLoader {
    /// Open a new connection to Redis at the given URL.
    pub fn new(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)
            .with_context(|| format!("failed to open Redis at {url}"))?;
        let conn = client
            .get_connection()
            .context("failed to connect to Redis")?;
        Ok(Self { conn })
    }

    /// Load all `succeeded_full` observations for a range of test runs.
    pub fn load_group(&mut self, label: &str, start: u64, end: u64) -> Result<ExperimentGroup> {
        let mut group = ExperimentGroup::new(label.to_string(), start, end);

        for test_run_id in start..=end {
            let trial_ids: Vec<String> = self
                .get_trial_ids_for_test_run(test_run_id);

            let mut best_gas: Option<GasObservation> = None;

            for tid_str in &trial_ids {
                let fields: HashMap<String, String> = self
                    .get_trial_hash(tid_str);

                let result_type = match fields.get("result_type") {
                    Some(rt) => rt.as_str(),
                    None => continue,
                };

                // We only care about succeeded_full.
                if result_type != "succeeded_full" {
                    continue;
                }

                let gas_str = match fields.get("gas_of_implementation") {
                    Some(g) => g,
                    None => continue,
                };

                let gas: u64 = match gas_str.parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let trial_id: u64 = match tid_str.parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                best_gas = Some(GasObservation {
                    test_run_id,
                    trial_id,
                    gas,
                });
            }

            // Push best gas (last succeeded_full or only one).
            if let Some(obs) = best_gas {
                group.add_observation(obs);
            }
        }

        Ok(group)
    }

    /// Get trial IDs for a test_run sorted by iteration (ascending).
    fn get_trial_ids_for_test_run(&mut self, test_run_id: u64) -> Vec<String> {
        let key = format!("synthesis_trial:by_test_run:{test_run_id}");
        match ::redis::cmd("ZRANGE")
            .arg(&key)
            .arg(0i64)
            .arg(-1i64)
            .query::<Vec<String>>(&mut self.conn)
        {
            Ok(ids) => ids,
            Err(_) => Vec::new(),
        }
    }

    /// Get all fields of a synthesis trial hash.
    fn get_trial_hash(&mut self, trial_id: &str) -> HashMap<String, String> {
        let key = format!("synthesis_trial:{trial_id}");
        match ::redis::cmd("HGETALL")
            .arg(&key)
            .query::<HashMap<String, String>>(&mut self.conn)
        {
            Ok(fields) => fields,
            Err(_) => HashMap::new(),
        }
    }
}
