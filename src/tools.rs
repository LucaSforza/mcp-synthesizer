use std::process::Command;
use std::sync::Mutex;

use rmcp::{tool, tool_router};

use crate::db::{Database, DbConfig};
use crate::pipeline::{extract_forge_gas_json, SynthesisPipeline};

pub struct SynthesisTools {
    pub cwd: String,
    pub db_config: DbConfig,
    pub db: Mutex<Box<dyn Database>>,
    pub project_name: String,
    pub number_invariants: i32,
    pub project_id: i64,
    pub test_run_id: i64,
    pub pipeline: Mutex<Option<SynthesisPipeline>>,
}

impl SynthesisTools {
    pub fn new(
        cwd: String,
        db_config: DbConfig,
        db: Box<dyn Database>,
        project_name: String,
        number_invariants: i32,
        project_id: i64,
    ) -> Self {
        eprintln!(
            "[DEBUG] tools::new cwd=\"{}\" project=\"{}\" invariants={} project_id={}",
            cwd, project_name, number_invariants, project_id
        );
        let test_run = db
            .create_test_run(project_id)
            .expect("Failed to create test run for standalone tools");
        eprintln!("[DEBUG] tools::new test_run_id={}", test_run.id);
        Self {
            cwd,
            db_config,
            db: Mutex::new(db),
            project_name,
            number_invariants,
            project_id,
            test_run_id: test_run.id,
            pipeline: Mutex::new(None),
        }
    }
}

impl SynthesisTools {
    fn next_iteration(&self) -> i32 {
        self.db
            .lock()
            .ok()
            .and_then(|db| db.get_max_iteration(self.test_run_id).ok())
            .map(|n| n + 1)
            .unwrap_or(1)
    }
}

#[tool_router(server_handler)]
impl SynthesisTools {
    #[tool(
        description = "Install Foundry project dependencies. Runs `forge install` in the project directory.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn forge_install(&self) -> Result<String, String> {
        eprintln!("[DEBUG] tools::forge_install cwd=\"{}\"", self.cwd);
        let output = Command::new("forge")
            .current_dir(&self.cwd)
            .arg("install")
            .output()
            .map_err(|e| {
                eprintln!("[DEBUG] tools::forge_install::err io_error=\"{}\"", e);
                format!("Failed to execute forge install: {}", e)
            })?;

