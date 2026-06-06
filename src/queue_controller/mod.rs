//! Queue controller: automated synthesis execution on Slurm cluster.
//!
//! Reads jobs from Redis priority queue (`cluster_runs`), submits Slurm
//! jobs for model serving, launches Claude Code with MCP integration.
//! Processes sequentially until queue empty.

mod claude;
mod git_persistence;
mod queue;
mod slurm;
mod synthesis_usage;

mod cleanup;
mod log_ctx;
mod synthesis_monitor;

use anyhow::{Context, Result, bail};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use std::path::{Path, PathBuf};

use self::cleanup::{
    CLEANUP, CleanupGuard, CleanupState, cleanup_and_reset, do_cleanup, take_slurm_job_id,
    with_cleanup,
};
use self::log_ctx::JOB_PREFIX;
use crate::Args;

/// Like `eprintln!` but prepends `[job:id]` from [`JOB_PREFIX`] if set.
macro_rules! debug_log {
    ($($arg:tt)*) => {{
        let _guard = JOB_PREFIX.lock().unwrap();
        if !_guard.is_empty() {
            eprint!("{} ", _guard);
        }
        eprintln!($($arg)*);
    }};
}
pub(crate) use debug_log;

// ---------------------------------------------------------------------------
// Loop step functions (in execution order)
// ---------------------------------------------------------------------------

/// Step 1-3: Peek highest-priority job from Redis queue,
/// parse `{model_name}:{job_id}` member, and load job metadata.
///
/// Returns `Ok(None)` when the queue is empty (caller should exit).
fn peek_and_load_job(
    qc: &mut queue::QueueClient,
) -> Result<Option<(String, String, String, i64, queue::JobMetadata)>> {
    eprintln!(
        "[DEBUG] [Step 1-3] peek_and_load_job — peek Redis queue, parse model:job_id, load metadata"
    );
    let (member, score) = match qc.peek_job()? {
        Some(m) => m,
        None => return Ok(None),
    };
    eprintln!("[DEBUG] Peeked job '{member}' (priority={score})");

    // Parse "{model_name}:{job_id}". Use rsplitn for colons in model name.
    let mut parts = member.rsplitn(2, ':');
    let job_id_str = parts
        .next()
        .context("missing job_id in member")?
        .to_string();
    let model_name = parts
        .next()
        .context("missing model_name in member")?
        .to_string();
    let job_id: i64 = job_id_str
        .parse()
        .context("job_id is not a valid integer")?;

    let job = qc.load_job(&model_name, job_id)?;
    eprintln!(
        "[DEBUG] Loaded job: model={} project={} seed={}",
        job.model_name, job.project, job.seed,
    );

    Ok(Some((member, model_name, job_id_str, job_id, job)))
}

/// Step 4-5: Construct model path, generate sbatch script, and submit
/// via SSH.  Returns the Slurm job ID.
fn submit_slurm_job(
    cluster_host: &str,
    models_path: &Path,
    model_name: &str,
    llama_path: &str,
    seed: &str,
) -> Result<String> {
    debug_log!("[DEBUG] [Step 4-5] submit_slurm_job — generate sbatch, submit via SSH");
    let model_path = models_path.join(model_name);
    debug_log!("[DEBUG] Model path: {model_path:?}");

    let sbatch = slurm::generate_sbatch(&model_path, llama_path, seed);
    let slurm_job_id = slurm::submit_sbatch(cluster_host, &sbatch)?;
    debug_log!("[DEBUG] Submitted Slurm job {slurm_job_id}");
    Ok(slurm_job_id)
}

/// Step 6: Poll Slurm job until RUNNING, retrieve compute node hostname
/// from squeue, convert to IP, and establish an SSH port-forwarding tunnel.
///
/// Returns a `TunnelHandle` whose `Drop` implementation closes the tunnel.
/// IMPORTANT: So, it is dropped every loop.
fn wait_and_create_tunnel(
    cluster_host: &str,
    slurm_job_id: &str,
    poll_interval: u64,
    poll_timeout: u64,
    tunnel_port: u16,
) -> Result<slurm::TunnelHandle> {
    debug_log!(
        "[DEBUG] [Step 6] wait_and_create_tunnel — poll job status, resolve node IP, establish SSH tunnel"
    );
    slurm::poll_job(cluster_host, slurm_job_id, poll_interval, poll_timeout)?;
    let node_name = slurm::get_job_node(cluster_host, slurm_job_id)?;
    let node_ip = slurm::node_name_to_ip(&node_name);
    let tunnel = slurm::establish_tunnel(cluster_host, &node_ip, tunnel_port)?;
    Ok(tunnel)
}

