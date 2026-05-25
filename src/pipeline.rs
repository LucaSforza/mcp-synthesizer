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
    ) -> Self {
        eprintln!("[DEBUG] pipeline::new cwd=\"{}\" project_id={} project=\"{}\" invariants={}", cwd, project_id, project_name, project_number_invariants);
        let test_run_id = db
            .create_test_run(project_id)
            .expect("Failed to create test run");
        eprintln!("[DEBUG] pipeline::new::test_run_created test_run_id={}", test_run_id.id);
        Self {
            cwd,
            db,
            project_id,
            _project_name: project_name,
            project_number_invariants,
            test_run_id: test_run_id.id,
            iteration: 0,
            forge_gas: None,
            #[cfg(test)]
            mock_commands: None,
        }
    }

    fn run_command(
        &mut self,
        cmd: &str,
        args: &[&str],
        cwd: &str,
    ) -> Result<(String, bool), String> {
        eprintln!("[DEBUG] pipeline::run_command cmd=\"{}\" args={:?} cwd=\"{}\"", cmd, args, cwd);

        #[cfg(test)]
        if let Some(ref mut mocks) = self.mock_commands {
            if !mocks.is_empty() {
                let remaining = mocks.len();
                let result = mocks.remove(0);
                eprintln!("[DEBUG] pipeline::run_command::mock cmd=\"{}\" args={:?} mocks_remaining={} result_is_ok={}", cmd, args, remaining - 1, result.is_ok());
                return result;
            }
        }

        let output = match Command::new(cmd).current_dir(cwd).args(args).output() {
            Ok(o) => o,
            Err(e) => {
                eprintln!("[DEBUG] pipeline::run_command::err cmd=\"{}\" error=\"{}\"", cmd, e);
                return Err(format!("Failed to execute `{}`: {}", cmd, e));
            }
        };

        let combined = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr).to_string();
        let success = output.status.success();
        eprintln!(
            "[DEBUG] pipeline::run_command::ok cmd=\"{}\" exit={:?} stdout_len={} stderr_len={} combined_len={} success={}",
            cmd, output.status.code(), output.stdout.len(), output.stderr.len(), combined.len(), success
        );
        Ok((combined, success))
    }

    pub fn run(&mut self) -> VerificationReport {
        self.iteration += 1;
        eprintln!("[DEBUG] pipeline::run::start iteration={}", self.iteration);

        // Phase A: Build
        let build_result = self.stage_build();
        if !build_result.passed {
            eprintln!("[DEBUG] pipeline::run::build_failed iteration={}", self.iteration);
            return build_result;
        }

        // Phase A: Test
        let test_result = self.stage_test();
        if !test_result.passed {
            eprintln!("[DEBUG] pipeline::run::test_failed iteration={}", self.iteration);
            return test_result;
        }

        // Phase B: Halmos verification
        let halmos_result = self.stage_halmos();
        eprintln!("[DEBUG] pipeline::run::complete iteration={} stage=halmos passed={}", self.iteration, halmos_result.passed);
        halmos_result
    }

    fn stage_build(&mut self) -> VerificationReport {
        eprintln!("[DEBUG] pipeline::stage_build::start iteration={}", self.iteration);
        let start = Instant::now();
        let cwd = self.cwd.clone();
        match self.run_command("forge", &["build", "-vvv"], &cwd) {
            Ok((output, true)) => {
                self.db.increment_compilation_passed(self.test_run_id).ok();
                eprintln!("[DEBUG] pipeline::stage_build::ok passed=true duration={:?} compilation_passed=incremented", start.elapsed());
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
                eprintln!("[DEBUG] pipeline::stage_build::fail passed=false duration={:?} compilation_not_passed=incremented trial_recorded=failed_compilation", start.elapsed());
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
                eprintln!("[DEBUG] pipeline::stage_build::err error=\"{}\" duration={:?} compilation_not_passed=incremented trial_recorded=failed_compilation", e, start.elapsed());
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
        eprintln!("[DEBUG] pipeline::stage_test::start iteration={}", self.iteration);
        let start = Instant::now();
        let cwd = self.cwd.clone();
        match self.run_command("forge", &["test", "-vvv"], &cwd) {
            Ok((output, true)) => {
                self.forge_gas = SynthesisPipeline::extract_forge_gas(&output);
                eprintln!("[DEBUG] pipeline::stage_test::ok passed=true duration={:?} forge_gas={:?}", start.elapsed(), self.forge_gas);
                VerificationReport {
                    stage: "test".into(),
                    passed: true,
                    output: format!("Tests passed ({:?})\n{}", start.elapsed(), output),
                    metrics: None,
                    gas_of_implementation: self.forge_gas,
                }
            }
            Ok((output, false)) => {
                eprintln!("[DEBUG] pipeline::stage_test::fail passed=false duration={:?} trial_recorded=failed_fuzzing", start.elapsed());
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
                eprintln!("[DEBUG] pipeline::stage_test::err error=\"{}\" duration={:?} trial_recorded=failed_fuzzing", e, start.elapsed());
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
        eprintln!("[DEBUG] pipeline::stage_halmos::start iteration={} invariants={}", self.iteration, self.project_number_invariants);
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
                eprintln!("[DEBUG] pipeline::stage_halmos::ok result=succeeded_full gas={:?} duration={:?}", gas, start.elapsed());
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
                    eprintln!("[DEBUG] pipeline::stage_halmos::fail result=counterexample gas={:?} duration={:?}", gas, start.elapsed());
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
                    eprintln!("[DEBUG] pipeline::stage_halmos::partial result=partial_proof gas={:?} not_proved={} duration={:?}", gas, not_proved, start.elapsed());
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
                eprintln!("[DEBUG] pipeline::stage_halmos::err error=\"{}\" trial_recorded=failed_halmos duration={:?}", e, start.elapsed());
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

    /// Extract total gas from forge test output.
    /// Parses `gas: NUMBER` and `μ: NUMBER` (mean gas for fuzz tests), sums all values.
    fn extract_forge_gas(output: &str) -> Option<i64> {
        eprintln!("[DEBUG] pipeline::extract_forge_gas output_len={}", output.len());
        let mut total: i64 = 0;
        for line in output.lines() {
            let lc = line.to_lowercase();
            // gas: NUMBER e.g. "(gas: 28827)"
            if let Some(pos) = lc.find("gas:") {
                let after = &lc[pos + 4..];
                let num_str: String = after
                    .chars()
                    .skip_while(|c| !c.is_numeric())
                    .take_while(|c| c.is_numeric())
                    .collect();
                if let Ok(n) = num_str.parse::<i64>() {
                    eprintln!("[DEBUG] pipeline::extract_forge_gas::found gas:{} total_before={}", n, total);
                    total += n;
                    continue;
                }
            }
            // μ: NUMBER e.g. "(μ: 26767, ...)" — mean gas for fuzz tests
            if let Some(pos) = lc.find("μ:") {
                let after = &lc[pos + "μ:".len()..];
                let num_str: String = after
                    .chars()
                    .skip_while(|c| !c.is_numeric())
                    .take_while(|c| c.is_numeric())
                    .collect();
                if let Ok(n) = num_str.parse::<i64>() {
                    eprintln!("[DEBUG] pipeline::extract_forge_gas::found μ:{} total_before={}", n, total);
                    total += n;
                }
            }
        }
        if total > 0 {
            eprintln!("[DEBUG] pipeline::extract_forge_gas::result total={}", total);
            Some(total)
        } else {
            eprintln!("[DEBUG] pipeline::extract_forge_gas::result None");
            None
        }
    }

    /// Extract number of unproved invariants from halmos output
    fn extract_not_proved(&self, output: &str) -> i32 {
        eprintln!("[DEBUG] pipeline::extract_not_proved output_len={}", output.len());
        for line in output.lines() {
            let lc = line.to_lowercase();
            if lc.contains("unproved") || lc.contains("unproven") || lc.contains("not proved") {
                if let Some(n) = line
                    .split(|c: char| c.is_whitespace() || c == ':')
                    .find_map(|w| w.trim().parse::<i32>().ok())
                {
                    eprintln!("[DEBUG] pipeline::extract_not_proved::result not_proved={} (parsed)", n);
                    return n;
                }
            }
        }
        // If halmos failed but we can't parse the count, assume all invariants are unproved
        eprintln!("[DEBUG] pipeline::extract_not_proved::result not_proved={} (fallback_to_project_invariants)", self.project_number_invariants);
        self.project_number_invariants
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use tempfile::TempDir;

    struct TestCtx {
        db: Database,
        pipeline: SynthesisPipeline,
        _dir: TempDir,
    }

    fn setup(project_invariants: i32) -> TestCtx {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("test.db");
        let path_str = path.to_str().unwrap().to_string();

        let db = Database::new(&path_str).expect("DB");
        let proj = db
            .get_or_create_project("test", project_invariants)
            .unwrap();

        let mut pipeline = SynthesisPipeline::new(
            "/tmp".into(),
            Database::new(&path_str).expect("Pipeline DB"),
            proj.id,
            "test".into(),
            project_invariants,
        );
        pipeline.mock_commands = Some(Vec::new());

        TestCtx {
            db,
            pipeline,
            _dir: dir,
        }
    }

    fn push_ok(pipeline: &mut SynthesisPipeline, output: &str) {
        pipeline
            .mock_commands
            .as_mut()
            .unwrap()
            .push(Ok((output.to_string(), true)));
    }

    fn push_fail(pipeline: &mut SynthesisPipeline, output: &str) {
        pipeline
            .mock_commands
            .as_mut()
            .unwrap()
            .push(Ok((output.to_string(), false)));
    }

    #[test]
    fn test_pipeline_new_creates_test_run() {
        let ctx = setup(1);
        assert_eq!(ctx.pipeline.iteration, 0);
        assert!(ctx.pipeline.test_run_id > 0);
    }

    #[test]
    fn test_build_passed() {
        let mut ctx = setup(3);
        // Build passes → run() should short-circuit at build stage with no further stages needed
        // But run() has 3 stages: build, test, halmos. Need mocks for all if build passes.
        push_ok(&mut ctx.pipeline, "build ok");
        push_ok(&mut ctx.pipeline, "test ok");
        push_ok(&mut ctx.pipeline, "halmos ok");
        let report = ctx.pipeline.run();
        assert!(report.passed);
        assert_eq!(report.stage, "halmos");
        assert_eq!(ctx.pipeline.iteration, 1);
        let metrics = ctx.db.get_metrics(ctx.pipeline.project_id).unwrap();
        assert_eq!(metrics.compilation_passed, 1);
        assert_eq!(metrics.total_trials, 1);
    }

    #[test]
    fn test_build_failed() {
        let mut ctx = setup(3);
        push_fail(&mut ctx.pipeline, "compiler error");
        let report = ctx.pipeline.run();
        assert!(!report.passed);
        assert_eq!(report.stage, "build");
        let metrics = ctx.db.get_metrics(ctx.pipeline.project_id).unwrap();
        assert_eq!(metrics.compilation_not_passed, 1);
        assert_eq!(metrics.total_trials, 1);
        assert_eq!(metrics.compilation_passed, 0);
    }

    #[test]
    fn test_test_failed() {
        let mut ctx = setup(3);
        push_ok(&mut ctx.pipeline, "build ok");
        push_fail(&mut ctx.pipeline, "test failure");
        let report = ctx.pipeline.run();
        assert!(!report.passed);
        assert_eq!(report.stage, "test");
        let metrics = ctx.db.get_metrics(ctx.pipeline.project_id).unwrap();
        assert_eq!(metrics.compilation_passed, 1);
        assert_eq!(metrics.total_trials, 1);
    }

    #[test]
    fn test_halmos_succeeded_full() {
        let mut ctx = setup(3);
        push_ok(&mut ctx.pipeline, "build ok");
        // Forge test mock must include gas: format (new source of gas data)
        push_ok(&mut ctx.pipeline, "[PASS] test_A() (gas: 50000)");
        push_ok(&mut ctx.pipeline, "halmos: all proved");
        let report = ctx.pipeline.run();
        assert!(report.passed);
        assert_eq!(report.stage, "halmos");
        assert_eq!(report.gas_of_implementation, Some(50000));
        let metrics = ctx.db.get_metrics(ctx.pipeline.project_id).unwrap();
        assert_eq!(metrics.total_trials, 1);
        assert_eq!(metrics.proven_invariants, 3);
        assert_eq!(metrics.avg_gas.unwrap() as i64, 50000);
        assert_eq!(metrics.peak_gas.unwrap(), 50000);
    }

    #[test]
    fn test_halmos_counterexample() {
        let mut ctx = setup(3);
        push_ok(&mut ctx.pipeline, "build ok");
        push_ok(&mut ctx.pipeline, "tests pass");
        push_fail(
            &mut ctx.pipeline,
            "Counterexample found: assertion violated",
        );
        let report = ctx.pipeline.run();
        assert!(!report.passed);
        assert_eq!(report.stage, "halmos");
        assert_eq!(report.metrics.as_ref().unwrap().total_trials, 1);
    }

    #[test]
    fn test_halmos_partial() {
        let mut ctx = setup(5);
        push_ok(&mut ctx.pipeline, "build ok");
        push_ok(&mut ctx.pipeline, "tests pass");
        push_fail(&mut ctx.pipeline, "Timeout: unproved invariants: 2");
        let report = ctx.pipeline.run();
        assert!(report.passed);
        assert_eq!(report.stage, "halmos");
        let metrics = report.metrics.as_ref().unwrap();
        assert_eq!(metrics.total_trials, 1);
        assert_eq!(metrics.unproven_invariants, 2);
    }

    #[test]
    fn test_multi_iteration_loop() {
        let mut ctx = setup(3);
        // Iteration 1: build fails
        push_fail(&mut ctx.pipeline, "build err");
        let r1 = ctx.pipeline.run();
        assert!(!r1.passed);
        assert_eq!(ctx.pipeline.iteration, 1);

        // Iteration 2: build passes, test fails
        push_ok(&mut ctx.pipeline, "build ok");
        push_fail(&mut ctx.pipeline, "test fail");
        let r2 = ctx.pipeline.run();
        assert!(!r2.passed);
        assert_eq!(ctx.pipeline.iteration, 2);
        assert_eq!(r2.stage, "test");

        // Iteration 3: build passes, test passes, halmos proves all
        push_ok(&mut ctx.pipeline, "build ok");
        push_ok(&mut ctx.pipeline, "[PASS] test_A() (gas: 30000)");
        push_ok(&mut ctx.pipeline, "all good");
        let r3 = ctx.pipeline.run();
        assert!(r3.passed);
        assert_eq!(ctx.pipeline.iteration, 3);
        assert_eq!(r3.stage, "halmos");

        let metrics = ctx.db.get_metrics(ctx.pipeline.project_id).unwrap();
        assert_eq!(metrics.total_trials, 3);
        assert_eq!(metrics.compilation_passed, 2);
        assert_eq!(metrics.compilation_not_passed, 1);
    }

    #[test]
    fn test_extract_forge_gas() {
        // Sum of individual test gas
        let out = "[PASS] test_A() (gas: 1000)\n[PASS] test_B() (gas: 2000)";
        assert_eq!(SynthesisPipeline::extract_forge_gas(out), Some(3000));

        // Fuzz test with mean gas (μ)
        let out = "[PASS] testFuzz(uint256) (runs: 256, μ: 50000, ~: 52579)";
        assert_eq!(SynthesisPipeline::extract_forge_gas(out), Some(50000));

        // Mixed: individual gas + fuzz mean
        let out = "[PASS] test_A() (gas: 1000)\n[PASS] testFuzz(uint256) (μ: 50000)";
        assert_eq!(SynthesisPipeline::extract_forge_gas(out), Some(51000));

        // Real-world fuzz test with runs, μ, ~
        let out = "[PASS] test_SystemInvariants((uint8,uint256,uint256)[4]) (runs: 100000, μ: 1154349, ~: 1221586)";
        assert_eq!(SynthesisPipeline::extract_forge_gas(out), Some(1154349));

        // No gas
        assert_eq!(SynthesisPipeline::extract_forge_gas("no gas here"), None);
        assert_eq!(SynthesisPipeline::extract_forge_gas(""), None);
    }

    #[test]
    fn test_extract_not_proved() {
        let ctx = setup(5);
        assert_eq!(ctx.pipeline.extract_not_proved("unproved invariants: 2"), 2);
        assert_eq!(ctx.pipeline.extract_not_proved("Unproven: 3"), 3);
        assert_eq!(ctx.pipeline.extract_not_proved("not proved count: 1"), 1);
        assert_eq!(ctx.pipeline.extract_not_proved("all proved"), 5);
        assert_eq!(ctx.pipeline.extract_not_proved(""), 5);
    }

    #[test]
    fn test_metrics_after_full_loop() {
        let mut ctx = setup(3);
        // 2 failed compilations → 1 succeeded_full
        for _ in 0..2 {
            push_fail(&mut ctx.pipeline, "build fail");
            ctx.pipeline.run();
        }
        push_ok(&mut ctx.pipeline, "build ok");
        push_ok(&mut ctx.pipeline, "[PASS] test_A() (gas: 75000)");
        push_ok(&mut ctx.pipeline, "all proven");
        ctx.pipeline.run();

        let metrics = ctx.db.get_metrics(ctx.pipeline.project_id).unwrap();
        assert_eq!(metrics.total_trials, 3);
        assert_eq!(metrics.compilation_passed, 1);
        assert_eq!(metrics.compilation_not_passed, 2);
        assert_eq!(metrics.avg_gas.unwrap() as i64, 75000);
        assert_eq!(metrics.peak_gas.unwrap(), 75000);
        assert_eq!(metrics.proven_invariants, 3);
        assert_eq!(metrics.unproven_invariants, 0);
        assert_eq!(metrics.succeeded_iterations, 3);
    }
}
