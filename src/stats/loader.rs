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

    /// Load all `succeeded_full` observations for a range of test runs.
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

                // We only care about succeeded_full for gas extraction.
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
                    synth_time_seconds: None, // will be set after the loop
                    model_name: model_name.clone(),
                    cost_usd,
                    input_tokens,
                    output_tokens,
                });
            }

            // Compute synth_time: difference between the latest and earliest
            // trial timestamps for this test run.  Works for any number of
            // iterations (single-iteration runs yield 0.0).
            let synth_time = compute_synth_time(test_run_id, &iteration_times);

            // Push best gas (last succeeded_full or only one) with synth_time attached.
            if let Some(obs) = best_gas {
                group.add_observation(GasObservation {
                    synth_time_seconds: synth_time,
                    ..obs
                });
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

/// Compute synth_time as the wall-clock seconds between the earliest and latest
/// trial timestamps for a test run.  Returns `None` when no timestamps are
/// available (no trials at all), `Some(0.0)` for a single trial, and
/// `Some(positive f64)` otherwise.
fn compute_synth_time(test_run_id: u64, iteration_times: &[(u64, String)]) -> Option<f64> {
    if iteration_times.is_empty() {
        eprintln!(
            "[WARNING] test_run {test_run_id}: no trials found, cannot compute synth_time"
        );
        return None;
    }

    // RFC 3339 strings are lexicographically sortable, so we can compare
    // them directly to find the earliest and latest timestamps.
    let (_, earliest) = iteration_times.iter().min_by(|a, b| a.1.cmp(&b.1))?;
    let (_, latest) = iteration_times.iter().max_by(|a, b| a.1.cmp(&b.1))?;

    let t1 = DateTime::parse_from_rfc3339(earliest).ok()?;
    let t2 = DateTime::parse_from_rfc3339(latest).ok()?;

    Some((t2.timestamp_millis() - t1.timestamp_millis()) as f64 / 1000.0)
}
