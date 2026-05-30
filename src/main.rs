mod db;
mod pipeline;
mod tools;

use clap::Parser;
use rmcp::{transport::stdio, ServiceExt};

use db::DbConfig;
use tools::SynthesisTools;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path of the Foundry project directory
    #[arg(short, long)]
    cwd: String,

    /// Database backend type: "redis" (default) or "sqlite"
    #[arg(long, default_value = "redis")]
    db_type: String,

    /// Redis server URL (used when --db-type=redis, default: redis://localhost:6379)
    #[arg(short = 'u', long)]
    redis_url: Option<String>,

    /// SQLite database file path (used when --db-type=sqlite)
    #[arg(short = 'l', long)]
    db_path: Option<String>,

    /// Project name identifier
    #[arg(short, long)]
    project: String,

    /// Number of invariants to verify in Halmos
    #[arg(short, long, default_value_t = 0)]
    invariants: i32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let db_config = match args.db_type.as_str() {
        "redis" => DbConfig::Redis {
            url: args.redis_url.unwrap_or_else(|| "redis://localhost:6379".into()),
        },
        "sqlite" => DbConfig::Sqlite {
            path: args.db_path.unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                format!("{}/Documents/solidity-synthesis.db", home)
            }),
        },
        other => anyhow::bail!(
            "Unsupported db_type '{}'. Supported: redis, sqlite", other,
        ),
    };

    eprintln!(
        "[DEBUG] main::start cwd=\"{}\" project=\"{}\" invariants={} db_config={:?}",
        args.cwd, args.project, args.invariants, db_config
    );

    let db = db_config.connect()?;
    eprintln!("[DEBUG] main::database_created");

    let project = db.get_or_create_project(&args.project, args.invariants)?;
    eprintln!(
        "[DEBUG] main::project id={} name=\"{}\" invariants={}",
        project.id, project.name, project.number_invariants
    );

    let tools = SynthesisTools::new(
        args.cwd,
        db_config,
        db,
        args.project,
        args.invariants,
        project.id,
    );

    eprintln!(
        "[DEBUG] main::tools_created project=\"{}\" invariants={} project_id={}",
        tools.project_name, tools.number_invariants, tools.project_id
    );

    let service = tools.serve(stdio()).await?;
    eprintln!("[DEBUG] main::server_listening transport=stdio");
    let result = service.waiting().await;
    if let Err(ref e) = result {
        eprintln!("[DEBUG] main::fatal error={:?}", e);
    }
    result?;
    Ok(())
}
