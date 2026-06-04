//! Queue controller: automated synthesis execution on Slurm cluster.
//!
//! Reads jobs from Redis priority queue (`cluster_runs`), submits Slurm
//! jobs for model serving, launches Claude Code with MCP integration.
//! Processes sequentially until queue empty.

mod claude;
mod queue;
mod slurm;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "queue_controller")]
pub struct Args {
    /// Base directory containing GGUF model files.
    #[arg(long)]
    pub models_path: PathBuf,

    /// Base directory containing synthesis project directories.
    #[arg(long)]
    pub project_root: PathBuf,

    /// Redis server URL.
    #[arg(long, default_value = "redis://localhost:6379")]
    pub redis_url: String,

    /// Model server URL (OpenAI-compatible API endpoint on the cluster).
    #[arg(long, default_value = "http://127.0.0.1:8080/v1")]
    pub model_url: String,

    /// SSH hostname for the Slurm cluster.
    #[arg(long, default_value = "cluster")]
    pub cluster_host: String,

    /// Polling interval in seconds for Slurm job status.
    #[arg(long, default_value_t = 30)]
    pub poll_interval: u64,

    /// Max wait time in seconds for Slurm job to reach RUNNING state.
    #[arg(long, default_value_t = 1800)]
    pub poll_timeout: u64,
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

/// Run the queue controller. Called from binary entrypoint.
pub fn run() -> Result<()> {
    let args = Args::parse();
    let mut qc = queue::QueueClient::open(&args.redis_url)?;
    eprintln!("[DEBUG] Connected to Redis at {}", args.redis_url);

    loop {
        // 1. Pop highest-priority job.
        let (member, score) = match qc.pop_job()? {
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
        let job = qc.load_job(model_name, job_id)?;
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
        let sbatch = slurm::generate_sbatch(&model_path, &job.seed);
        let slurm_job_id = slurm::submit_sbatch(&args.cluster_host, &sbatch)?;
        eprintln!("[DEBUG] Submitted Slurm job {slurm_job_id}");

        // 6. Wait until model server is RUNNING.
        slurm::poll_job(
            &args.cluster_host,
            &slurm_job_id,
            args.poll_interval,
            args.poll_timeout,
        )?;

        // 7. Resolve project directory; validate it exists.
        let project_dir = claude::resolve_project_dir(&args.project_root, &job.project);
        if !project_dir.exists() {
            bail!("project directory not found: {project_dir:?}");
        }
        eprintln!("[DEBUG] Project dir: {project_dir:?}");

        // 8. Set up Claude Code MCP settings (backup existing).
        let project_dir_str = project_dir.to_string_lossy().to_string();
        let backup = claude::setup_claude_settings(
            &project_dir,
            &args.model_url,
            &project_dir_str,
            &job.project,
        )?;

        // 9. Launch Claude Code with synthesis prompt (blocking).
        eprintln!("[DEBUG] Launching Claude Code...");
        let result = claude::launch_claude(&project_dir, &job.prompt);

        // 10. Restore original settings.
        claude::restore_claude_settings(&project_dir, backup)?;

        // 11. Handle result; fail-fast on error.
        match result {
            Ok(()) => eprintln!("[DEBUG] Synthesis completed for job {member}"),
            Err(e) => bail!("synthesis failed for job {member}: {e}"),
        }
    }
}
