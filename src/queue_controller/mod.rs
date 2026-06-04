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

    /// Port for SSH tunnel to reach the model server on the cluster.
    #[arg(long, default_value_t = 8080)]
    pub tunnel_port: u16,

    /// Path to llama-server executable on the cluster.
    // TODO: remove hardcoded default — make configurable through Redis job metadata.
    #[arg(long, default_value = "/home/sforza_2050030/.local/bin/llama-server")]
    pub llama_path: String,
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
        // 1. Peek highest-priority job (no removal).
        let (member, score) = match qc.peek_job()? {
            Some(m) => m,
            None => {
                eprintln!("[DEBUG] Queue empty. Exiting.");
                return Ok(());
            }
        };
        eprintln!("[DEBUG] Peeked job '{member}' (priority={score})");

        // 2. Parse member as "{model_name}:{job_id}".
        // Use rsplitn to handle model_name containing colons.
        let mut parts = member.rsplitn(2, ':');
        let job_id_str = parts.next().context("missing job_id in member")?;
        let model_name = parts.next().context("missing model_name in member")?;
        let job_id: i64 = job_id_str
            .parse()
            .context("job_id is not a valid integer")?;
        let _member_slug = member.replace(":", "-");

        // 3. Load + validate job metadata.
        let job = qc.load_job(model_name, job_id)?;
        eprintln!(
            "[DEBUG] Loaded job: model={} project={} seed={}",
            job.model_name, job.project, job.seed,
        );

        // 4. Construct model path (used in sbatch on the cluster).
        let model_path = args.models_path.join(&job.model_name);
        eprintln!("[DEBUG] Model path: {model_path:?}");

        // 5. Generate and submit sbatch via SSH.
        let sbatch = slurm::generate_sbatch(&model_path, &args.llama_path, &job.seed);
        let slurm_job_id = slurm::submit_sbatch(&args.cluster_host, &sbatch)?;
        eprintln!("[DEBUG] Submitted Slurm job {slurm_job_id}");

        // 6. Wait until model server is RUNNING.
        slurm::poll_job(
            &args.cluster_host,
            &slurm_job_id,
            args.poll_interval,
            args.poll_timeout,
        )?;

        // 6b. Resolve compute node IP and establish SSH tunnel.
        let node_name = slurm::get_job_node(&args.cluster_host, &slurm_job_id)?;
        let node_ip = slurm::node_name_to_ip(&node_name);
        let _tunnel = slurm::establish_tunnel(&args.cluster_host, &node_ip, args.tunnel_port)?;

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

        // 9. Launch Claude Code with synthesis prompt (blocking),
        //    pipe through jq, save to {model_name}_{job_id}.json.
        eprintln!("[DEBUG] Launching Claude Code...");
        let output_path = project_dir.join(format!("{}_{}.json", model_name, job_id_str));
        let claude_result = claude::launch_claude(&project_dir, &job.prompt, &output_path);

        // 10. Restore original settings regardless of outcome.
        claude::restore_claude_settings(&project_dir, backup)?;

        // 11. If Claude Code itself failed, bail immediately.
        if let Err(e) = claude_result {
            bail!("synthesis failed for job {member}: {e}");
        }
        eprintln!("[DEBUG] Saved synthesis output to {output_path:?}");

        // 12. Check synthesis result; remove from queue only on succeeded_full.
        let succeeded = qc.check_succeeded_full(&job.project)?;
        if succeeded {
            qc.remove_job(&member)?;
            eprintln!("[DEBUG] Synthesis succeeded for {member}, removed from queue");
        } else {
            bail!("synthesis not successful for {member}, job remains in queue");
        }
    }
}
