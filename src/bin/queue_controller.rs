//! Queue controller: automated synthesis execution on Slurm cluster.
//!
//! Reads jobs from Redis sorted set `cluster_runs` (priority queue),
//! submits Slurm jobs for model serving, launches Claude Code with
//! MCP integration, processes sequentially until queue empty.

use anyhow::{bail, Context, Result};
use clap::Parser;
use redis::Commands;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "queue_controller")]
struct Args {
    /// Base directory containing GGUF model files.
    #[arg(long)]
    models_path: PathBuf,

    /// Base directory containing synthesis project directories.
    #[arg(long)]
    project_root: PathBuf,

    /// Redis server URL.
    #[arg(long, default_value = "redis://localhost:6379")]
    redis_url: String,

    /// Model server URL (OpenAI-compatible API endpoint on the cluster).
    #[arg(long, default_value = "http://127.0.0.1:8080/v1")]
    model_url: String,

    /// SSH hostname for the Slurm cluster.
    #[arg(long, default_value = "cluster")]
    cluster_host: String,

    /// Polling interval in seconds for Slurm job status.
    #[arg(long, default_value_t = 30)]
    poll_interval: u64,

    /// Max wait time in seconds for Slurm job to reach RUNNING state.
    #[arg(long, default_value_t = 1800)]
    poll_timeout: u64,
}

// ---------------------------------------------------------------------------
// Redis-backed job data
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct JobMetadata {
    model_name: String,
    seed: String,
    project: String,
    prompt: String,
}

/// Thin wrapper around a Redis connection for queue operations.
struct QueueClient {
    conn: redis::Connection,
}

impl QueueClient {
    fn open(url: &str) -> Result<Self> {
        let client =
            redis::Client::open(url).with_context(|| format!("failed to open Redis at {url}"))?;
        let conn = client.get_connection().context("failed to connect to Redis")?;
        Ok(Self { conn })
    }

    /// Pop the highest-priority job from `cluster_runs`.
    /// Returns `(member, score)` where member is `"{model_name}:{job_id}"`.
    fn pop_job(&mut self) -> Result<Option<(String, f64)>> {
        let results: Vec<(String, f64)> =
            redis::cmd("ZPOPMAX").arg("cluster_runs").query(&mut self.conn)?;
        Ok(results.into_iter().next())
    }

