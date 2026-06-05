//! Queue controller: automated synthesis execution on Slurm cluster.
//!
//! Reads jobs from Redis priority queue (`cluster_runs`), submits Slurm
//! jobs for model serving, launches Claude Code with MCP integration.
//! Processes sequentially until queue empty.

mod claude;
mod git_persistence;
mod queue;
mod slurm;

use anyhow::{bail, Context, Result};
use clap::Parser;
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Graceful shutdown state
// ---------------------------------------------------------------------------

struct CleanupState {
    slurm_job_id: Option<String>,
    cluster_host: String,
    project_dir: Option<PathBuf>,
    settings_backup: Option<PathBuf>,
    claude_child_pid: Option<u32>,
    orig_branch: Option<(PathBuf, String)>,  // (project_dir, branch_name) to restore on failure
}

static CLEANUP: Mutex<Option<CleanupState>> = Mutex::new(None);

fn with_cleanup<F>(f: F)
where
    F: FnOnce(&mut CleanupState),
{
    if let Ok(mut guard) = CLEANUP.lock()
        && let Some(ref mut state) = *guard
    {
        f(state);
    }
}

fn do_cleanup(state: &CleanupState) {
    if let Some(pid) = state.claude_child_pid {
        let _ = Command::new("kill").args(["--", &pid.to_string()]).status();
        eprintln!("[CLEANUP] Killed claude child {pid}");
    }

    if let Some(ref job_id) = state.slurm_job_id {
        slurm::cancel_job(&state.cluster_host, job_id);
    }

    if let Some(ref project_dir) = state.project_dir {
        let _ = claude::restore_claude_settings(project_dir, state.settings_backup.clone());
        eprintln!("[CLEANUP] Restored claude settings");
    }

    if let Some((ref project_dir, ref branch_name)) = state.orig_branch {
        if let Ok(repo) = git2::Repository::open(project_dir) {
            let _ = repo.set_head(&format!("refs/heads/{}", branch_name));
            let _ = repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()));
            eprintln!("[CLEANUP] Restored git branch '{branch_name}'");
        }
    }

    eprintln!("[CLEANUP] Graceful shutdown complete");
}

/// Drop guard: runs cleanup on any remaining state when `run()` exits.
struct CleanupGuard;

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Ok(guard) = CLEANUP.lock()
            && let Some(ref state) = *guard
        {
            let has_work = state.claude_child_pid.is_some()
                || state.slurm_job_id.is_some()
                || state.project_dir.is_some()
                || state.orig_branch.is_some();
            if has_work {
                do_cleanup(state);
            }
        }
    }
}

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

    /// Path to SSH private key for Git push authentication.
    /// If omitted, Git persistence is skipped.
    #[arg(long)]
    pub git_ssh_key: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

