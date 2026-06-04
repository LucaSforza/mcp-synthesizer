//! Queue population utility.
//!
//! Generates batch of synthesis jobs and enqueues into Redis for the
//! queue controller. Uses deterministic RNG so same seed always
//! produces same job sequence.

use anyhow::{bail, Context, Result};
use clap::Parser;
use rand_chacha::ChaCha8Rng;
use rand_core::{RngCore, SeedableRng};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "populate_queue")]
struct Args {
    /// Model filename identifier (stored in Redis, used by controller).
    #[arg(long)]
    model: String,

    /// Initial RNG seed for deterministic job seed generation.
    #[arg(long)]
    seed: u64,

    /// Target synthesis project identifier.
    #[arg(long)]
    project: String,

    /// Path to file containing synthesis prompt.
    #[arg(long)]
    prompt_file: PathBuf,

    /// Number of jobs to generate (must be > 0).
    #[arg(long)]
    iterations: u32,

    /// Redis server URL.
    #[arg(long, default_value = "redis://localhost:6379")]
    redis_url: String,
}

// ---------------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------------

fn main() {
    match run() {
        Ok(count) => {
            eprintln!("[DEBUG] Enqueued {count} jobs successfully");
        }
        Err(e) => {
            eprintln!("[ERROR] {e:#}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<u32> {
    let args = Args::parse();

    // ---- validation (all upfront) ----
    if args.model.is_empty() {
        bail!("--model must not be empty");
    }
    if args.project.is_empty() {
        bail!("--project must not be empty");
    }
    if args.iterations == 0 {
        bail!("--iterations must be > 0");
    }
    let prompt = std::fs::read_to_string(&args.prompt_file)
        .with_context(|| format!("failed to read --prompt-file '{}'", args.prompt_file.display()))?;
    if prompt.trim().is_empty() {
        bail!("prompt file '{}' is empty", args.prompt_file.display());
    }

    // ---- Redis connection ----
    let client = redis::Client::open(args.redis_url.as_str())
        .with_context(|| format!("failed to open Redis at {}", args.redis_url))?;
    let mut conn = client.get_connection().context("failed to connect to Redis")?;

    // ---- deterministic RNG ----
    let mut rng = ChaCha8Rng::seed_from_u64(args.seed);

    // ---- generate + enqueue ----
    for i in 1..=args.iterations {
        let generated_seed = rng.next_u64();
        let job_key = format!("{}:{}", args.model, i);

        // Create job hash.
        let hash_fields = &[
            ("seed", &generated_seed.to_string()),
            ("project", &args.project),
            ("prompt", &prompt),
        ];
        redis::cmd("HSET")
            .arg(&job_key)
            .arg(hash_fields)
            .query::<()>(&mut conn)
            .with_context(|| format!("failed to HSET {job_key} (iteration {i})"))?;

        // Enqueue in priority sorted set.
        redis::cmd("ZADD")
            .arg("cluster_runs")
            .arg(i as f64)
            .arg(&job_key)
            .query::<()>(&mut conn)
            .with_context(|| format!("failed to ZADD {job_key} (iteration {i})"))?;
    }

    Ok(args.iterations)
}