/// Step 7 + 7b: Resolve the project directory under `project_root` and
/// read the project-specific system prompt from `prompt.md`.
fn prepare_project_environment(
    project_root: &Path,
    project_name: &str,
) -> Result<(PathBuf, String)> {
    debug_log!(
        "[DEBUG] [Step 7+7b] prepare_project_environment — resolve project dir, read prompt.md"
    );
    let project_dir = claude::resolve_project_dir(project_root, project_name);
    if !project_dir.exists() {
        bail!("project directory not found: {project_dir:?}");
    }
    debug_log!("[DEBUG] Project dir: {project_dir:?}");

    let system_prompt_path = project_dir.join("prompt.md");
    let system_prompt = if system_prompt_path.exists() {
        std::fs::read_to_string(&system_prompt_path)
            .with_context(|| format!("failed to read {system_prompt_path:?}"))?
    } else {
        debug_log!(
            "[WARN] No prompt.md found at {system_prompt_path:?}, \
             --append-system-prompt omitted"
        );
        String::new()
    };
    debug_log!(
        "[DEBUG] System prompt (prompt.md): {} bytes",
        system_prompt.len()
    );

    Ok((project_dir, system_prompt))
}

/// Step 8 + 8b: Inject MCP server settings into `.claude/settings.local.json`
/// and, if SSH key is configured, create a synthesis git branch and checkout.
///
/// Returns the settings backup path and optional synthesis branch info
/// `(orig_branch, branch_name)`.
fn setup_claude_and_git(
    project_dir: &Path,
    model_url: &str,
    model_name: &str,
    project_name: &str,
    git_ssh_key: Option<&PathBuf>,
    seed: u64,
    iteration: u64,
) -> Result<(Option<PathBuf>, Option<(String, String)>)> {
    debug_log!(
        "[DEBUG] [Step 8+8b] setup_claude_and_git — inject MCP settings, create synthesis git branch"
    );
    // Step 8: inject MCP settings (backup existing).
    let project_dir_str = project_dir.to_string_lossy().to_string();
    let backup = claude::setup_claude_settings(
        project_dir,
        model_url,
        model_name,
        &project_dir_str,
        project_name,
    )?;
    with_cleanup(|s| {
        s.settings_backup = backup.clone();
        s.project_dir = Some(project_dir.to_path_buf());
    });

    // Step 8b: create synthesis branch and checkout NOTE: this must be executed before spawning Claude Code.
    let synthesis_branch = if let Some(key_path) = git_ssh_key {
        let auth_config = git_persistence::GitAuthConfig::new(key_path.clone());
        let git = git_persistence::GitPersistence::new(project_dir, &auth_config)
            .context("git persistence setup failed")?;
        let (orig_branch, branch_name) = git
            .checkout_synthesis_branch(model_name, iteration, seed)
            .context("failed to prepare git branch for synthesis")?;
        with_cleanup(|s| {
            s.orig_branch = Some((project_dir.to_path_buf(), orig_branch.clone()));
        });
        Some((orig_branch, branch_name))
    } else {
        None
    };

    Ok((backup, synthesis_branch))
}

/// Step 9: Kill stale `mcp_synth` process, launch Claude Code, and return
/// the child process handle together with the output file path.
fn run_claude_code(
    project_dir: &Path,
    prompt: &str,
    system_prompt: &str,
    model_name: &str,
    job_id_str: &str,
) -> Result<(std::process::Child, PathBuf)> {
    debug_log!("[DEBUG] [Step 9] run_claude_code — kill stale mcp_synth, spawn Claude Code");
    claude::kill_existing_mcp_synth();
    debug_log!("[DEBUG] Launching Claude Code...");
    let output_path = project_dir.join(format!("{}_{}.jsonl", model_name, job_id_str));
    let child = claude::spawn_claude(project_dir, prompt, system_prompt, &output_path, model_name)?;
    Ok((child, output_path))
}

/// Step 10 + 10b: Restore original `.claude/settings.local.json` from
/// backup and cancel the Slurm model-serving job.
fn cleanup_environment(
    project_dir: &Path,
    backup: Option<PathBuf>,
    cluster_host: &str,
) -> Result<()> {
    debug_log!(
        "[DEBUG] [Step 10+10b] cleanup_environment — restore claude settings, cancel Slurm job"
    );
    // Step 10: restore settings.
    claude::restore_claude_settings(project_dir, backup)?;
    with_cleanup(|s| {
        s.settings_backup = None;
        s.project_dir = None;
    });

    // Step 10b: cancel Slurm job — model server no longer needed.
    if let Some(ref job_id) = take_slurm_job_id() {
        slurm::cancel_job(cluster_host, job_id);
    } else {
        debug_log!("[WARN] no slurm job id to cancel");
    }

    Ok(())
}

