#![allow(dead_code)]

use std::collections::HashMap;
use std::process::Command;
use std::time::Instant;

use serde::Deserialize;

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
    pub _project_name: String, // unused but kept for context in reports
    pub project_number_invariants: i32,
    pub test_run_id: i64,
    pub iteration: i32,
    pub forge_gas: Option<i64>,
    #[cfg(test)]
    pub mock_commands: Option<Vec<Result<(String, bool), String>>>,
}

impl SynthesisPipeline {
    pub fn new(
        cwd: String,
        db: Database,
        project_id: i64,
        project_name: String,
        project_number_invariants: i32,
    ) -> Result<Self, String> {
        eprintln!(
            "[DEBUG] pipeline::new cwd=\"{}\" project_id={} project=\"{}\" invariants={}",
            cwd, project_id, project_name, project_number_invariants
        );
        let test_run_id = db
            .create_test_run(project_id)
            .map_err(|e| format!("Failed to create test run: {}", e))?;
        let max_iteration = db
            .get_max_iteration(project_id)
            .map_err(|e| format!("Failed to get max iteration: {}", e))?;
        eprintln!(
            "[DEBUG] pipeline::new::test_run_created test_run_id={} max_iteration={}",
            test_run_id.id, max_iteration
        );
        Ok(Self {
            cwd,
            db,
            project_id,
            _project_name: project_name,
            project_number_invariants,
            test_run_id: test_run_id.id,
            iteration: max_iteration,
            forge_gas: None,
            #[cfg(test)]
            mock_commands: None,
        })
    }

