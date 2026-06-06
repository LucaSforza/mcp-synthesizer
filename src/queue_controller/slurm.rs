//! Sbatch generation and Slurm interaction via SSH.

use anyhow::{Context, Result, bail};
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

/// Slurm job state from `squeue --format %T`.
#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Timeout,
    Other(String),
    NotFound,
}

impl FromStr for JobState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "PENDING" | "PENDING+" | "SUSPENDED" | "CONFIGURING" => Ok(JobState::Pending),
            "RUNNING" => Ok(JobState::Running),
            "COMPLETED" => Ok(JobState::Completed),
            "FAILED" => Ok(JobState::Failed),
            "CANCELLED" => Ok(JobState::Cancelled),
            "TIMEOUT" => Ok(JobState::Timeout),
            other => Ok(JobState::Other(other.to_string())),
        }
    }
}

impl JobState {
    /// True for terminal states that require model-server recovery.
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobState::Completed
                | JobState::Failed
                | JobState::Cancelled
                | JobState::Timeout
                | JobState::NotFound,
        )
    }
}

impl fmt::Display for JobState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobState::Pending => write!(f, "PENDING"),
            JobState::Running => write!(f, "RUNNING"),
            JobState::Completed => write!(f, "COMPLETED"),
            JobState::Failed => write!(f, "FAILED"),
            JobState::Cancelled => write!(f, "CANCELLED"),
            JobState::Timeout => write!(f, "TIMEOUT"),
            JobState::Other(s) => write!(f, "{s}"),
            JobState::NotFound => write!(f, "NOT_FOUND"),
        }
    }
}

/// Handle for SSH port forwarding tunnel. Kills tunnel on drop.
pub struct TunnelHandle {
    child: Child,
}

impl Drop for TunnelHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        eprintln!("[DEBUG] SSH tunnel closed");
    }
}

/// Generate sbatch script content.
/// Only MODEL_PATH, LLAMA_PATH and SEED are parameterized; everything else hardcoded.
pub fn generate_sbatch(model_path: &Path, llama_path: &str, seed: &str) -> String {
    let model_path_str = model_path.to_string_lossy();
    let model_slug = model_path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_else(|| model_path_str.clone());

    format!(
        r#"#!/bin/bash
#SBATCH --job-name=synth-{model_slug}
#SBATCH --output=synth-%j.out
#SBATCH --error=synth-%j.err
#SBATCH --gpus=1
#SBATCH --mem=41G

{llama_path} \
    --model {model_path} \
    --seed {seed} \
    --models-max 1 \
    -t 8 \
    -ngl 99 \
    -c 256000 \
    --host 0.0.0.0 \
    --cache-reuse 256 \
    --temp 0.6 \
    --top-p 0.95 \
    --top-k 20 \
    --min-p 0.0 \
    --presence-penalty 0.0 \
    --repeat-penalty 1.0

# TODO: Make all llama.cpp parameters configurable through Redis.
"#,
        model_slug = model_slug,
        model_path = model_path_str,
        llama_path = llama_path,
        seed = seed,
    )
}

