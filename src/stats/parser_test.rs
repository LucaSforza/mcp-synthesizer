use super::*;

#[test]
fn test_parse_valid() {
    let (label, start, end) = parse_range("test=1:50").expect("parse should succeed");
    assert_eq!(label, "test");
    assert_eq!(start, 1);
    assert_eq!(end, 50);
}

#[test]
fn test_parse_multi_digit() {
    let (label, start, end) = parse_range("experiment_42=100:200").expect("parse should succeed");
    assert_eq!(label, "experiment_42");
    assert_eq!(start, 100);
    assert_eq!(end, 200);
}

#[test]
fn test_parse_single_run() {
    let (label, start, end) = parse_range("single=5:5").expect("parse should succeed");
    assert_eq!(label, "single");
    assert_eq!(start, 5);
    assert_eq!(end, 5);
}

#[test]
fn test_parse_label_with_dash() {
    let (label, start, end) = parse_range("my-group=1:10").expect("parse should succeed");
    assert_eq!(label, "my-group");
    assert_eq!(start, 1);
    assert_eq!(end, 10);
}

#[test]
fn test_parse_missing_equals() {
    let result = parse_range("invalid");
    assert!(result.is_err(), "expected error for missing '='");
}

#[test]
fn test_parse_missing_colon() {
    let result = parse_range("label=123");
    assert!(result.is_err(), "expected error for missing ':'");
}

#[test]
fn test_parse_empty_label() {
    let result = parse_range("=1:50");
    assert!(result.is_err(), "expected error for empty label");
}

#[test]
fn test_parse_invalid_start() {
    let result = parse_range("label=abc:50");
    assert!(result.is_err(), "expected error for invalid start");
}

#[test]
fn test_parse_invalid_end() {
    let result = parse_range("label=1:xyz");
    assert!(result.is_err(), "expected error for invalid end");
}

#[test]
fn test_parse_zero_start() {
    let result = parse_range("label=0:50");
    assert!(result.is_err(), "expected error for zero start");
}

#[test]
fn test_parse_end_less_than_start() {
    let result = parse_range("label=10:5");
    assert!(result.is_err(), "expected error for end < start");
}