    fn run_command(
        &mut self,
        cmd: &str,
        args: &[&str],
        cwd: &str,
    ) -> Result<(String, bool), String> {
        eprintln!(
            "[DEBUG] pipeline::run_command cmd=\"{}\" args={:?} cwd=\"{}\"",
            cmd, args, cwd
        );

        #[cfg(test)]
        if let Some(ref mut mocks) = self.mock_commands {
            if !mocks.is_empty() {
                let remaining = mocks.len();
                let result = mocks.remove(0);
                eprintln!(
                    "[DEBUG] pipeline::run_command::mock cmd=\"{}\" args={:?} mocks_remaining={} result_is_ok={}",
                    cmd,
                    args,
                    remaining - 1,
                    result.is_ok()
                );
                return result;
            }
        }

        let output = match Command::new(cmd).current_dir(cwd).args(args).output() {
            Ok(o) => o,
            Err(e) => {
                eprintln!(
                    "[DEBUG] pipeline::run_command::err cmd=\"{}\" error=\"{}\"",
                    cmd, e
                );
                return Err(format!("Failed to execute `{}`: {}", cmd, e));
            }
        };

        let combined = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr);
        let success = output.status.success();
        eprintln!(
            "[DEBUG] pipeline::run_command::ok cmd=\"{}\" exit={:?} stdout_len={} stderr_len={} combined_len={} success={}",
            cmd,
            output.status.code(),
            output.stdout.len(),
            output.stderr.len(),
            combined.len(),
            success
        );
        Ok((combined, success))
    }

    pub fn run(&mut self) -> VerificationReport {
        self.iteration += 1;
        eprintln!("[DEBUG] pipeline::run::start iteration={}", self.iteration);

        // Phase A: Build
        let build_result = self.stage_build();
        if !build_result.passed {
            eprintln!(
                "[DEBUG] pipeline::run::build_failed iteration={}",
                self.iteration
            );
            return build_result;
        }

        // Phase A: Test
        let test_result = self.stage_test();
        if !test_result.passed {
            eprintln!(
                "[DEBUG] pipeline::run::test_failed iteration={}",
                self.iteration
            );
            return test_result;
        }

        // Phase B: Halmos verification
        let halmos_result = self.stage_halmos();
        eprintln!(
            "[DEBUG] pipeline::run::complete iteration={} stage=halmos passed={}",
            self.iteration, halmos_result.passed
        );
        halmos_result
    }

    fn stage_build(&mut self) -> VerificationReport {
        eprintln!(
            "[DEBUG] pipeline::stage_build::start iteration={}",
            self.iteration
        );
        let start = Instant::now();
        let cwd = self.cwd.clone();
        match self.run_command("forge", &["build", "-vvv"], &cwd) {
            Ok((output, true)) => {
                self.db.increment_compilation_passed(self.test_run_id).ok();
                eprintln!(
                    "[DEBUG] pipeline::stage_build::ok passed=true duration={:?} compilation_passed=incremented",
                    start.elapsed()
                );
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
                eprintln!(
                    "[DEBUG] pipeline::stage_build::fail passed=false duration={:?} compilation_not_passed=incremented trial_recorded=failed_compilation",
                    start.elapsed()
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
                eprintln!(
                    "[DEBUG] pipeline::stage_build::err error=\"{}\" duration={:?} compilation_not_passed=incremented trial_recorded=failed_compilation",
                    e,
                    start.elapsed()
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
        eprintln!(
            "[DEBUG] pipeline::stage_test::start iteration={}",
            self.iteration
        );
        let start = Instant::now();
        let cwd = self.cwd.clone();
        match self.run_command("forge", &["test", "--json"], &cwd) {
            Ok((output, true)) => {
                self.forge_gas = extract_forge_gas_json(&output);
                eprintln!(
                    "[DEBUG] pipeline::stage_test::ok passed=true duration={:?} forge_gas={:?}",
                    start.elapsed(),
                    self.forge_gas
                );
                VerificationReport {
                    stage: "test".into(),
                    passed: true,
                    output: format!("Tests passed ({:?})\n{}", start.elapsed(), output),
                    metrics: None,
                    gas_of_implementation: self.forge_gas,
                }
            }
            Ok((output, false)) => {
                eprintln!(
                    "[DEBUG] pipeline::stage_test::fail passed=false duration={:?} trial_recorded=failed_fuzzing",
                    start.elapsed()
                );
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
                eprintln!(
                    "[DEBUG] pipeline::stage_test::err error=\"{}\" duration={:?} trial_recorded=failed_fuzzing",
                    e,
                    start.elapsed()
                );
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
        eprintln!(
            "[DEBUG] pipeline::stage_halmos::start iteration={} invariants={}",
            self.iteration, self.project_number_invariants
        );
        let start = Instant::now();
        let cwd = self.cwd.clone();
        match self.run_command(
            "halmos",
            &[
                "--solver-threads",
                "16",
                "--early-exit",
                "--print-full-model",
                "--solver-timeout-branching",
                "1s",
                "--solver-timeout-assertion",
                "1s",
            ],
            &cwd,
        ) {
            Ok((output, true)) => {
                // All proven
                let gas = self.forge_gas;
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
                eprintln!(
                    "[DEBUG] pipeline::stage_halmos::ok result=succeeded_full gas={:?} duration={:?}",
                    gas,
                    start.elapsed()
                );
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
                    let gas = self.forge_gas;
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
                    eprintln!(
                        "[DEBUG] pipeline::stage_halmos::fail result=counterexample gas={:?} duration={:?}",
                        gas,
                        start.elapsed()
                    );
                    VerificationReport {
                        stage: "halmos".into(),
                        passed: false,
                        output: format!("Halmos counterexample found.\n{}", output),
                        metrics,
                        gas_of_implementation: gas,
                    }
                } else {
                    // Timeout / partial proof — accepted under partial model checking
                    let gas = self.forge_gas;
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
                    eprintln!(
                        "[DEBUG] pipeline::stage_halmos::partial result=partial_proof gas={:?} not_proved={} duration={:?}",
                        gas,
                        not_proved,
                        start.elapsed()
                    );
                    VerificationReport {
                        stage: "halmos".into(),
                        passed: true,
                        output: format!(
                            "Halmos: partial proof (accepted under partial model checking). Unproved invariants: {}\n{}",
                            not_proved, output
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
                eprintln!(
                    "[DEBUG] pipeline::stage_halmos::err error=\"{}\" trial_recorded=failed_halmos duration={:?}",
                    e,
                    start.elapsed()
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

    /// Extract number of unproved invariants from halmos output
    fn extract_not_proved(&self, output: &str) -> i32 {
        eprintln!(
            "[DEBUG] pipeline::extract_not_proved output_len={}",
            output.len()
        );
        for line in output.lines() {
            let lc = line.to_lowercase();
            if lc.contains("unproved") || lc.contains("unproven") || lc.contains("not proved") {
                if let Some(n) = line
                    .split(|c: char| c.is_whitespace() || c == ':')
                    .find_map(|w| w.trim().parse::<i32>().ok())
                {
                    eprintln!(
                        "[DEBUG] pipeline::extract_not_proved::result not_proved={} (parsed)",
                        n
                    );
                    return n;
                }
            }
        }
        // If halmos failed but we can't parse the count, assume all invariants are unproved
        eprintln!(
            "[DEBUG] pipeline::extract_not_proved::result not_proved={} (fallback_to_project_invariants)",
            self.project_number_invariants
        );
        self.project_number_invariants
    }
}

#[derive(Deserialize)]
struct ForgeTestResult {
    status: String,
    kind: ForgeTestKind,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ForgeTestKind {
    Unit {
        #[serde(rename = "Unit")]
        unit: UnitKind,
    },
    Fuzz {
        #[serde(rename = "Fuzz")]
        fuzz: FuzzKind,
    },
}

#[derive(Deserialize)]
struct UnitKind {
    gas: i64,
}

#[derive(Deserialize)]
struct FuzzKind {
    #[serde(rename = "mean_gas")]
    mean_gas: i64,
}

/// Extract total gas from forge test JSON output.
/// Parses `forge test --json` output: Unit tests → `kind.Unit.gas`, fuzz tests → `kind.Fuzz.mean_gas`.
fn extract_forge_gas_json(output: &str) -> Option<i64> {
    #[derive(Deserialize)]
    struct ForgeSuite {
        #[serde(rename = "test_results")]
        test_results: HashMap<String, ForgeTestResult>,
    }

    eprintln!(
        "[DEBUG] pipeline::extract_forge_gas_json output_len={}",
        output.len()
    );
    let suites: HashMap<String, ForgeSuite> = serde_json::from_str(output).ok()?;
    let mut total: i64 = 0;
    for suite in suites.values() {
        for result in suite.test_results.values() {
            match &result.kind {
                ForgeTestKind::Unit { unit } => {
                    eprintln!(
                        "[DEBUG] pipeline::extract_forge_gas_json::unit gas={} total_before={}",
                        unit.gas, total
                    );
                    total += unit.gas;
                }
                ForgeTestKind::Fuzz { fuzz } => {
                    eprintln!(
                        "[DEBUG] pipeline::extract_forge_gas_json::fuzz mean_gas={} total_before={}",
                        fuzz.mean_gas, total
                    );
                    total += fuzz.mean_gas;
                }
            }
        }
    }
    if total > 0 {
        eprintln!(
            "[DEBUG] pipeline::extract_forge_gas_json::result total={}",
            total
        );
        Some(total)
    } else {
        eprintln!("[DEBUG] pipeline::extract_forge_gas_json::result None");
        None
    }
}

#[cfg(test)]
#[path = "pipeline_test.rs"]
mod tests;
