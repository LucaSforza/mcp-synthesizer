use std::process::Command;
use std::sync::Mutex;

use rmcp::{tool, tool_router};

use crate::db::Database;
use crate::pipeline::SynthesisPipeline;

pub struct SynthesisTools {
    pub cwd: String,
    pub db_path: String,
    pub db: Mutex<Database>,
    pub project_name: String,
    pub number_invariants: i32,
    pub project_id: i64,
    pub pipeline: Mutex<Option<SynthesisPipeline>>,
}

impl SynthesisTools {
    pub fn new(
        cwd: String,
        db_path: String,
        db: Database,
        project_name: String,
        number_invariants: i32,
        project_id: i64,
    ) -> Self {
        Self {
            cwd,
            db_path,
            db: Mutex::new(db),
            project_name,
            number_invariants,
            project_id,
            pipeline: Mutex::new(None),
        }
    }
}

#[tool_router(server_handler)]
impl SynthesisTools {
    #[tool(
        description = "Install Foundry project dependencies. Runs `forge install` in the project directory.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true)
    )]
    fn forge_install(&self) -> Result<String, String> {
        let output = Command::new("forge")
            .current_dir(&self.cwd)
            .arg("install")
            .output()
            .map_err(|e| format!("Failed to execute forge install: {}", e))?;

        let combined =
            String::from_utf8_lossy(&output.stdout).to_string()
                + &String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(combined)
        } else {
            Err(format!("forge install failed.\n{}", combined))
        }
    }

    #[tool(
        description = "Compile the Foundry project with `forge build`. Returns compiler output. Captures compilation success/failure telemetry.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true)
    )]
    fn forge_build(&self) -> Result<String, String> {
        let output = Command::new("forge")
            .current_dir(&self.cwd)
            .args(["build", "-vvv"])
            .output()
            .map_err(|e| format!("Failed to execute forge build: {}", e))?;

        let combined =
            String::from_utf8_lossy(&output.stdout).to_string()
                + &String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            self.db.lock().ok().and_then(|db| db.increment_compilation_passed(self.project_id).ok());
            Ok(format!("Build passed.\n{}", combined))
        } else {
            self.db.lock().ok().and_then(|db| db.increment_compilation_not_passed(self.project_id).ok());
            Err(format!("Build failed.\n{}", combined))
        }
    }

    #[tool(
        description = "Run Foundry unit and fuzzy tests with `forge test`. Returns detailed failure logs if assertions or fuzzing invariants fail.",
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true)
    )]
    fn forge_test(&self) -> Result<String, String> {
        let output = Command::new("forge")
            .current_dir(&self.cwd)
            .args(["test", "-vvv"])
            .output()
            .map_err(|e| format!("Failed to execute forge test: {}", e))?;

        let combined =
            String::from_utf8_lossy(&output.stdout).to_string()
                + &String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(format!("Tests passed.\n{}", combined))
        } else {
            Err(format!("Tests failed.\n{}", combined))
        }
    }

    #[tool(
        description = "Run the full synthesis pipeline: compile with forge build, run unit/fuzzy tests with forge test, then verify with Halmos. Records every attempt in the database and returns a verification report with gas metrics, test results, and invariant proof status.",
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false)
    )]
    fn run_synthesis(&self) -> Result<String, String> {
        let mut pipe_lock = self
            .pipeline
            .lock()
            .map_err(|e| format!("Internal lock error: {}", e))?;

        if pipe_lock.is_none() {
            *pipe_lock = Some(SynthesisPipeline::new(
                self.cwd.clone(),
                Database::new(&self.db_path)
                    .map_err(|e| format!("Failed to open DB: {}", e))?,
                self.project_id,
                self.project_name.clone(),
                self.number_invariants,
            ));
        }

        let pipeline = pipe_lock.as_mut().unwrap();
        let report = pipeline.run();

        let mut result = format!(
            "=== Synthesis Pipeline Report ===\n\
             Project: {}\n\
             Iteration: {}\n\
             Stage: {}\n\
             Passed: {}\n\n{}",
            self.project_name,
            pipeline.iteration,
            report.stage,
            if report.passed { "yes" } else { "no" },
            report.output,
        );

        if let Some(metrics) = &report.metrics {
            result.push_str(&format!(
                "\n--- Metrics ---\n\
                 Avg gas: {:?}\n\
                 Peak gas: {:?}\n\
                 Compilation passed: {}\n\
                 Compilation not passed: {}\n\
                 Total trials: {}\n\
                 Proven invariants: {}\n\
                 Unproven invariants: {}\n\
                 Succeeded at iteration: {}\n",
                metrics.avg_gas,
                metrics.peak_gas,
                metrics.compilation_passed,
                metrics.compilation_not_passed,
                metrics.total_trials,
                metrics.proven_invariants,
                metrics.unproven_invariants,
                metrics.succeeded_iterations,
            ));
        }

        if report.passed {
            Ok(result)
        } else {
            Err(result)
        }
    }
}
