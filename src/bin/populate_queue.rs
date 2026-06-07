//! Queue population utility.
//!
//! Generates batch of synthesis jobs and enqueues into Redis for the
//! queue controller. Uses deterministic RNG so same seed always
//! produces same job sequence.

use anyhow::{Context, Result, bail};
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
    /// Model filename identifier (cluster mode). Mutually exclusive with --api-url.
    #[arg(long)]
    model: Option<String>,

    /// External API endpoint URL (API mode). Mutually exclusive with --model.
    #[arg(long)]
    api_url: Option<String>,

    /// Model name for ANTHROPIC_MODEL when using --api-url.
    /// Required for API mode; ignored in cluster mode.
    #[arg(long)]
    api_model_name: Option<String>,

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
    let use_model = args.model.is_some();
    let use_api = args.api_url.is_some();

    match (use_model, use_api) {
        (true, true) => bail!("--model and --api-url are mutually exclusive"),
        (false, false) => bail!("exactly one of --model or --api-url must be provided"),
        (true, false) => {
            // Cluster mode.
            let model = args.model.as_ref().unwrap();
            if model.is_empty() {
                bail!("--model must not be empty");
            }
        }
        (false, true) => {
            // API mode.
            let url = args.api_url.as_ref().unwrap();
            if url.is_empty() {
                bail!("--api-url must not be empty");
            }
            if args.api_model_name.as_ref().map_or(true, |s| s.is_empty()) {
                bail!("--api-model-name is required with --api-url");
            }
        }
    }

    if args.project.is_empty() {
        bail!("--project must not be empty");
    }
    if args.iterations == 0 {
        bail!("--iterations must be > 0");
    }
    let prompt = std::fs::read_to_string(&args.prompt_file).with_context(|| {
        format!(
            "failed to read --prompt-file '{}'",
            args.prompt_file.display()
        )
    })?;
    if prompt.trim().is_empty() {
        bail!("prompt file '{}' is empty", args.prompt_file.display());
    }

    // ---- Redis connection ----
    let client = redis::Client::open(args.redis_url.as_str())
        .with_context(|| format!("failed to open Redis at {}", args.redis_url))?;
    let mut conn = client
        .get_connection()
        .context("failed to connect to Redis")?;

    // ---- deterministic RNG ----
    let mut rng = ChaCha8Rng::seed_from_u64(args.seed);

    // ---- determine execution mode ----
    let is_api_mode = args.api_url.is_some();

    // ---- generate + enqueue ----
    for i in 1..=args.iterations {
        let generated_seed = rng.next_u64();

        let (job_key, hash_fields): (String, Vec<(&str, String)>) = if is_api_mode {
            let key = format!("api:{i}");
            let fields = vec![
                ("execution_mode", "api".to_string()),
                ("api_url", args.api_url.as_ref().unwrap().clone()),
                ("model", args.api_model_name.as_ref().unwrap().clone()),
                ("seed", generated_seed.to_string()),
                ("project", args.project.clone()),
                ("prompt", prompt.clone()),
            ];
            (key, fields)
        } else {
            let model = args.model.as_ref().unwrap();
            let key = format!("{model}:{i}");
            let fields = vec![
                ("execution_mode", "cluster".to_string()),
                ("seed", generated_seed.to_string()),
                ("project", args.project.clone()),
                ("prompt", prompt.clone()),
            ];
            (key, fields)
        };

        // Create job hash.
        redis::cmd("HSET")
            .arg(&job_key)
            .arg(&hash_fields)
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
