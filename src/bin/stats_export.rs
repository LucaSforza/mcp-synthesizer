//! Statistical analysis of synthesis experiments stored in Redis.
//!
//! Exports a canonical `analysis.json` dataset plus summary reports.
//! Visualization is handled by `scripts/visualize_synthesis.py`.

#[path = "../stats/mod.rs"]
mod stats;

use anyhow::{Context, Result, bail};
use clap::Parser;
use std::path::PathBuf;

use stats::loader::RedisLoader;
use stats::parser::parse_range;
use stats::report::generate_all_reports;
use stats::statistics::compute_statistics;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(name = "stats_export", about = "Export synthesis experiment data from Redis")]
struct Args {
    /// Redis server URL.
    #[arg(long, default_value = "redis://localhost:6379")]
    redis_url: String,

    /// Output directory for reports.
    #[arg(long)]
    output: PathBuf,

    /// Experiment group in format label=start:end. Can be repeated.
    #[arg(short = 'r', long = "range")]
    ranges: Vec<String>,
}

// ---------------------------------------------------------------------------
// Entrypoint
// ---------------------------------------------------------------------------

fn main() {
    match run() {
        Ok(()) => {
            eprintln!("[DEBUG] Stats export completed successfully");
        }
        Err(e) => {
            eprintln!("[ERROR] {e:#}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<()> {
    let args = Args::parse();

    if args.ranges.is_empty() {
        bail!("at least one --range is required");
    }

    // Parse ranges.
    let mut groups_data: Vec<(String, u64, u64)> = Vec::new();
    for range_str in &args.ranges {
        let (label, start, end) =
            parse_range(range_str).with_context(|| format!("invalid range '{range_str}'"))?;
        groups_data.push((label, start, end));
    }

    // Ensure output directory exists.
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("failed to create output directory {:?}", args.output))?;

    // Connect to Redis and load all groups.
    eprintln!("[DEBUG] Loading data from Redis at {}", args.redis_url);
    let mut loader =
        RedisLoader::new(&args.redis_url).context("failed to connect to Redis")?;

    let mut groups = Vec::new();
    for (label, start, end) in &groups_data {
        eprintln!("[DEBUG] Loading group '{label}' (test runs {start}-{end})");
        let group = loader
            .load_group(label, *start, *end)
            .with_context(|| format!("failed to load group '{label}'"))?;
        eprintln!(
            "[DEBUG] Group '{label}': {} observations loaded",
            group.count()
        );
        groups.push(group);
    }

    // Compute and display per-group statistics.
    eprintln!("\n[DEBUG] Per-group statistics:");
    for group in &groups {
        let s = compute_statistics(group);
        eprintln!(
            "  {}: count={} mean={:.0} median={:.0} std_dev={:.0} min={} max={} CV={:.1}% outliers={}",
            s.label,
            s.count,
            s.mean,
            s.median,
            s.std_dev,
            s.min,
            s.max,
            s.coefficient_of_variation * 100.0,
            s.outliers.len(),
        );
    }

    // Generate reports (analysis.json, summary.json, summary.csv, report.md).
    eprintln!("[DEBUG] Generating reports...");
    generate_all_reports(&groups, &args.output)?;

    Ok(())
}