/// Submit sbatch script via stdin to `ssh cluster sbatch`.
/// Returns Slurm job ID.
pub fn submit_sbatch(cluster_host: &str, sbatch_content: &str) -> Result<String> {
    let mut child = Command::new("ssh")
        .args([cluster_host, "sbatch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn ssh for sbatch submission")?;

    if let Some(ref mut stdin) = child.stdin {
        stdin.write_all(sbatch_content.as_bytes())?;
    }
    // Drop stdin to signal EOF to sbatch.
    drop(child.stdin.take());

    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "sbatch submission failed (exit={}): stdout={stdout} stderr={stderr}",
            output.status
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // sbatch prints: "Submitted batch job 123456"
    let job_id = stdout
        .split_whitespace()
        .last()
        .context("unable to parse job ID from sbatch output")?
        .to_string();
    Ok(job_id)
}

/// Check state of the job in the cluster.
/// Returns `Ok(true)` if RUNNING (caller should stop polling),
/// `Ok(false)` if still pending, `Err` on terminal states.
fn check_job(cluster_host: &str, job_id: &str, attempt: u64) -> Result<bool> {
    let state = get_job_state(cluster_host, job_id)?;
    match state {
        JobState::Running => {
            eprintln!("[DEBUG] Job {job_id} is RUNNING");
            return Ok(true);
        }
        JobState::Completed => {
            bail!("job {job_id} already COMPLETED (model server exited early)");
        }
        JobState::Failed => {
            bail!("job {job_id} entered FAILED state");
        }
        JobState::Cancelled => {
            bail!("job {job_id} was CANCELLED");
        }
        JobState::Timeout => {
            bail!("job {job_id} timed out");
        }
        JobState::NotFound => {
            bail!("job {job_id} not found in squeue");
        }
        JobState::Pending | JobState::Other(_) => {
            eprintln!(
                "[DEBUG] Job {job_id} state: {state} (attempt {})",
                attempt + 1
            );
        }
    }
    Ok(false)
}

/// Poll Slurm via `ssh cluster squeue` until job reaches RUNNING or terminal state.
pub fn poll_job(
    cluster_host: &str,
    job_id: &str,
    interval_secs: u64,
    timeout_secs: u64,
) -> Result<()> {
    let max_polls = if interval_secs == 0 {
        u64::MAX
    } else {
        timeout_secs.div_ceil(interval_secs)
    };

    for attempt in 0..max_polls {
        if check_job(cluster_host, job_id, attempt)? {
            return Ok(());
        }
        sleep(Duration::from_secs(interval_secs));
    }

    bail!("job {job_id} did not reach RUNNING state within {timeout_secs}s");
}

/// Get single job state via squeue.
pub(crate) fn get_job_state(cluster_host: &str, job_id: &str) -> Result<JobState> {
    let output = Command::new("ssh")
        .args([
            cluster_host,
            "squeue",
            "--job",
            job_id,
            "--noheader",
            "--format",
            "%T",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run squeue for job {job_id}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Ok(JobState::NotFound);
    }
    stdout
        .parse::<JobState>()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Get compute node hostname for a Slurm job via squeue, fallback to sacct.
pub fn get_job_node(cluster_host: &str, job_id: &str) -> Result<String> {
    // Try squeue first (active jobs).
    let squeue = Command::new("ssh")
        .args([
            cluster_host,
            "squeue",
            "--job",
            job_id,
            "--noheader",
            "--format",
            "%N",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("failed to run squeue for node lookup")?;
    let node = String::from_utf8_lossy(&squeue.stdout).trim().to_string();
    if !node.is_empty() && node != "N/A" {
        return Ok(node);
    }

    // Fallback to sacct (historical data for finished jobs).
    let sacct = Command::new("ssh")
        .args([
            cluster_host,
            "sacct",
            "--job",
            job_id,
            "--noheader",
            "--format",
            "Node",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("failed to run sacct for node lookup")?;
    let node = String::from_utf8_lossy(&sacct.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if node.is_empty() {
        bail!("could not determine compute node for job {job_id}");
    }
    Ok(node)
}

/// Convert node hostname to IP using convention: `node123` → `10.0.0.23`.
pub fn node_name_to_ip(node_name: &str) -> String {
    let digits: String = node_name.chars().filter(|c| c.is_ascii_digit()).collect();
    let suffix = if digits.len() > 2 {
        &digits[digits.len() - 2..]
    } else {
        &digits
    };
    format!("10.0.0.{}", suffix)
}

/// Establish SSH port forwarding: `ssh -L port:node_ip:port cluster_host -N`.
pub fn establish_tunnel(cluster_host: &str, node_ip: &str, port: u16) -> Result<TunnelHandle> {
    let child = Command::new("ssh")
        .args([
            "-L",
            &format!("{port}:{node_ip}:{port}"),
            cluster_host,
            "-N",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn SSH tunnel")?;
    eprintln!("[DEBUG] SSH tunnel established: localhost:{port} -> {node_ip}:{port}");
    Ok(TunnelHandle { child })
}

/// Cancel a Slurm job via `ssh cluster scancel`.
pub fn cancel_job(cluster_host: &str, job_id: &str) {
    let _ = Command::new("ssh")
        .args([cluster_host, "scancel", job_id])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    eprintln!("[DEBUG] Cancelled Slurm job {job_id}");
}