    /// Load job metadata from hash `{model_name}:{job_id}`.
    fn load_job(&mut self, model_name: &str, job_id: i64) -> Result<JobMetadata> {
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

// ---------------------------------------------------------------------------
// Sbatch generation
// ---------------------------------------------------------------------------

fn generate_sbatch(model_path: &Path, seed: &str) -> String {
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

// ---------------------------------------------------------------------------
// Slurm interaction via SSH
// ---------------------------------------------------------------------------

/// Submit sbatch script via stdin to `ssh cluster sbatch`.
/// Returns the Slurm job ID.
fn submit_sbatch(cluster_host: &str, sbatch_content: &str) -> Result<String> {
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
fn poll_job(
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
                eprintln!("[DEBUG] Job {job_id} state: {other} (attempt {})", attempt + 1);
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
    if stdout.is_empty() { Ok(None) } else { Ok(Some(stdout)) }
}

// ---------------------------------------------------------------------------
// Claude Code launch
// ---------------------------------------------------------------------------

fn resolve_project_dir(project_root: &Path, project_name: &str) -> PathBuf {
    project_root.join(project_name)
}

/// Generate `.claude/settings.json` for project. Backs up existing file.
fn setup_claude_settings(
    project_dir: &Path,
    model_url: &str,
    mcp_cwd: &str,
    mcp_project: &str,
) -> Result<Option<PathBuf>> {
    let claude_dir = project_dir.join(".claude");
    let settings_path = claude_dir.join("settings.json");

    std::fs::create_dir_all(&claude_dir)?;

    let backup = if settings_path.exists() {
        let backup_path = settings_path.with_extension("settings.json.queue_backup");
        std::fs::copy(&settings_path, &backup_path)?;
        eprintln!("[DEBUG] Backed up settings to {backup_path:?}");
        Some(backup_path)
    } else {
        None
    };

    let settings = serde_json::json!({
        "primaryModel": "queue-synth-model",
        "provider": {
            "id": "openai",
            "config": {
                "baseUrl": model_url,
                "apiKey": "not-needed"
            }
        },
        "models": {
            "queue-synth-model": {
                "provider": "openai",
                "model": "queue-synth-model"
            }
        },
        "mcpServers": {
            "mcp_synth": {
                "command": "mcp_synth",
                "args": [
                    "--cwd", mcp_cwd,
                    "--project", mcp_project,
                    "--db-type", "redis"
                ],
                "env": {}
            }
        }
    });

    let content = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, content)?;
    eprintln!("[DEBUG] Wrote MCP settings to {settings_path:?}");
    Ok(backup)
}

/// Restore original settings or remove temp file.
fn restore_claude_settings(project_dir: &Path, backup: Option<PathBuf>) -> Result<()> {
    let settings_path = project_dir.join(".claude").join("settings.json");
    match backup {
        Some(ref backup_path) if backup_path.exists() => {
            std::fs::copy(backup_path, &settings_path)?;
            let _ = std::fs::remove_file(backup_path);
            eprintln!("[DEBUG] Restored original settings");
        }
        _ => {
            let _ = std::fs::remove_file(&settings_path);
            eprintln!("[DEBUG] Removed temporary settings");
        }
    }
    Ok(())
}

/// Launch Claude Code with prompt in project directory (blocking).
fn launch_claude(project_dir: &Path, prompt: &str) -> Result<()> {
    let status = Command::new("claude")
        .args(["--prompt", prompt, "--cd", &project_dir.to_string_lossy()])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to launch Claude Code")?;

    if !status.success() {
        bail!("Claude Code exited with status {}", status);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

fn run_controller(args: &Args) -> Result<()> {
    let mut queue = QueueClient::open(&args.redis_url)?;
    eprintln!("[DEBUG] Connected to Redis at {}", args.redis_url);

    loop {
        // 1. Pop highest-priority job.
        let (member, score) = match queue.pop_job()? {
            Some(m) => m,
            None => {
                eprintln!("[DEBUG] Queue empty. Exiting.");
                return Ok(());
            }
        };
        eprintln!("[DEBUG] Popped job '{member}' (priority={score})");

        // 2. Parse member as "{model_name}:{job_id}".
        // Use rsplitn to handle model_name containing colons.
        let mut parts = member.rsplitn(2, ':');
        let job_id_str = parts.next().context("missing job_id in member")?;
        let model_name = parts.next().context("missing model_name in member")?;
        let job_id: i64 = job_id_str
            .parse()
            .context("job_id is not a valid integer")?;

        // 3. Load + validate job metadata.
        let job = queue.load_job(model_name, job_id)?;
        eprintln!(
            "[DEBUG] Loaded job: model={} project={} seed={}",
            job.model_name, job.project, job.seed,
        );

        // 4. Construct model path; validate it exists.
        let model_path = args.models_path.join(&job.model_name);
        if !model_path.exists() {
            bail!("model file not found: {model_path:?}");
        }
        eprintln!("[DEBUG] Model path: {model_path:?}");

        // 5. Generate and submit sbatch via SSH.
        let sbatch = generate_sbatch(&model_path, &job.seed);
        let slurm_job_id = submit_sbatch(&args.cluster_host, &sbatch)?;
        eprintln!("[DEBUG] Submitted Slurm job {slurm_job_id}");

        // 6. Wait until model server is RUNNING.
        poll_job(
            &args.cluster_host,
            &slurm_job_id,
            args.poll_interval,
            args.poll_timeout,
        )?;

        // 7. Resolve project directory; validate it exists.
        let project_dir = resolve_project_dir(&args.project_root, &job.project);
        if !project_dir.exists() {
            bail!("project directory not found: {project_dir:?}");
        }
        eprintln!("[DEBUG] Project dir: {project_dir:?}");

        // 8. Set up Claude Code MCP settings (backup existing).
        let project_dir_str = project_dir.to_string_lossy().to_string();
        let backup =
            setup_claude_settings(&project_dir, &args.model_url, &project_dir_str, &job.project)?;

        // 9. Launch Claude Code with synthesis prompt (blocking).
        eprintln!("[DEBUG] Launching Claude Code...");
        let result = launch_claude(&project_dir, &job.prompt);

        // 10. Restore original settings.
        restore_claude_settings(&project_dir, backup)?;

        // 11. Handle result; fail-fast on error.
        match result {
            Ok(()) => eprintln!("[DEBUG] Synthesis completed for job {member}"),
            Err(e) => bail!("synthesis failed for job {member}: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------------

fn main() {
    let args = Args::parse();

    match run_controller(&args) {
        Ok(()) => eprintln!("[DEBUG] Queue controller finished successfully"),
        Err(e) => {
            eprintln!("[ERROR] {e:#}");
            std::process::exit(1);
        }
    }
}