/// Step 11: Check Claude Code exit status and verify synthesis produced
/// a `succeeded_full` trial in the database.
///
/// Returns `Ok(())` only if both checks pass.
fn check_claude_result(
    claude_status: std::process::ExitStatus,
    output_path: &Path,
    qc: &mut queue::QueueClient,
    project_name: &str,
    member: &str,
) -> Result<()> {
    debug_log!("[DEBUG] [Step 11] check_claude_result — verify exit status + succeeded_full");
    if !claude_status.success() {
        bail!("Claude Code exited with status {claude_status}");
    }
    debug_log!("[DEBUG] Saved synthesis output to {output_path:?}");

    if !qc.check_succeeded_full(project_name)? {
        bail!("synthesis not successful for {member}, job remains in queue");
    }

    Ok(())
}

/// Step 12: Remove successfully completed job from Redis queue.
fn remove_job_from_queue(qc: &mut queue::QueueClient, member: &str) -> Result<()> {
    debug_log!("[DEBUG] [Step 12] remove_job_from_queue — synthesis succeeded");
    qc.remove_job(member)?;
    debug_log!("[DEBUG] Synthesis succeeded for {member}, removed from queue");
    Ok(())
}

/// Step 13: Parse usage metrics from the Claude Code stream-json output
/// file and persist them to the project's `test_run` hash in Redis.
///
/// All errors are non-fatal (logged as WARN).
fn persist_usage_to_redis(redis_url: &str, output_path: &Path, project_name: &str) {
    debug_log!(
        "[DEBUG] [Step 13] persist_usage_to_redis — parse Claude Code output, write usage to Redis"
    );
    match synthesis_usage::parse_output_file(output_path) {
        Ok(usage) => {
            debug_log!(
                "[DEBUG] Usage parsed: {} in / {} out / ${:.4}",
                usage.input_tokens,
                usage.output_tokens,
                usage.cost_usd,
            );
            match redis::Client::open(redis_url).and_then(|c| c.get_connection()) {
                Ok(mut usage_conn) => {
                    if let Err(e) = synthesis_usage::write_usage_to_test_run(
                        &mut usage_conn,
                        project_name,
                        &usage,
                    ) {
                        debug_log!("[WARN] Failed to write usage to test_run: {e:#}");
                    }
                }
                Err(e) => debug_log!("[WARN] Failed to connect to Redis for usage write: {e:#}"),
            }
        }
        Err(e) => debug_log!(
            "[WARN] Failed to parse usage from {}: {e:#}",
            output_path.display(),
        ),
    }
}

