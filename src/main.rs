use std::process::Command;

use clap::Parser;
use rmcp::{tool, tool_router, transport::stdio, ServiceExt};
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path of the directory to work inside of
    #[arg(short, long)]
    cwd: String,
}
#[derive(Clone)]
struct SolTools {
    cwd: String,
}

impl SolTools {
    fn new(_cwd: String) -> Self {
        Self { cwd: _cwd }
    }
}

#[tool_router(server_handler)]
impl SolTools {
    // #[tool(description = "Add two numbers")]
    // fn add(&self, Parameters(AddParams { a, b }): Parameters<AddParams>) -> String {
    //     eprintln!("Summing");
    //     (a + b).to_string()
    // }
    //
    #[tool(description = "compile foundry project")]
    fn compile(&self) -> String {
        let out = Command::new("forge")
            .current_dir(self.cwd.clone())
            .arg("build")
            .arg("-vvv")
            .arg("2>&1")
            .output()
            .expect("failed to execute process");

        String::from_utf8(out.stdout).unwrap()
    }
    #[tool(description = "run foundry project fuzzy test")]
    fn fuzzy_testing(&self) -> String {
        let out = Command::new("forge")
            .current_dir(self.cwd.clone())
            .arg("test")
            .arg("-vvv")
            .arg("2>&1")
            .output()
            .expect("failed to execute process");

        String::from_utf8(out.stdout).unwrap()
    }
    #[tool(description = "verify contracts - runs invariant tests with Foundry, then applies Halmos symbolic execution for formal verification of passed invariants")]
    fn verify(&self) -> String {
        // Step 1: Run Foundry's invariant testing
        let forge_out = Command::new("forge")
            .current_dir(self.cwd.clone())
            .arg("test")
            .arg("--match-contract")
            .arg("SystemInvariantTest")
            .arg("-vvv")
            .output();

        let forge_output = match forge_out {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                format!("=== FORGE INVARIANT TEST RESULTS ===\n{}\n\n=== FORGE STDERR ===\n{}", stdout, stderr)
            }
            Err(e) => format!("=== FORGE INVARIANT TEST RESULTS ===\nError executing forge test: {}\n", e),
        };

        // Step 2: Run Halmos for formal verification on the test contract
        let halmos_out = Command::new("halmos")
            .current_dir(self.cwd.clone())
            .arg("--match-contract")
            .arg("SystemInvariantTest")
            .output();

        let halmos_output = match halmos_out {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                format!("\n=== HALMOS FORMAL VERIFICATION RESULTS ===\n{}\n\n=== HALMOS STDERR ===\n{}", stdout, stderr)
            }
            Err(e) => format!("\n=== HALMOS FORMAL VERIFICATION RESULTS ===\nError executing halmos: {}\n", e),
        };

        // Combine results with clear separators
        format!("{}{}", forge_output, halmos_output)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let service = SolTools::new(args.cwd).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
