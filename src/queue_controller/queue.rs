//! Redis queue operations for the synthesis job queue.
//!
//! Reads jobs from Redis sorted set `cluster_runs` (priority queue).
//! Each entry references a Redis hash with job metadata.

use anyhow::{Context, Result, bail};
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
        let conn = client
            .get_connection()
            .context("failed to connect to Redis")?;
        Ok(Self { conn })
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

    /// Check if latest synthesis trial for project was `succeeded_full`.
    pub fn check_succeeded_full(&mut self, project_name: &str) -> Result<bool> {
        let pid_key = format!("project:name:{project_name}");
        let project_id: Option<String> = redis::cmd("GET").arg(&pid_key).query(&mut self.conn)?;
        let project_id = match project_id {
            Some(id) => id,
            None => return Ok(false),
        };

        let trial_ids: Vec<String> = redis::cmd("SMEMBERS")
            .arg(format!("synthesis_trial:by_project:{project_id}"))
            .query(&mut self.conn)?;

        let max_id = match trial_ids
            .iter()
            .filter_map(|id| id.parse::<i64>().ok())
            .max()
        {
            Some(id) => id,
            None => return Ok(false),
        };

        let fields: HashMap<String, String> = redis::cmd("HGETALL")
            .arg(format!("synthesis_trial:{max_id}"))
            .query(&mut self.conn)?;

        match fields.get("result_type").map(|s| s.as_str()) {
            Some("succeeded_full") => Ok(true),
            _ => Ok(false),
        }
    }

    /// Load job metadata from Redis hash `{model_name}:{job_id}`.
    /// Validates all required fields exist.
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
        Ok(JobMetadata {
            model_name: model_name.to_string(),
            seed,
            project,
            prompt,
        })
    }
}