/// Step 14: Stage all changes, create a git commit, push to origin, and
/// restore the original branch.
fn push_synthesis_to_git(
    project_dir: &Path,
    git_ssh_key: &Path,
    model_name: &str,
    iteration: u64,
    seed: u64,
    orig_branch: &str,
    branch_name: &str,
) -> Result<()> {
    debug_log!("[DEBUG] [Step 14] push_synthesis_to_git — commit, push, restore branch");
    let auth_config = git_persistence::GitAuthConfig::new(git_ssh_key.to_path_buf());
    let commit_message = format!("Synthesis: {model_name} iteration {iteration} seed {seed}");
    let git = git_persistence::GitPersistence::new(project_dir, &auth_config)
        .context("git persistence setup failed")?;
    git.commit_and_push(branch_name, orig_branch, &commit_message)
        .context("failed to persist synthesis to git")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

/// Run the queue controller.  Called from binary entrypoint.
pub fn run(args: Args) -> Result<()> {
    // Register signal handler for graceful shutdown.
    *CLEANUP.lock().unwrap() = Some(CleanupState {
        slurm_job_id: None,
        cluster_host: args.cluster_host.clone(),
        project_dir: None,
        settings_backup: None,
        claude_child_pid: None,
        orig_branch: None,
    });

    let mut signals =
        Signals::new([SIGINT, SIGTERM]).context("failed to register signal handlers")?;
    std::thread::spawn(move || {
        for sig in signals.forever() {
            eprintln!("[DEBUG] Received signal {sig}, cleaning up...");
            let guard = CLEANUP.lock().unwrap();
            if let Some(ref state) = *guard {
                do_cleanup(state);
            }
            eprintln!("[CLEANUP] Graceful shutdown complete");
            std::process::exit(128 + sig);
        }
    });

    // Drop guard cleans up any remaining state on exit (Ok or Err).
    let _guard = CleanupGuard;

    let mut qc = queue::QueueClient::open(&args.redis_url)?;
    eprintln!("[DEBUG] Connected to Redis at {}", args.redis_url);

    // TODO: controlla che tutto funzioni prima di iniziare il loop. Non ha senso fallire durante
    // le sintesi.

    loop {
        // ------------------------------------------------------------------
        // Phase 1 — Acquire job from Redis queue (Step 1-3)
        // ------------------------------------------------------------------
        let Some((member, model_name, job_id_str, job_id, job)) = peek_and_load_job(&mut qc)?
        else {
            eprintln!("[DEBUG] Queue empty. Exiting.");
            return Ok(());
        };
        let seed: u64 = job.seed.parse().context("seed is not a valid u64")?;
        let iteration = job_id as u64;

        // Print job separator banner BEFORE setting per-job prefix, so it
        // stands out as the first visual element for this job.
        eprintln!("\n{}", "=".repeat(80));
        eprintln!(
            "===== Job {}:{} (seed {}, iteration {}) =====",
            model_name, job_id_str, seed, iteration,
        );
        eprintln!("{}", "=".repeat(80));

        // Set per-job debug prefix for all subsequent debug_log calls.
        log_ctx::set(&job_id_str);

        // ------------------------------------------------------------------
        // Phase 2 — Submit Slurm job and establish SSH tunnel (Step 4-6)
        // ------------------------------------------------------------------

        let slurm_job_id = submit_slurm_job(
            &args.cluster_host,
            &args.models_path,
            &model_name,
            &args.llama_path,
            &job.seed,
        )?;
        with_cleanup(|s| s.slurm_job_id = Some(slurm_job_id.clone()));

        let tunnel = wait_and_create_tunnel(
            &args.cluster_host,
            &slurm_job_id,
            args.poll_interval,
            args.poll_timeout,
            args.tunnel_port,
        )?;

        // ------------------------------------------------------------------
        // Phase 3 — Prepare local environment (Step 7+7b, Step 8+8b)
        // ------------------------------------------------------------------
        let (project_dir, system_prompt) =
            prepare_project_environment(&args.project_root, &job.project)?;

        let (backup, synthesis_branch) = setup_claude_and_git(
            &project_dir,
            &args.model_url,
            &model_name,
            &job.project,
            args.git_ssh_key.as_ref(),
            seed,
            iteration,
        )?;

        // ------------------------------------------------------------------
        // Phase 4 — Run Claude Code (Step 9)
        // ------------------------------------------------------------------
        let (mut claude_child, output_path) = run_claude_code(
            &project_dir,
            &job.prompt,
            &system_prompt,
            &model_name,
            &job_id_str,
        )?;
        let child_pid = claude_child.id();
        with_cleanup(|s| s.claude_child_pid = Some(child_pid));

        let mut monitor = synthesis_monitor::SynthesisMonitor::new(
            slurm_job_id.clone(),
            tunnel,
            &args.models_path,
            &model_name,
            &args.llama_path,
            &job.seed,
            &args.cluster_host,
            args.tunnel_port,
            args.poll_interval,
            args.poll_timeout,
        );
        let claude_status = monitor.wait_for_completion(&mut claude_child)?;
        with_cleanup(|s| s.claude_child_pid = None);

        // ------------------------------------------------------------------
        // Phase 5 — Tear down environment (Step 10+10b)
        // ------------------------------------------------------------------
        cleanup_environment(&project_dir, backup, &args.cluster_host)?;

        // ------------------------------------------------------------------
        // Phase 6 — Evaluate result (Step 11-12)
        // ------------------------------------------------------------------
        check_claude_result(claude_status, &output_path, &mut qc, &job.project, &member)?;
        remove_job_from_queue(&mut qc, &member)?;

        // ------------------------------------------------------------------
        // Phase 7 — Persist results (Step 13-14)
        // ------------------------------------------------------------------
        persist_usage_to_redis(&args.redis_url, &output_path, &job.project);

        if let Some((ref orig_branch, ref branch_name)) = synthesis_branch {
            push_synthesis_to_git(
                &project_dir,
                args.git_ssh_key
                    .as_ref()
                    .expect("git_ssh_key is Some when synthesis_branch is Some"),
                &model_name,
                iteration,
                seed,
                orig_branch,
                branch_name,
            )?;
            with_cleanup(|s| s.orig_branch = None);
        }

        eprintln!("===== End job {}:{} =====\n", model_name, job_id_str);

        // Reset per-iteration state and job prefix for next job.
        if let Ok(mut guard) = CLEANUP.lock()
            && let Some(ref mut state) = *guard
        {
            cleanup_and_reset(state);
        }

        // Loop back for next job.
    }
}
