//! Synthesis monitor: polls Claude Code and Slurm job during synthesis,
//! recovers model server when Slurm job expires.
//!
//! Replaces the blocking `claude_child.wait()` with a polling loop that
//! monitors both processes and resubmits the model-serving job on expiry.

use std::path::{Path, PathBuf};
use std::process::Child;
use std::thread::sleep;
use std::time::Duration;

use anyhow::Result;

use super::cleanup::with_cleanup;
use super::slurm::{self, TunnelHandle};

/// Monitors Claude Code execution and Slurm job health.
///
/// Owns the SSH tunnel handle and current Slurm job ID. When the
/// model-serving job terminates, automatically recreates it and
/// re-establishes the tunnel.
pub(crate) struct SynthesisMonitor {
    // Recovery parameters (stored for resubmission).
    models_path: PathBuf,
    model_name: String,
    llama_path: String,
    seed: String,
    ctx_size: u64,
    cluster_host: String,
    tunnel_port: u16,
    poll_interval: u64,
    poll_timeout: u64,

    // Owned state.
    slurm_job_id: String,
    tunnel: TunnelHandle,
}

impl SynthesisMonitor {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        slurm_job_id: String,
        tunnel: TunnelHandle,
        models_path: &Path,
        model_name: &str,
        llama_path: &str,
        seed: &str,
        ctx_size: u64,
        cluster_host: &str,
        tunnel_port: u16,
        poll_interval: u64,
        poll_timeout: u64,
    ) -> Self {
        Self {
            models_path: models_path.to_path_buf(),
            model_name: model_name.to_string(),
            llama_path: llama_path.to_string(),
            seed: seed.to_string(),
            ctx_size,
            cluster_host: cluster_host.to_string(),
            tunnel_port,
            poll_interval,
            poll_timeout,
            slurm_job_id,
            tunnel,
        }
    }

    /// Poll Claude Code exit status and Slurm job health until Claude finishes.
    ///
    /// If the Slurm job enters a terminal state (Completed, Failed, Cancelled,
    /// Timeout, NotFound) the model server is automatically recovered via
    /// [`Self::recover`].
    pub(crate) fn wait_for_completion(
        &mut self,
        claude_child: &mut Child,
    ) -> Result<std::process::ExitStatus> {
        let poll_duration = Duration::from_secs(self.poll_interval);

        loop {
            // Check Claude Code exit status (non-blocking).
            match claude_child.try_wait() {
                Ok(Some(status)) => {
                    eprintln!("[DEBUG] Claude Code exited with status {status}");
                    return Ok(status);
                }
                Ok(None) => { /* still running */ }
                Err(e) => {
                    anyhow::bail!("failed to check Claude Code status: {e}");
                }
            }

            // Check Slurm job health.
            match slurm::get_job_state(&self.cluster_host, &self.slurm_job_id) {
                Ok(ref state) => {
                    if state.is_terminal() {
                        eprintln!(
                            "[DEBUG] Slurm job {} entered terminal state {state}, recovering...",
                            self.slurm_job_id,
                        );
                        self.recover()?;
                    }
                }
                Err(e) => {
                    eprintln!("[WARN] Failed to check Slurm job state: {e:#}");
                }
            }

            sleep(poll_duration);
        }
    }

    /// Resubmit the model-serving job and re-establish the SSH tunnel.
    fn recover(&mut self) -> Result<()> {
        eprintln!("[RECOVERY] Submitting new Slurm job...");
        let model_path = self.models_path.join(&self.model_name);
        let sbatch = slurm::generate_sbatch(&model_path, &self.llama_path, &self.seed, self.ctx_size);
        let new_job_id = slurm::submit_sbatch(&self.cluster_host, &sbatch)?;
        eprintln!("[RECOVERY] Submitted new Slurm job {new_job_id}");

        eprintln!("[RECOVERY] Waiting for new job to reach RUNNING...");
        slurm::poll_job(
            &self.cluster_host,
            &new_job_id,
            self.poll_interval,
            self.poll_timeout,
        )?;

        let node_name = slurm::get_job_node(&self.cluster_host, &new_job_id)?;
        let node_ip = slurm::node_name_to_ip(&node_name);
        let new_tunnel = slurm::establish_tunnel(&self.cluster_host, &node_ip, self.tunnel_port)?;

        // Replace tunnel: the old handle drops here, closing the old SSH tunnel.
        // The log from the old handle's Drop appears before this message.
        self.tunnel = new_tunnel;
        self.slurm_job_id = new_job_id;
        eprintln!(
            "[RECOVERY] New SSH tunnel established to {node_ip}:{}",
            self.tunnel_port,
        );

        // Sync cleanup state so signal/guard cancels the right job.
        with_cleanup(|s| s.slurm_job_id = Some(self.slurm_job_id.clone()));
        eprintln!(
            "[RECOVERY] Model server recovered, job {}",
            self.slurm_job_id,
        );

        Ok(())
    }
}
