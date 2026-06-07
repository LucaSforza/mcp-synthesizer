//! Redis queue operations for the synthesis job queue.
//!
//! Reads jobs from Redis sorted set `cluster_runs` (priority queue).
//! Each entry references a Redis hash with job metadata.

use anyhow::{Context, Result, bail};
use redis::Commands;
use std::collections::HashMap;

/// Execution mode for a synthesis job: cluster (Slurm + tunnel) or API.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExecutionMode {
    Cluster,
    Api,
}

/// Metadata for a synthesis job loaded from Redis.
#[derive(Debug)]
pub struct JobMetadata {
    pub model_name: String,
    pub seed: String,
    pub project: String,
    pub prompt: String,
    pub execution_mode: ExecutionMode,
    pub api_url: Option<String>,
}

/// Thin wrapper around Redis connection for queue operations.
pub struct QueueClient {
    conn: redis::Connection,
}

impl QueueClient {
    /// Open connection to Redis.
    pub fn open(url: &str) -> Result<Self> {
        let client =
            redis::Client::open(url).with_context(|| format!("failed to open Redis at {url}"))?;
        let conn = client
            .get_connection()
            .context("failed to connect to Redis")?;
        Ok(Self { conn })
    }

    /// Lightweight connectivity check. Returns `Ok(())` if Redis responds.
    pub fn ping(&mut self) -> Result<()> {
        ::redis::cmd("PING").query::<()>(&mut self.conn)?;
        Ok(())
    }

    /// Read highest-priority job from `cluster_runs` without removing.
    /// Returns `(member, score)` where member is `"{model_name}:{job_id}"`.
    pub fn peek_job(&mut self) -> Result<Option<(String, f64)>> {
        let results: Vec<(String, f64)> = redis::cmd("ZREVRANGE")
            .arg("cluster_runs")
            .arg("0")
            .arg("0")
            .arg("WITHSCORES")
            .query(&mut self.conn)?;
        Ok(results.into_iter().next())
    }

    /// Remove a specific member from `cluster_runs`.
    pub fn remove_job(&mut self, member: &str) -> Result<()> {
        redis::cmd("ZREM")
            .arg("cluster_runs")
            .arg(member)
            .query::<()>(&mut self.conn)?;
        Ok(())
    }

    /// Check if the newest test_run for the project has a `succeeded_full` trial.
    pub fn check_succeeded_full(&mut self, project_name: &str) -> Result<bool> {
        let pid_key = format!("project:name:{project_name}");
        let project_id: Option<String> = redis::cmd("GET").arg(&pid_key).query(&mut self.conn)?;
        let project_id = match project_id {
            Some(id) => id,
            None => return Ok(false),
        };

        // Find the newest test_run (highest ID) for this project.
        let test_run_ids: Vec<String> = redis::cmd("SMEMBERS")
            .arg(format!("test_run:by_project:{project_id}"))
            .query(&mut self.conn)?;
        let max_test_run_id = match test_run_ids
            .iter()
            .filter_map(|id| id.parse::<i64>().ok())
            .max()
        {
            Some(id) => id,
            None => return Ok(false),
        };

        // Check if any trial in that test_run has result_type == "succeeded_full".
        let trial_ids: Vec<String> = redis::cmd("ZRANGE")
            .arg(format!("synthesis_trial:by_test_run:{max_test_run_id}"))
            .arg("0")
            .arg("-1")
            .query(&mut self.conn)?;
        for tid_str in &trial_ids {
            let fields: HashMap<String, String> = redis::cmd("HGETALL")
                .arg(format!("synthesis_trial:{tid_str}"))
                .query(&mut self.conn)?;
            if fields.get("result_type").map(|s| s.as_str()) == Some("succeeded_full") {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Load job metadata from Redis hash `{model_name}:{job_id}`.
    /// Validates all required fields exist.
    ///
    /// `model_name_param` is parsed from the queue key (`{model}:{job_id}`).
    /// For cluster mode this IS the model name. For API mode the actual model
    /// name is read from the hash `model` field.
    pub fn load_job(&mut self, model_name_param: &str, job_id: i64) -> Result<JobMetadata> {
        let key = format!("{model_name_param}:{job_id}");
        let fields: HashMap<String, String> = self.conn.hgetall(&key)?;
        if fields.is_empty() {
            bail!("job metadata not found for key '{key}'");
        }
        let seed = fields
            .get("seed")
            .cloned()
            .context("missing 'seed' field in job metadata")?;
        let project = fields
            .get("project")
            .cloned()
            .context("missing 'project' field in job metadata")?;
        let prompt = fields
            .get("prompt")
            .cloned()
            .context("missing 'prompt' field in job metadata")?;

        let execution_mode = match fields.get("execution_mode").map(|s| s.as_str()) {
            Some("api") => ExecutionMode::Api,
            _ => ExecutionMode::Cluster,
        };

        let api_url = fields.get("api_url").cloned();

        let model_name = match execution_mode {
            ExecutionMode::Cluster => model_name_param.to_string(),
            ExecutionMode::Api => fields
                .get("model")
                .cloned()
                .context("missing 'model' field for API job")?,
        };

        Ok(JobMetadata {
            model_name,
            seed,
            project,
            prompt,
            execution_mode,
            api_url,
        })
    }
}
