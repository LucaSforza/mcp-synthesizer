//! Health checks for the queue controller.
//!
//! Three categories:
//! - **Startup**: run once before the main loop, validate all infrastructure.
//! - **Loop**: run before each job, detect degradation during runtime.
//! - **Job preflight**: run after loading job metadata, before cluster allocation.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::Args;
use super::queue;

// ---------------------------------------------------------------------------
// Private helpers — individual checks
// ---------------------------------------------------------------------------

/// Run `ssh {host} true` to verify SSH reachability and authentication.
fn check_cluster_ssh(cluster_host: &str) -> Result<()> {
    eprintln!("[HEALTH] Checking cluster SSH connectivity...");
    let output = Command::new("ssh")
        .args([cluster_host, "true"])
        .output()
        .with_context(|| format!("failed to spawn ssh to {cluster_host}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("SSH connection to {cluster_host} failed: {stderr}");
    }
    eprintln!("[HEALTH] Cluster SSH connectivity OK");
    Ok(())
}

/// Verify `claude` binary exists on PATH.
fn check_claude_binary() -> Result<()> {
    eprintln!("[HEALTH] Checking claude binary...");
    let output = Command::new("which")
        .arg("claude")
        .output()
        .context("failed to check claude binary")?;
    if !output.status.success() {
        bail!("claude binary not found on PATH");
    }
    eprintln!("[HEALTH] Claude binary OK");
    Ok(())
}

/// Verify Slurm scheduler is reachable on the cluster.
fn check_slurm_available(cluster_host: &str) -> Result<()> {
    eprintln!("[HEALTH] Checking Slurm availability...");
    let output = Command::new("ssh")
        .args([cluster_host, "sinfo", "--version"])
        .output()
        .with_context(|| format!("failed to check Slurm on {cluster_host}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Slurm not available on {cluster_host}: {stderr}");
    }
    eprintln!("[HEALTH] Slurm availability OK");
    Ok(())
}

/// Verify models directory exists on the cluster.
fn check_models_directory(cluster_host: &str, path: &Path) -> Result<()> {
    eprintln!("[HEALTH] Checking models directory...");
    let output = Command::new("ssh")
        .args([cluster_host, "test", "-d", &path.to_string_lossy()])
        .output()
        .with_context(|| format!("failed to check models directory on {cluster_host}"))?;
    if !output.status.success() {
        bail!("models directory not found on cluster: {path:?}");
    }
    eprintln!("[HEALTH] Models directory OK");
    Ok(())
}

/// Verify llama-server executable exists on the cluster.
fn check_cluster_llama_path(cluster_host: &str, path: &str) -> Result<()> {
    eprintln!("[HEALTH] Checking llama-server path...");
    let output = Command::new("ssh")
        .args([cluster_host, "test", "-x", path])
        .output()
        .with_context(|| format!("failed to check llama path on {cluster_host}"))?;
    if !output.status.success() {
        bail!("llama-server not found or not executable on cluster: {path}");
    }
    eprintln!("[HEALTH] Llama-server path OK");
    Ok(())
}

/// Verify local project root directory exists.
fn check_project_root(path: &Path) -> Result<()> {
    eprintln!("[HEALTH] Checking project root...");
    if !path.exists() {
        bail!("project root not found: {path:?}");
    }
    if !path.is_dir() {
        bail!("project root is not a directory: {path:?}");
    }
    eprintln!("[HEALTH] Project root OK");
    Ok(())
}

/// Verify SSH key file exists and has safe permissions (not world-readable).
fn check_ssh_key_file(key_path: Option<&PathBuf>) -> Result<()> {
    let Some(path) = key_path else {
        eprintln!("[HEALTH] No SSH key configured, skipping");
        return Ok(());
    };
    eprintln!("[HEALTH] Checking SSH key file...");
    if !path.exists() {
        bail!("SSH key file not found: {path:?}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mode = meta.permissions().mode();
            if mode & 0o004 != 0 {
                eprintln!("[WARN] SSH key file is world-readable: {path:?} (mode {:#o}) — may be rejected by SSH", mode);
            }
        }
    }
    eprintln!("[HEALTH] SSH key file OK");
    Ok(())
}

/// Verify project directory exists under project root.
fn check_project_exists(project_root: &Path, project_name: &str) -> Result<PathBuf> {
    let project_dir = project_root.join(project_name);
    eprintln!("[HEALTH] Checking project directory...");
    if !project_dir.exists() {
        bail!("project directory not found: {project_dir:?}");
    }
    if !project_dir.is_dir() {
        bail!("project path is not a directory: {project_dir:?}");
    }
    eprintln!("[HEALTH] Project directory OK");
    Ok(project_dir)
}

/// Verify prompt.md exists (warn-only — file is optional).
fn check_prompt_file(project_dir: &Path) {
    let prompt_path = project_dir.join("prompt.md");
    if prompt_path.exists() {
        eprintln!("[HEALTH] prompt.md found");
    } else {
        eprintln!("[WARN] No prompt.md found at {prompt_path:?} (system prompt will be empty)");
    }
}

/// Verify git repository is valid: .git exists, HEAD readable, origin remote set.
fn check_git_repository(project_dir: &Path) -> Result<()> {
    eprintln!("[HEALTH] Checking git repository...");
    let repo = git2::Repository::open(project_dir)
        .with_context(|| format!("failed to open git repository at {project_dir:?}"))?;
    repo.head()
        .context("repository has no HEAD (maybe empty?")?;
    repo.find_remote("origin")
        .context("remote 'origin' not configured for git push")?;
    eprintln!("[HEALTH] Git repository OK");
    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run once before entering the main controller loop.
///
/// Validates generic infrastructure. Any failure is fatal — the controller
/// exits before allocating any resources.
/// Cluster-specific checks run later when a cluster-mode job is loaded.
pub fn run_startup_checks(args: &Args) -> Result<()> {
    eprintln!("[HEALTH] ===== Startup checks =====");

    check_claude_binary()?;
    check_project_root(&args.project_root)?;
    check_ssh_key_file(args.git_ssh_key.as_ref())?;

    eprintln!("[HEALTH] ===== All startup checks passed =====\n");
    Ok(())
}

/// Run before processing a cluster-mode job. Validates cluster infrastructure.
pub fn run_cluster_startup_checks(args: &Args) -> Result<()> {
    eprintln!("[HEALTH] ===== Cluster startup checks =====");

    check_cluster_ssh(&args.cluster_host)?;
    check_slurm_available(&args.cluster_host)?;
    check_models_directory(&args.cluster_host, &args.models_path)?;
    check_cluster_llama_path(&args.cluster_host, &args.llama_path)?;

    eprintln!("[HEALTH] ===== All cluster startup checks passed =====\n");
    Ok(())
}

/// Run before each job iteration.
///
/// Fast generic checks to detect infrastructure degradation while the controller
/// is running. Cluster-specific checks are run separately when processing a
/// cluster-mode job.
pub fn run_loop_checks(_args: &Args, qc: &mut queue::QueueClient) -> Result<()> {
    qc.ping().context("Redis ping failed — connection may be lost")?;
    check_claude_binary()?;

    Ok(())
}

/// Run after loading job metadata, before allocating cluster resources.
///
/// Validates project-specific requirements so we fail before spending
/// Slurm / SSH tunnel time on a doomed job.
pub fn run_job_preflight_checks(
    project_root: &Path,
    job: &queue::JobMetadata,
    git_ssh_key: Option<&Path>,
) -> Result<()> {
    let project_dir = check_project_exists(project_root, &job.project)?;
    check_prompt_file(&project_dir);

    if git_ssh_key.is_some() {
        check_git_repository(&project_dir)?;
    } else {
        eprintln!("[HEALTH] No SSH key configured, skipping git repository check");
    }

    Ok(())
}

/// Poll the model server endpoint until it responds with HTTP 200 or timeout.
///
/// Runs after the SSH tunnel is established, before launching Claude Code.
/// This prevents Claude from starting while the model server is still loading.
pub fn wait_for_model_endpoint(
    model_url: &str,
    poll_interval: u64,
    poll_timeout: u64,
) -> Result<()> {
    let health_url = format!("{}/models", model_url.trim_end_matches('/'));
    let max_attempts = if poll_interval == 0 {
        u64::MAX
    } else {
        poll_timeout.div_ceil(poll_interval)
    };

    eprintln!("[HEALTH] Waiting for model endpoint at {health_url}...");

    for attempt in 0..max_attempts {
        let output = Command::new("curl")
            .args([
                "-s",
                "-o", "/dev/null",
                "-w", "%{http_code}",
                &health_url,
            ])
            .output();

        match output {
            Ok(out) => {
                let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if status == "200" {
                    eprintln!("[HEALTH] Model endpoint ready (HTTP 200)");
                    return Ok(());
                }
                eprintln!(
                    "[DEBUG] Model endpoint returned HTTP {}, retry {}/{}",
                    status,
                    attempt + 1,
                    max_attempts,
                );
            }
            Err(e) => {
                eprintln!(
                    "[DEBUG] Model endpoint check failed (attempt {}/{}): {e}",
                    attempt + 1,
                    max_attempts,
                );
            }
        }

        sleep(Duration::from_secs(poll_interval));
    }

    bail!(
        "model endpoint at {health_url} not ready within {poll_timeout}s"
    );
}
