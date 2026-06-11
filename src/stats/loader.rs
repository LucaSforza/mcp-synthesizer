use anyhow::{Context, Result};
use chrono::DateTime;
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

    /// Load all `succeeded_full` / `succeeded_partial` observations for a range of test runs.
    ///
    /// For each test_run, reads the `synthesis_trial:by_test_run` ZSET (sorted by iteration),
    /// finds the LAST trial with result_type `succeeded_full` or `succeeded_partial`,
    /// and reads token/cost information from the `test_run:{id}` hash.
    ///
    /// When multiple trials succeed for the same test_run, the highest-iteration one wins.
    pub fn load_group(&mut self, label: &str, start: u64, end: u64) -> Result<ExperimentGroup> {
        let mut group = ExperimentGroup::new(label.to_string(), start, end);

        for test_run_id in start..=end {
            // Read test_run-level metadata (model_name, cost, tokens).
            let tr_fields = self.get_test_run_hash(test_run_id);
            let model_name = tr_fields.get("model_name").cloned();
            let cost_usd = tr_fields
                .get("cost_of_synthesis_USD")
                .and_then(|v| v.parse::<f64>().ok());
            let input_tokens = tr_fields
                .get("totalInputTokens")
                .and_then(|v| v.parse::<u64>().ok());
            let output_tokens = tr_fields
                .get("totalOutputTokens")
                .and_then(|v| v.parse::<u64>().ok());

            let trial_ids: Vec<String> = self
                .get_trial_ids_for_test_run(test_run_id);

            let mut best_gas: Option<GasObservation> = None;
            let mut iteration_times: Vec<(u64, String)> = Vec::new();

            for tid_str in &trial_ids {
                let fields: HashMap<String, String> = self
                    .get_trial_hash(tid_str);

                // Collect iteration timestamps for synth_time computation.
                if let (Some(iter_str), Some(ts)) = (fields.get("iteration"), fields.get("created_at"))
                {
                    if let Ok(iter) = iter_str.parse::<u64>() {
                        iteration_times.push((iter, ts.clone()));
                    }
                }

                let result_type = match fields.get("result_type") {
                    Some(rt) => rt.as_str(),
                    None => continue,
                };

                // Include both succeeded_full and succeeded_partial.
                if result_type != "succeeded_full" && result_type != "succeeded_partial" {
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

                // Read test_run hash for token/cost metadata.
                let tr_fields = self.get_test_run_hash(test_run_id);
                let total_input: u64 = tr_fields
                    .get("totalInputTokens")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let total_output: u64 = tr_fields
                    .get("totalOutputTokens")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                let cost: f64 = tr_fields
                    .get("cost_of_synthesis_USD")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.0);
                let model = tr_fields
                    .get("model_name")
                    .cloned()
                    .unwrap_or_default();
                let project: u64 = tr_fields
                    .get("project_id")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);

                best_gas = Some(GasObservation {
                    test_run_id,
                    trial_id,
                    gas,
                    total_tokens: total_input.saturating_add(total_output),
                    cost_of_synthesis_usd: cost,
                    model_name: model,
                    project_id: project,
                });
            }

            // Push best gas (last succeeded_full/partial, i.e. highest iteration).
            // Skip observations missing token/cost data (manual runs, not queue controller).
            if let Some(obs) = best_gas {
                if obs.total_tokens > 0 || obs.cost_of_synthesis_usd > 0.0 {
                    group.add_observation(obs);
                }
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

    /// Get all fields of a test_run hash.
    fn get_test_run_hash(&mut self, test_run_id: u64) -> HashMap<String, String> {
        let key = format!("test_run:{test_run_id}");
        match ::redis::cmd("HGETALL")
            .arg(&key)
            .query::<HashMap<String, String>>(&mut self.conn)
        {
            Ok(fields) => fields,
            Err(_) => HashMap::new(),
        }
    }
}
