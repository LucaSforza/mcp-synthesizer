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

    let db = Database::new(&db_path)?;
    let project = db.get_or_create_project(&args.project, args.invariants)?;

    let tools = SynthesisTools::new(
        args.cwd,
        db_path,
        db,
        args.project,
        args.invariants,
        project.id,
    );

    let service = tools.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