/// Run the queue controller. Called from binary entrypoint.
pub fn run() -> Result<()> {
    let args = Args::parse();

    // Register signal handler for graceful shutdown.
    *CLEANUP.lock().unwrap() = Some(CleanupState {
        slurm_job_id: None,
        cluster_host: args.cluster_host.clone(),
        project_dir: None,
        settings_backup: None,
        claude_child_pid: None,
        orig_branch: None,
    });

    let mut signals = Signals::new(&[SIGINT, SIGTERM])
        .context("failed to register signal handlers")?;
    std::thread::spawn(move || {
        for sig in signals.forever() {
            eprintln!("[DEBUG] Received signal {sig}, cleaning up...");
            let guard = CLEANUP.lock().unwrap();
            if let Some(ref state) = *guard {
                do_cleanup(state);
            }
            std::process::exit(128 + sig);
        }
    });

    // Drop guard cleans up any remaining state on exit (Ok or Err).
    let _guard = CleanupGuard;

    let mut qc = queue::QueueClient::open(&args.redis_url)?;
    eprintln!("[DEBUG] Connected to Redis at {}", args.redis_url);

    loop {
        // 1. Peek highest-priority job.
        let (member, score) = match qc.peek_job()? {
            Some(m) => m,
            None => {
                eprintln!("[DEBUG] Queue empty. Exiting.");
                return Ok(());
            }
        };
        eprintln!("[DEBUG] Peeked job '{member}' (priority={score})");

        // 2. Parse "{model_name}:{job_id}". Use rsplitn for colons in model name.
        let mut parts = member.rsplitn(2, ':');
        let job_id_str = parts.next().context("missing job_id in member")?;
        let model_name = parts.next().context("missing model_name in member")?;
        let job_id: i64 = job_id_str.parse().context("job_id is not a valid integer")?;

        // 3. Load + validate job metadata.
        let job = qc.load_job(model_name, job_id)?;
        eprintln!(
            "[DEBUG] Loaded job: model={} project={} seed={}",
            job.model_name, job.project, job.seed,
        );

        // 4. Construct model path.
        let model_path = args.models_path.join(&job.model_name);
        eprintln!("[DEBUG] Model path: {model_path:?}");

        // 5. Submit sbatch via SSH.
        let sbatch = slurm::generate_sbatch(&model_path, &args.llama_path, &job.seed);
        let slurm_job_id = slurm::submit_sbatch(&args.cluster_host, &sbatch)?;
        eprintln!("[DEBUG] Submitted Slurm job {slurm_job_id}");
        with_cleanup(|s| s.slurm_job_id = Some(slurm_job_id.clone()));

        // 6. Poll until RUNNING, then establish tunnel to compute node.
        slurm::poll_job(
            &args.cluster_host,
            &slurm_job_id,
            args.poll_interval,
            args.poll_timeout,
        )?;

        let node_name = slurm::get_job_node(&args.cluster_host, &slurm_job_id)?;
        let node_ip = slurm::node_name_to_ip(&node_name);
        let _tunnel = slurm::establish_tunnel(&args.cluster_host, &node_ip, args.tunnel_port)?;

        // 7. Resolve project directory.
        let project_dir = claude::resolve_project_dir(&args.project_root, &job.project);
        if !project_dir.exists() {
            bail!("project directory not found: {project_dir:?}");
        }
        eprintln!("[DEBUG] Project dir: {project_dir:?}");

        // 7b. Read project's system prompt (prompt.md) for --append-system-prompt.
        let system_prompt_path = project_dir.join("prompt.md");
        let system_prompt = if system_prompt_path.exists() {
            std::fs::read_to_string(&system_prompt_path)
                .with_context(|| format!("failed to read {system_prompt_path:?}"))?
        } else {
            eprintln!("[WARN] No prompt.md found at {system_prompt_path:?}, --append-system-prompt omitted");
            String::new()
        };
        eprintln!("[DEBUG] System prompt (prompt.md): {} bytes", system_prompt.len());

        // 8. Inject MCP settings (backup existing).
        let project_dir_str = project_dir.to_string_lossy().to_string();
        let backup = claude::setup_claude_settings(
            &project_dir,
            &args.model_url,
            model_name,
            &project_dir_str,
            &job.project,
        )?;
        with_cleanup(|s| {
            s.settings_backup = backup.clone();
            s.project_dir = Some(project_dir.clone());
        });

        // 8b. Create synthesis branch and checkout (before Claude Code).
        let mut synthesis_branch: Option<(String, String)> = None; // (orig_branch, branch_name)
        let seed: u64 = job.seed.parse().context("seed is not a valid u64")?;
        let iteration = job_id as u64;
        if let Some(ref git_ssh_key) = args.git_ssh_key {
            let auth_config = git_persistence::GitAuthConfig::new(git_ssh_key.clone());
            let git = git_persistence::GitPersistence::new(&project_dir, &auth_config)
                .context("git persistence setup failed")?;
            let (orig_branch, branch_name) = git
                .checkout_synthesis_branch(model_name, iteration, seed)
                .context("failed to prepare git branch for synthesis")?;
            with_cleanup(|s| {
                s.orig_branch = Some((project_dir.clone(), orig_branch.clone()));
            });
            synthesis_branch = Some((orig_branch, branch_name));
        }

        // 9. Kill stale mcp_synth and launch Claude Code.
        claude::kill_existing_mcp_synth();
        eprintln!("[DEBUG] Launching Claude Code...");
        let output_path = project_dir.join(format!("{}_{}.json", model_name, job_id_str));
        let mut claude_child = claude::spawn_claude(
            &project_dir, &job.prompt, &system_prompt, &output_path, model_name,
        )?;
        let child_pid = claude_child.id();
        with_cleanup(|s| s.claude_child_pid = Some(child_pid));

        let claude_status = claude_child
            .wait()
            .context("failed to wait for Claude Code")?;
        with_cleanup(|s| s.claude_child_pid = None);

        // 10. Restore settings regardless of outcome.
        claude::restore_claude_settings(&project_dir, backup)?;
        with_cleanup(|s| {
            s.settings_backup = None;
            s.project_dir = None;
        });

        // 10b. Cancel Slurm job — model server no longer needed.
        let slurm_job_id = CLEANUP.lock().ok().and_then(|mut guard| {
            guard.as_mut().and_then(|s| s.slurm_job_id.take())
        });
        if let Some(ref job_id) = slurm_job_id {
            slurm::cancel_job(&args.cluster_host, job_id);
        }

        // 11. Bail if Claude Code failed.
        if !claude_status.success() {
            bail!("Claude Code exited with status {claude_status}");
        }
        eprintln!("[DEBUG] Saved synthesis output to {output_path:?}");

        // 12. Check result; remove from queue only on succeeded_full.
        let succeeded = qc.check_succeeded_full(&job.project)?;
        if succeeded {
            qc.remove_job(&member)?;
            eprintln!("[DEBUG] Synthesis succeeded for {member}, removed from queue");

            // 13. Commit and push synthesis results to Git.
            if let Some((ref orig_branch, ref branch_name)) = synthesis_branch {
                let auth_config = git_persistence::GitAuthConfig::new(
                    args.git_ssh_key.clone().expect("git_ssh_key checked above"),
                );
                let commit_message = format!(
                    "Synthesis: {model_name} iteration {iteration} seed {seed}"
                );
                let git = git_persistence::GitPersistence::new(&project_dir, &auth_config)
                    .context("git persistence setup failed")?;
                git.commit_and_push(branch_name, orig_branch, &commit_message)
                    .context("failed to persist synthesis to git")?;
                with_cleanup(|s| s.orig_branch = None);
            }

            // 14. Loop to next job.
        } else {
            bail!("synthesis not successful for {member}, job remains in queue");
        }
    }
}
