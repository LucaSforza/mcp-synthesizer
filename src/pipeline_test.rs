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
    // Forge test mock as forge test --json output
    push_ok(
        &mut ctx.pipeline,
        r#"{"A.t.sol:A":{"test_results":{"a()":{"status":"Success","kind":{"Unit":{"gas":50000}}}}}}"#,
    );
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
    push_ok(
        &mut ctx.pipeline,
        r#"{"A.t.sol:A":{"test_results":{"a()":{"status":"Success","kind":{"Unit":{"gas":30000}}}}}}"#,
    );
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
fn test_extract_forge_gas_json() {
    // Unit test
    let json = r#"{"A.t.sol:A":{"test_results":{"a()":{"status":"Success","kind":{"Unit":{"gas":28827}}}}}}"#;
    assert_eq!(extract_forge_gas_json(json), Some(28827));

    // Fuzz test
    let json = r#"{"A.t.sol:A":{"test_results":{"a(uint256)":{"status":"Success","kind":{"Fuzz":{"mean_gas":27545,"runs":256}}}}}}"#;
    assert_eq!(extract_forge_gas_json(json), Some(27545));

    // Mixed: unit + fuzz
    let json = r#"{"A.t.sol:A":{"test_results":{"a()":{"status":"Success","kind":{"Unit":{"gas":1000}}},"b(uint256)":{"status":"Success","kind":{"Fuzz":{"mean_gas":50000,"runs":256}}}}}}"#;
    assert_eq!(extract_forge_gas_json(json), Some(51000));

    // Empty / invalid
    assert_eq!(extract_forge_gas_json("{}"), None);
    assert_eq!(extract_forge_gas_json("invalid"), None);
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
    push_ok(
        &mut ctx.pipeline,
        r#"{"A.t.sol:A":{"test_results":{"a()":{"status":"Success","kind":{"Unit":{"gas":75000}}}}}}"#,
    );
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
