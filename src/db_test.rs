use super::*;

fn setup_db() -> Database {
    let url = std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
    let db = Database::new(&url).expect("Failed to connect to Redis");
    // Flush all data for test isolation
    let mut conn = db.client.get_connection().expect("conn");
    let _: () = redis::cmd("FLUSHALL").query(&mut conn).expect("flushall");
    db
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
            false,
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
            true,
        )
        .unwrap();
    assert_eq!(trial.result_type, "succeeded_full");
    assert_eq!(trial.gas_of_implementation, Some(50000));
    assert_eq!(trial.not_proved_invariants, 0);
    assert_eq!(trial.failure_detail, None);
    assert!(trial.is_full_synthesis);
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
            false,
        )
        .unwrap();
    assert_eq!(trial.result_type, "succeeded_partial");
    assert_eq!(trial.not_proved_invariants, 2);
}

#[test]
fn test_record_trial_fuzzing_only() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 3).unwrap();
    let tr = db.create_test_run(proj.id).unwrap();
    let trial = db
        .record_trial(
            tr.id,
            1,
            None,
            "succeeded_fuzzing",
            0,
            None,
            proj.number_invariants,
            false,
        )
        .unwrap();
    assert_eq!(trial.result_type, "succeeded_fuzzing");
    assert_eq!(trial.is_full_synthesis, false);
    assert_eq!(trial.not_proved_invariants, 0);
    assert_eq!(trial.gas_of_implementation, None);
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
        false,
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
    assert!(metrics.median_gas.is_none());
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
    db.record_trial(
        tr1.id,
        1,
        None,
        "failed_compilation",
        0,
        Some("err"),
        5,
        false,
    )
    .unwrap();

    // tr2: 1 succeeded_full with gas
    db.record_trial(tr2.id, 1, Some(100000), "succeeded_full", 0, None, 5, false)
        .unwrap();
    db.record_trial(
        tr2.id,
        2,
        Some(90000),
        "succeeded_partial",
        2,
        None,
        5,
        false,
    )
    .unwrap();

    let metrics = db.get_metrics(proj.id).unwrap();
    assert_eq!(metrics.compilation_passed, 2);
    assert_eq!(metrics.compilation_not_passed, 1);
    assert_eq!(metrics.total_trials, 3);
    assert_eq!(metrics.median_gas.unwrap() as i64, 95000); // (100000 + 90000) / 2
    assert_eq!(metrics.peak_gas.unwrap(), 100000);
    assert_eq!(metrics.proven_invariants, 5); // 5 - 0 for succeeded_full
    assert_eq!(metrics.unproven_invariants, 2); // from succeeded_partial
    assert_eq!(metrics.succeeded_iterations, 1); // first succeeded at iteration 1
}

#[test]
fn test_get_max_iteration_empty_db() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 3).unwrap();
    let tr = db.create_test_run(proj.id).unwrap();
    let max = db.get_max_iteration(tr.id).unwrap();
    assert_eq!(max, 0);
}

#[test]
fn test_get_max_iteration_with_trials() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 3).unwrap();
    let tr1 = db.create_test_run(proj.id).unwrap();
    let tr2 = db.create_test_run(proj.id).unwrap();

    // tr1: iterations 1, 2
    db.record_trial(
        tr1.id,
        1,
        None,
        "failed_compilation",
        0,
        Some("err"),
        3,
        false,
    )
    .unwrap();
    db.record_trial(tr1.id, 2, Some(50000), "succeeded_full", 0, None, 3, false)
        .unwrap();

    // tr2: iteration 5
    db.record_trial(tr2.id, 5, None, "failed_fuzzing", 0, Some("fail"), 3, false)
        .unwrap();

    assert_eq!(db.get_max_iteration(tr1.id).unwrap(), 2);
    assert_eq!(db.get_max_iteration(tr2.id).unwrap(), 5);
}

#[test]
fn test_result_type_check_constraint() {
    let db = setup_db();
    let proj = db.get_or_create_project("p", 1).unwrap();
    let tr = db.create_test_run(proj.id).unwrap();
    let result = db.record_trial(tr.id, 1, None, "invalid_type", 0, None, 1, false);
    assert!(result.is_err());
}