        let combined = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            eprintln!(
                "[DEBUG] tools::forge_install::ok status=success output_len={}",
                combined.len()
            );
            Ok(combined)
        } else {
            eprintln!(
                "[DEBUG] tools::forge_install::err status=failed output_len={}",
                combined.len()
            );
            Err(format!("forge install failed.\n{}", combined))
        }
    }

    #[tool(
        description = "Compile the Foundry project with `forge build`. Returns compiler output. Captures compilation success/failure telemetry.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn forge_build(&self) -> Result<String, String> {
        eprintln!("[DEBUG] tools::forge_build cwd=\"{}\"", self.cwd);
        let output = Command::new("forge")
            .current_dir(&self.cwd)
            .args(["build", "-vvv"])
            .output()
            .map_err(|e| {
                eprintln!("[DEBUG] tools::forge_build::err io_error=\"{}\"", e);
                format!("Failed to execute forge build: {}", e)
            })?;

        let combined = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            self.db
                .lock()
                .ok()
                .and_then(|db| db.increment_compilation_passed(self.test_run_id).ok());
            eprintln!(
                "[DEBUG] tools::forge_build::ok status=success output_len={}",
                combined.len()
            );
            Ok(format!("Build passed.\n{}", combined))
        } else {
            self.db
                .lock()
                .ok()
                .and_then(|db| db.increment_compilation_not_passed(self.test_run_id).ok());
            eprintln!(
                "[DEBUG] tools::forge_build::err status=failed output_len={}",
                combined.len()
            );
            Err(format!("Build failed.\n{}", combined))
        }
    }

    #[tool(
        description = "Run Foundry unit and fuzzy tests with `forge test`. Returns detailed failure logs if assertions or fuzzing invariants fail.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    fn forge_test(&self) -> Result<String, String> {
        eprintln!("[DEBUG] tools::forge_test cwd=\"{}\"", self.cwd);
        let output = Command::new("forge")
            .current_dir(&self.cwd)
            .args(["test", "--json"])
            .output()
            .map_err(|e| {
                eprintln!("[DEBUG] tools::forge_test::err io_error=\"{}\"", e);
                format!("Failed to execute forge test: {}", e)
            })?;

        let combined = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr).to_string();

        let iteration = self.next_iteration();
        let gas = extract_forge_gas_json(&combined);

        if output.status.success() {
            self.db.lock().ok().and_then(|db| {
                db.record_trial(
                    self.test_run_id,
                    iteration,
                    gas,
                    "succeeded_fuzzing",
                    0,
                    None,
                    self.number_invariants,
                    false,
                )
                .ok()
            });
            eprintln!(
                "[DEBUG] tools::forge_test::ok status=success output_len={} gas={:?}",
                combined.len(),
                gas
            );
            Ok(format!("Tests passed.\n{}", combined))
        } else {
            self.db.lock().ok().and_then(|db| {
                db.record_trial(
                    self.test_run_id,
                    iteration,
                    gas,
                    "failed_fuzzing",
                    0,
                    Some(&combined),
                    self.number_invariants,
                    false,
                )
                .ok()
            });
            eprintln!(
                "[DEBUG] tools::forge_test::err status=failed output_len={} gas={:?}",
                combined.len(),
                gas
            );
            Err(format!("Tests failed.\n{}", combined))
        }
    }

    #[tool(
        description = "Run the full synthesis pipeline: compile with forge build, run unit/fuzzy tests with forge test, then verify with Halmos. Records every attempt in the database and returns a verification report with gas metrics, test results, and invariant proof status.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    fn run_synthesis(&self) -> Result<String, String> {
        eprintln!(
            "[DEBUG] tools::run_synthesis cwd=\"{}\" project=\"{}\" invariants={}",
            self.cwd, self.project_name, self.number_invariants
        );
        let mut pipe_lock = self
            .pipeline
            .lock()
            .map_err(|e| format!("Internal lock error: {}", e))?;

        if pipe_lock.is_none() {
            eprintln!("[DEBUG] tools::run_synthesis::lazy_init creating pipeline");
            *pipe_lock = Some(
                SynthesisPipeline::new(
                    self.cwd.clone(),
                    self.db_config.connect()
                        .map_err(|e| format!("Failed to open DB: {}", e))?,
                    self.project_id,
                    self.project_name.clone(),
                    self.number_invariants,
                    self.test_run_id,
                )
                .map_err(|e| format!("Failed to init pipeline: {}", e))?,
            );
        }

        let pipeline = pipe_lock.as_mut().unwrap();
        eprintln!(
            "[DEBUG] tools::run_synthesis::before_run iteration={}",
            pipeline.iteration
        );
        let report = pipeline.run();
        eprintln!(
            "[DEBUG] tools::run_synthesis::report iteration={} stage={} passed={} has_metrics={}",
            pipeline.iteration,
            report.stage,
            report.passed,
            report.metrics.is_some()
        );

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
                 Median gas: {:?}\n\
                 Peak gas: {:?}\n\
                 Compilation passed: {}\n\
                 Compilation not passed: {}\n\
                 Total trials: {}\n\
                 Proven invariants: {}\n\
                 Unproven invariants: {}\n\
                 Succeeded at iteration: {}\n",
                metrics.median_gas,
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
            eprintln!(
                "[DEBUG] tools::run_synthesis::ok iteration={} stage={}",
                pipeline.iteration, report.stage
            );
            Ok(result)
        } else {
            eprintln!(
                "[DEBUG] tools::run_synthesis::err iteration={} stage={}",
                pipeline.iteration, report.stage
            );
            Err(result)
        }
    }
}
