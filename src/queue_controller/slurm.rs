//! Sbatch generation and Slurm interaction via SSH.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

/// Generate sbatch script content.
/// Only MODEL_PATH and SEED are parameterized; everything else hardcoded.
pub fn generate_sbatch(model_path: &Path, seed: &str) -> String {
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
#SBATCH --time=04:00:00
#SBATCH --mem=32G

llama-server \
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
        .trim()
        .split_whitespace()
        .last()
        .context("unable to parse job ID from sbatch output")?
        .to_string();
    Ok(job_id)
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
        (timeout_secs + interval_secs - 1) / interval_secs
    };

    for attempt in 0..max_polls {
        let state = get_job_state(cluster_host, job_id)?;
        match state.as_deref() {
            Some("RUNNING") => {
                eprintln!("[DEBUG] Job {job_id} is RUNNING");
                return Ok(());
            }
            Some("COMPLETED") => {
                bail!("job {job_id} already COMPLETED (model server exited early)");
            }
            Some("FAILED") => {
                bail!("job {job_id} entered FAILED state");
            }
            Some("CANCELLED") => {
                bail!("job {job_id} was CANCELLED");
            }
            Some("TIMEOUT") => {
                bail!("job {job_id} timed out");
            }
            Some(other) => {
                eprintln!(
                    "[DEBUG] Job {job_id} state: {other} (attempt {})",
                    attempt + 1
                );
            }
            None => {
                bail!("job {job_id} not found in squeue");
            }
        }
        sleep(Duration::from_secs(interval_secs));
    }

    bail!("job {job_id} did not reach RUNNING state within {timeout_secs}s");
}

/// Get single job state via squeue.
fn get_job_state(cluster_host: &str, job_id: &str) -> Result<Option<String>> {
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
        Ok(None)
    } else {
        Ok(Some(stdout))
    }
}
