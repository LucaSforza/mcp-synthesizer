use super::*;

fn setup_db() -> Database {
    Database::new(":memory:").expect("Failed to create in-memory DB")
}

#[test]
fn test_create_project() {
    let db = setup_db();
    let project = db.get_or_create_project("test-project", 5).unwrap();
    assert_eq!(project.name, "test-project");
    assert_eq!(project.number_invariants, 5);
    assert!(project.id > 0);
}

#[test]
fn test_get_existing_project() {
    let db = setup_db();
    let p1 = db.get_or_create_project("my-proj", 3).unwrap();
    let p2 = db.get_or_create_project("my-proj", 3).unwrap();
    assert_eq!(p1.id, p2.id);
    assert_eq!(p1.name, p2.name);
}

#[test]
fn test_create_test_run() {
    let db = setup_db();
    let project = db.get_or_create_project("p", 2).unwrap();
    let tr = db.create_test_run(project.id).unwrap();
    assert_eq!(tr.project_id, project.id);
    assert_eq!(tr.compilation_passed, 0);
    assert_eq!(tr.compilation_not_passed, 0);
}

#[test]
fn test_record_trial_failed_compilation() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 3).unwrap();
    let tr = db.create_test_run(proj.id).unwrap();
    let trial = db
        .record_trial(
            tr.id,
            1,
            None,
            "failed_compilation",
            0,
            Some("err"),
            proj.number_invariants,
        )
        .unwrap();
    assert_eq!(trial.test_run_id, tr.id);
    assert_eq!(trial.iteration, 1);
    assert_eq!(trial.gas_of_implementation, None);
    assert_eq!(trial.result_type, "failed_compilation");
    assert_eq!(trial.not_proved_invariants, 0);
    assert_eq!(trial.failure_detail, Some("err".to_string()));
}

#[test]
fn test_record_trial_succeeded_full() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 3).unwrap();
    let tr = db.create_test_run(proj.id).unwrap();
    let trial = db
        .record_trial(
            tr.id,
            1,
            Some(50000),
            "succeeded_full",
            0,
            None,
            proj.number_invariants,
        )
        .unwrap();
    assert_eq!(trial.result_type, "succeeded_full");
    assert_eq!(trial.gas_of_implementation, Some(50000));
    assert_eq!(trial.not_proved_invariants, 0);
    assert_eq!(trial.failure_detail, None);
}

#[test]
fn test_record_trial_succeeded_partial() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 5).unwrap();
    let tr = db.create_test_run(proj.id).unwrap();
    let trial = db
        .record_trial(
            tr.id,
            1,
            Some(30000),
            "succeeded_partial",
            2,
            None,
            proj.number_invariants,
        )
        .unwrap();
    assert_eq!(trial.result_type, "succeeded_partial");
    assert_eq!(trial.not_proved_invariants, 2);
}

#[test]
#[should_panic(expected = "not_proved_invariants")]
fn test_invariants_constraint() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 3).unwrap();
    let tr = db.create_test_run(proj.id).unwrap();
    // 5 unproven > 3 invariants → should panic
    db.record_trial(
        tr.id,
        1,
        None,
        "succeeded_partial",
        5,
        None,
        proj.number_invariants,
    )
    .unwrap();
}

#[test]
fn test_increment_compilation_passed() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 1).unwrap();
    let tr = db.create_test_run(proj.id).unwrap();
    db.increment_compilation_passed(tr.id).unwrap();
    let metrics = db.get_metrics(proj.id).unwrap();
    assert_eq!(metrics.compilation_passed, 1);
    assert_eq!(metrics.compilation_not_passed, 0);
}

#[test]
fn test_increment_compilation_not_passed() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 1).unwrap();
    let tr = db.create_test_run(proj.id).unwrap();
    db.increment_compilation_not_passed(tr.id).unwrap();
    let metrics = db.get_metrics(proj.id).unwrap();
    assert_eq!(metrics.compilation_passed, 0);
    assert_eq!(metrics.compilation_not_passed, 1);
}

#[test]
fn test_get_metrics_empty() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 5).unwrap();
    let metrics = db.get_metrics(proj.id).unwrap();
    assert_eq!(metrics.total_trials, 0);
    assert_eq!(metrics.proven_invariants, 0);
    assert_eq!(metrics.unproven_invariants, 0);
    assert_eq!(metrics.compilation_passed, 0);
    assert_eq!(metrics.compilation_not_passed, 0);
    assert_eq!(metrics.succeeded_iterations, 0);
    assert!(metrics.avg_gas.is_none());
    assert!(metrics.peak_gas.is_none());
}

#[test]
fn test_get_metrics_aggregation() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 5).unwrap();

    // Two test runs with multiple trials
    let tr1 = db.create_test_run(proj.id).unwrap();
    let tr2 = db.create_test_run(proj.id).unwrap();

    // tr1: 2 passed, 1 failed compilation
    db.increment_compilation_passed(tr1.id).unwrap();
    db.increment_compilation_passed(tr1.id).unwrap();
    db.increment_compilation_not_passed(tr1.id).unwrap();
    db.record_trial(tr1.id, 1, None, "failed_compilation", 0, Some("err"), 5)
        .unwrap();

    // tr2: 1 succeeded_full with gas
    db.record_trial(tr2.id, 1, Some(100000), "succeeded_full", 0, None, 5)
        .unwrap();
    db.record_trial(tr2.id, 2, Some(90000), "succeeded_partial", 2, None, 5)
        .unwrap();

    let metrics = db.get_metrics(proj.id).unwrap();
    assert_eq!(metrics.compilation_passed, 2);
    assert_eq!(metrics.compilation_not_passed, 1);
    assert_eq!(metrics.total_trials, 3);
    assert_eq!(metrics.avg_gas.unwrap() as i64, 95000); // (100000 + 90000) / 2
    assert_eq!(metrics.peak_gas.unwrap(), 100000);
    assert_eq!(metrics.proven_invariants, 5); // 5 - 0 for succeeded_full
    assert_eq!(metrics.unproven_invariants, 2); // from succeeded_partial
    assert_eq!(metrics.succeeded_iterations, 1); // first succeeded at iteration 1
}

#[test]
fn test_result_type_check_constraint() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 1).unwrap();
    let tr = db.create_test_run(proj.id).unwrap();
    let result = db.record_trial(tr.id, 1, None, "invalid_type", 0, None, 1);
    assert!(result.is_err());
}
