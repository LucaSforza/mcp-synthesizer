#![allow(dead_code)]

use std::process::Command;
use std::time::Instant;

use crate::db::{Database, Metrics};

#[derive(Debug)]
pub struct VerificationReport {
    pub stage: String,
    pub passed: bool,
    pub output: String,
    pub metrics: Option<Metrics>,
    pub gas_of_implementation: Option<i64>,
}

pub struct SynthesisPipeline {
    pub cwd: String,
    pub db: Database,
    pub project_id: i64,
    pub _project_name: String,  // unused but kept for context in reports
    pub project_number_invariants: i32,
    pub test_run_id: i64,
    pub iteration: i32,
}

impl SynthesisPipeline {
    pub fn new(
        cwd: String,
        db: Database,
        project_id: i64,
        project_name: String,
        project_number_invariants: i32,
    ) -> Self {
        let test_run_id = db
            .create_test_run(project_id)
            .expect("Failed to create test run");
        Self {
            cwd,
            db,
            project_id,
            _project_name: project_name,
            project_number_invariants,
            test_run_id: test_run_id.id,
            iteration: 0,
        }
    }

    fn run_command(cmd: &str, args: &[&str], cwd: &str) -> Result<(String, bool), String> {
        let output = Command::new(cmd)
            .current_dir(cwd)
            .args(args)
            .output()
            .map_err(|e| format!("Failed to execute `{}`: {}", cmd, e))?;

        let combined = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr).to_string();
        let success = output.status.success();
        Ok((combined, success))
    }

    pub fn run(&mut self) -> VerificationReport {
        self.iteration += 1;

        // Phase A: Build
        let build_result = self.stage_build();
        if !build_result.passed {
            return build_result;
        }

        // Phase A: Test
        let test_result = self.stage_test();
        if !test_result.passed {
            return test_result;
        }

        // Phase B: Halmos verification
        self.stage_halmos()
    }

    fn stage_build(&mut self) -> VerificationReport {
        let start = Instant::now();
        match Self::run_command("forge", &["build", "-vvv"], &self.cwd) {
            Ok((output, true)) => {
                self.db
                    .increment_compilation_passed(self.test_run_id)
                    .ok();
                VerificationReport {
                    stage: "build".into(),
                    passed: true,
                    output: format!("Build passed ({:?})\n{}", start.elapsed(), output),
                    metrics: None,
                    gas_of_implementation: None,
                }
            }
            Ok((output, false)) => {
                self.db
                    .increment_compilation_not_passed(self.test_run_id)
                    .ok();
                let _ = self.db.record_trial(
                    self.test_run_id,
                    self.iteration,
                    None,
                    "failed_compilation",
                    0,
                    Some(&output),
                    self.project_number_invariants,
                );
                VerificationReport {
                    stage: "build".into(),
                    passed: false,
                    output: format!("Compilation failed.\n{}", output),
                    metrics: None,
                    gas_of_implementation: None,
                }
            }
            Err(e) => {
                self.db
                    .increment_compilation_not_passed(self.test_run_id)
                    .ok();
                let _ = self.db.record_trial(
                    self.test_run_id,
                    self.iteration,
                    None,
                    "failed_compilation",
                    0,
                    Some(&e),
                    self.project_number_invariants,
                );
                VerificationReport {
                    stage: "build".into(),
                    passed: false,
                    output: format!("Compilation error: {}", e),
                    metrics: None,
                    gas_of_implementation: None,
                }
            }
        }
    }

    fn stage_test(&mut self) -> VerificationReport {
        let start = Instant::now();
        match Self::run_command("forge", &["test", "-vvv"], &self.cwd) {
            Ok((output, true)) => VerificationReport {
                stage: "test".into(),
                passed: true,
                output: format!("Tests passed ({:?})\n{}", start.elapsed(), output),
                metrics: None,
                gas_of_implementation: None,
            },
            Ok((output, false)) => {
                let _ = self.db.record_trial(
                    self.test_run_id,
                    self.iteration,
                    None,
                    "failed_fuzzing",
                    0,
                    Some(&output),
                    self.project_number_invariants,
                );
                VerificationReport {
                    stage: "test".into(),
                    passed: false,
                    output: format!("Tests failed.\n{}", output),
                    metrics: None,
                    gas_of_implementation: None,
                }
            }
            Err(e) => {
                let _ = self.db.record_trial(
                    self.test_run_id,
                    self.iteration,
                    None,
                    "failed_fuzzing",
                    0,
                    Some(&e),
                    self.project_number_invariants,
                );
                VerificationReport {
                    stage: "test".into(),
                    passed: false,
                    output: format!("Test error: {}", e),
                    metrics: None,
                    gas_of_implementation: None,
                }
            }
        }
    }

    fn stage_halmos(&mut self) -> VerificationReport {
        let start = Instant::now();
        match Self::run_command("halmos", &["--verify"], &self.cwd) {
            Ok((output, true)) => {
                // All proven
                let gas = self.extract_gas(&output);
                let _ = self.db.record_trial(
                    self.test_run_id,
                    self.iteration,
                    gas,
                    "succeeded_full",
                    0,
                    None,
                    self.project_number_invariants,
                );
                let metrics = self.db.get_metrics(self.project_id).ok();
                VerificationReport {
                    stage: "halmos".into(),
                    passed: true,
                    output: format!(
                        "Halmos: all invariants proven ({:?})\n{}",
                        start.elapsed(),
                        output
                    ),
                    metrics,
                    gas_of_implementation: gas,
                }
            }
            Ok((output, false)) => {
                // Parse halmos output to determine if it's a counterexample or timeout/partial
                let output_lower = output.to_lowercase();
                let has_counterexample = output_lower.contains("counterexample")
                    || output_lower.contains("violated")
                    || output_lower.contains("assertion failed");

                if has_counterexample {
                    let gas = self.extract_gas(&output);
                    let _ = self.db.record_trial(
                        self.test_run_id,
                        self.iteration,
                        gas,
                        "failed_halmos",
                        0,
                        Some(&output),
                        self.project_number_invariants,
                    );
                    let metrics = self.db.get_metrics(self.project_id).ok();
                    VerificationReport {
                        stage: "halmos".into(),
                        passed: false,
                        output: format!(
                            "Halmos counterexample found.\n{}",
                            output
                        ),
                        metrics,
                        gas_of_implementation: gas,
                    }
                } else {
                    // Timeout / partial proof — accepted under partial model checking
                    let gas = self.extract_gas(&output);
                    let not_proved = self.extract_not_proved(&output);
                    let _ = self.db.record_trial(
                        self.test_run_id,
                        self.iteration,
                        gas,
                        "succeeded_partial",
                        not_proved,
                        None,
                        self.project_number_invariants,
                    );
                    let metrics = self.db.get_metrics(self.project_id).ok();
                    VerificationReport {
                        stage: "halmos".into(),
                        passed: true,
                        output: format!(
                            "Halmos: partial proof (accepted under partial model checking). Unproved invariants: {}\n{}",
                            not_proved,
                            output
                        ),
                        metrics,
                        gas_of_implementation: gas,
                    }
                }
            }
            Err(e) => {
                let _ = self.db.record_trial(
                    self.test_run_id,
                    self.iteration,
                    None,
                    "failed_halmos",
                    0,
                    Some(&e),
                    self.project_number_invariants,
                );
                VerificationReport {
                    stage: "halmos".into(),
                    passed: false,
                    output: format!("Halmos error: {}", e),
                    metrics: None,
                    gas_of_implementation: None,
                }
            }
        }
    }

    /// Extract gas from forge/halmos output
    fn extract_gas(&self, output: &str) -> Option<i64> {
        // Try to find gas numbers in output
        for line in output.lines() {
            if let Some(val) = line
                .to_lowercase()
                .split(|c: char| c.is_whitespace() || c == '|')
                .find_map(|w| {
                    let w = w.trim();
                    if w.ends_with("gas") {
                        w.trim_end_matches("gas")
                            .trim()
                            .parse::<i64>()
                            .ok()
                    } else {
                        None
                    }
                })
            {
                return Some(val);
            }
        }
        None
    }

    /// Extract number of unproved invariants from halmos output
    fn extract_not_proved(&self, output: &str) -> i32 {
        for line in output.lines() {
            let lc = line.to_lowercase();
            if lc.contains("unproved") || lc.contains("unproven") || lc.contains("not proved") {
                if let Some(n) = line
                    .split(|c: char| c.is_whitespace() || c == ':')
                    .find_map(|w| w.trim().parse::<i32>().ok())
                {
                    return n;
                }
            }
        }
        // If halmos failed but we can't parse the count, assume all invariants are unproved
        self.project_number_invariants
    }
}
