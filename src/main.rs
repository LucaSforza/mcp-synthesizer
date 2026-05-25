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

    /// Path to the SQLite database (default: $HOME/Documents/solidity-synthesis.db)
    #[arg(short, long)]
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

    let db_path = args.db_path.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{}/Documents/solidity-synthesis.db", home)
    });

    eprintln!("[DEBUG] main::start cwd=\"{}\" project=\"{}\" invariants={} db_path=\"{}\"", args.cwd, args.project, args.invariants, db_path);

    let db = Database::new(&db_path)?;
    eprintln!("[DEBUG] main::database_created path=\"{}\"", db_path);

    let project = db.get_or_create_project(&args.project, args.invariants)?;
    eprintln!("[DEBUG] main::project id={} name=\"{}\" invariants={}", project.id, project.name, project.number_invariants);

    let tools = SynthesisTools::new(
        args.cwd,
        db_path,
        db,
        args.project,
        args.invariants,
        project.id,
    );

    eprintln!("[DEBUG] main::tools_created cwd=\"{}\" db_path=\"{}\" project=\"{}\" invariants={} project_id={}", tools.cwd, tools.db_path, tools.project_name, tools.number_invariants, tools.project_id);

    let service = tools.serve(stdio()).await?;
    eprintln!("[DEBUG] main::server_listening transport=stdio");
    let result = service.waiting().await;
    if let Err(ref e) = result {
        eprintln!("[DEBUG] main::fatal error={:?}", e);
    }
    result?;
    Ok(())
}
