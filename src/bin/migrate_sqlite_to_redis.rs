use clap::Parser;
use redis::{Client, Commands};
use rusqlite::Connection;

#[derive(Parser, Debug)]
#[command(name = "migrate", about = "Migrate SQLite data to Redis")]
struct Args {
    /// Path to existing SQLite database file
    #[arg(short, long)]
    sqlite_path: String,

    /// Redis server URL
    #[arg(short = 'u', long, default_value = "redis://localhost:6379")]
    redis_url: String,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    eprintln!(
        "[MIGRATE] Opening SQLite: {}",
        args.sqlite_path
    );
    let sqlite = Connection::open(&args.sqlite_path)?;

    eprintln!("[MIGRATE] Connecting to Redis: {}", args.redis_url);
    let client = Client::open(args.redis_url.as_str())?;
    let mut redis = client.get_connection()?;

    // --- Migrate projects ---
    eprintln!("[MIGRATE] Migrating projects...");
    let mut stmt = sqlite.prepare(
        "SELECT id, name, number_invariants, created_at FROM project ORDER BY id",
    )?;
    let projects: Vec<(i64, String, i32, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get::<_, String>(3).unwrap_or_default(),
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    for (id, name, invariants, created_at) in &projects {
        let key = format!("project:{}", id);
        let _: bool = redis.hset(&key, "name", name.as_str())?;
        let _: bool = redis.hset(&key, "number_invariants", &invariants.to_string())?;
        let _: bool = redis.hset(&key, "created_at", created_at)?;
        let _: () = redis.set(format!("project:name:{}", name), *id)?;
        eprintln!("  project {}: \"{}\" ({} invariants)", id, name, invariants);
    }
    let max_project_id = projects.last().map(|(id, _, _, _)| *id).unwrap_or(0);
    let _: () = redis.set("project:ids", max_project_id)?;
    eprintln!("  -> project:ids initialized to {}", max_project_id);

    // --- Migrate test_runs ---
    eprintln!("[MIGRATE] Migrating test runs...");
    let mut stmt = sqlite.prepare(
        "SELECT id, project_id, compilation_passed, compilation_not_passed, created_at FROM test_run ORDER BY id",
    )?;
    let test_runs: Vec<(i64, i64, i32, i32, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get::<_, String>(4).unwrap_or_default(),
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    for (id, project_id, comp_p, comp_np, created_at) in &test_runs {
        let key = format!("test_run:{}", id);
        let _: bool = redis.hset(&key, "project_id", &project_id.to_string())?;
        let _: bool = redis.hset(&key, "compilation_passed", &comp_p.to_string())?;
        let _: bool = redis.hset(&key, "compilation_not_passed", &comp_np.to_string())?;
        let _: bool = redis.hset(&key, "created_at", created_at)?;
        let _: i64 = redis.sadd(format!("test_run:by_project:{}", project_id), *id)?;
        eprintln!("  test_run {} (project {})", id, project_id);
    }
    let max_tr_id = test_runs.last().map(|(id, _, _, _, _)| *id).unwrap_or(0);
    let _: () = redis.set("test_run:ids", max_tr_id)?;
    eprintln!("  -> test_run:ids initialized to {}", max_tr_id);

    // --- Migrate synthesis_trials ---
    eprintln!("[MIGRATE] Migrating synthesis trials...");
    let mut stmt = sqlite.prepare(
        "SELECT id, test_run_id, iteration, gas_of_implementation, result_type, not_proved_invariants, failure_detail, is_full_synthesis, created_at FROM synthesis_trial ORDER BY id",
    )?;
    let trials: Vec<(i64, i64, i32, Option<i64>, String, i32, Option<String>, bool, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get::<_, i32>(7)? != 0,
                row.get::<_, String>(8).unwrap_or_default(),
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    for (id, test_run_id, iteration, gas, result_type, npi, fd, ifs, created_at) in &trials {
        let key = format!("synthesis_trial:{}", id);
        let _: bool = redis.hset(&key, "test_run_id", &test_run_id.to_string())?;
        let _: bool = redis.hset(&key, "iteration", &iteration.to_string())?;
        let _: bool = redis.hset(&key, "result_type", result_type.as_str())?;
        let _: bool = redis.hset(&key, "not_proved_invariants", &npi.to_string())?;
        let _: bool = redis.hset(&key, "is_full_synthesis", &(*ifs as i32).to_string())?;
        let _: bool = redis.hset(&key, "created_at", created_at)?;
        if let Some(g) = gas {
            let _: bool = redis.hset(&key, "gas_of_implementation", &g.to_string())?;
        }
        if let Some(detail) = fd {
            let _: bool = redis.hset(&key, "failure_detail", detail.as_str())?;
        }

        let _: i64 = redis.zadd(
            format!("synthesis_trial:by_test_run:{}", test_run_id),
            *id,
            *iteration as f64,
        )?;

        // Look up project_id for project-level indices
        let pid_str: String = redis.hget(format!("test_run:{}", test_run_id), "project_id")?;
        let project_id: i64 = pid_str.parse().unwrap_or(0);

        let _: i64 = redis.sadd(
            format!("synthesis_trial:by_project:{}", project_id),
            *id,
        )?;
        if let Some(g) = gas {
            let _: i64 = redis.zadd(
                format!("synthesis_trial:gas:by_project:{}", project_id),
                *id,
                *g as f64,
            )?;
        }
        eprintln!("  trial {} (test_run {}, iteration {})", id, test_run_id, iteration);
    }
    let max_trial_id = trials.last().map(|(id, _, _, _, _, _, _, _, _)| *id).unwrap_or(0);
    let _: () = redis.set("synthesis_trial:ids", max_trial_id)?;
    eprintln!("  -> synthesis_trial:ids initialized to {}", max_trial_id);

    eprintln!(
        "[MIGRATE] Migration complete: {} projects, {} test runs, {} trials",
        projects.len(),
        test_runs.len(),
        trials.len()
    );
    Ok(())
}
