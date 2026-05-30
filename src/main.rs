mod db;
mod pipeline;
mod tools;

use clap::Parser;
use rmcp::{transport::stdio, ServiceExt};

use db::Database;
use tools::SynthesisTools;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path of the Foundry project directory
    #[arg(short, long)]
    cwd: String,

    /// Redis server URL (default: redis://localhost:6379)
    #[arg(short = 'u', long)]
    redis_url: Option<String>,

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

    let redis_url = args.redis_url.unwrap_or_else(|| "redis://localhost:6379".into());

    eprintln!(
        "[DEBUG] main::start cwd=\"{}\" project=\"{}\" invariants={} redis_url=\"{}\"",
        args.cwd, args.project, args.invariants, redis_url
    );

    let db = Database::new(&redis_url)?;
    eprintln!("[DEBUG] main::database_created url=\"{}\"", redis_url);

    let project = db.get_or_create_project(&args.project, args.invariants)?;
    eprintln!(
        "[DEBUG] main::project id={} name=\"{}\" invariants={}",
        project.id, project.name, project.number_invariants
    );

    let tools = SynthesisTools::new(
        args.cwd,
        redis_url,
        db,
        args.project,
        args.invariants,
        project.id,
    );

    eprintln!(
        "[DEBUG] main::tools_created cwd=\"{}\" redis_url=\"{}\" project=\"{}\" invariants={} project_id={}",
        tools.cwd, tools.redis_url, tools.project_name, tools.number_invariants, tools.project_id
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
