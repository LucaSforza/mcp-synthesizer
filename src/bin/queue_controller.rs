//! Queue controller entrypoint.
//!
//! Module tree lives in `src/queue_controller/`.
//! This file is just the thin binary wrapper and CLI argument definition.

#[path = "../queue_controller/mod.rs"]
mod app;

use std::path::PathBuf;

use clap::Parser;

/// Queue controller: automated synthesis execution on Slurm cluster.
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

fn main() {
    let args = Args::parse();
    match app::run(args) {
        Ok(()) => eprintln!("[DEBUG] Queue controller finished successfully"),
        Err(e) => {
            eprintln!("[ERROR] {e:#}");
            std::process::exit(1);
        }
    }
}
