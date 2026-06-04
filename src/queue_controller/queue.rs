//! Redis queue operations for the synthesis job queue.
//!
//! Reads jobs from Redis sorted set `cluster_runs` (priority queue).
//! Each entry references a Redis hash with job metadata.

use anyhow::{bail, Context, Result};
use redis::Commands;
use std::collections::HashMap;

/// Metadata for a synthesis job loaded from Redis.
#[derive(Debug)]
pub struct JobMetadata {
    pub model_name: String,
    pub seed: String,
    pub project: String,
    pub prompt: String,
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
        let conn = client.get_connection().context("failed to connect to Redis")?;
        Ok(Self { conn })
    }

    /// Pop highest-priority job from `cluster_runs`.
    /// Returns `(member, score)` where member is `"{model_name}:{job_id}"`.
    pub fn pop_job(&mut self) -> Result<Option<(String, f64)>> {
        let results: Vec<(String, f64)> =
            redis::cmd("ZPOPMAX").arg("cluster_runs").query(&mut self.conn)?;
        Ok(results.into_iter().next())
    }

    /// Load job metadata from Redis hash `{model_name}:{job_id}`.
    /// Validates all required fields exist and model_name matches.
    pub fn load_job(&mut self, model_name: &str, job_id: i64) -> Result<JobMetadata> {
        let key = format!("{model_name}:{job_id}");
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
        let hash_model_name = fields
            .get("model_name")
            .cloned()
            .context("missing 'model_name' field in job metadata")?;
        if hash_model_name != model_name {
            bail!(
                "model_name mismatch: queue member '{model_name}' != hash field '{hash_model_name}'"
            );
        }
        Ok(JobMetadata { model_name: model_name.to_string(), seed, project, prompt })
    }
}
