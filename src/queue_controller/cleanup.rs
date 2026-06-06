//! Resource lifecycle management for the queue controller loop.
//!
//! Manages external resources (Slurm job, Claude child, claude settings,
//! git branch) allocated during a synthesis iteration.  Provides graceful
//! shutdown on signal, automatic cleanup on unwind via `CleanupGuard`,
//! and per-iteration state reset.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use super::claude;
use super::slurm;

// ---------------------------------------------------------------------------
// Cleanup state
// ---------------------------------------------------------------------------

/// Per-iteration state shared between the main loop and the signal handler.
/// Populated incrementally as each step acquires a resource.
pub(crate) struct CleanupState {
    pub(crate) slurm_job_id: Option<String>,
    pub(crate) cluster_host: String,
    pub(crate) project_dir: Option<PathBuf>,
    pub(crate) settings_backup: Option<PathBuf>,
    pub(crate) claude_child_pid: Option<u32>,
    pub(crate) orig_branch: Option<(PathBuf, String)>,
}

pub(crate) static CLEANUP: Mutex<Option<CleanupState>> = Mutex::new(None);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Run a closure with mutable access to [`CLEANUP`] state.
pub(crate) fn with_cleanup<F>(f: F)
where
    F: FnOnce(&mut CleanupState),
{
    if let Ok(mut guard) = CLEANUP.lock()
        && let Some(ref mut state) = *guard
    {
        f(state);
    }
}

/// Execute cleanup actions for every resource currently tracked in `state`.
/// Does **not** reset fields — call [`cleanup_and_reset`] for that.
pub(crate) fn do_cleanup(state: &CleanupState) {
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

    if let Some((ref project_dir, ref branch_name)) = state.orig_branch
        && let Ok(repo) = git2::Repository::open(project_dir)
    {
        let _ = repo.set_head(&format!("refs/heads/{}", branch_name));
        let _ = repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()));
        eprintln!("[CLEANUP] Restored git branch '{branch_name}'");
    }
}

/// Take the Slurm job ID out of [`CLEANUP`], leaving `None` in its place.
pub(crate) fn take_slurm_job_id() -> Option<String> {
    CLEANUP
        .lock()
        .ok()
        .and_then(|mut guard| guard.as_mut().and_then(|s| s.slurm_job_id.take()))
}

// ---------------------------------------------------------------------------
// Guard
// ---------------------------------------------------------------------------

/// Drop guard: runs cleanup on any remaining state when `run()` exits.
pub(crate) struct CleanupGuard;

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Ok(guard) = CLEANUP.lock()
            && let Some(ref state) = *guard
        {
            do_cleanup(state);
        }
        eprintln!("[CLEANUP] Graceful shutdown complete");
    }
}

// ---------------------------------------------------------------------------
// Per-iteration reset
// ---------------------------------------------------------------------------

/// Run pending cleanup actions and reset per-iteration state.
pub(crate) fn cleanup_and_reset(state: &mut CleanupState) {
    do_cleanup(state);
    state.slurm_job_id = None;
    state.project_dir = None;
    state.settings_backup = None;
    state.claude_child_pid = None;
    state.orig_branch = None;
}
