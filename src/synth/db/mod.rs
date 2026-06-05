#![allow(dead_code)]

use std::fmt;
use std::error::Error as StdError;


// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub number_invariants: i32,
}

#[derive(Debug, Clone)]
pub struct TestRun {
    pub id: i64,
    pub project_id: i64,
    pub compilation_passed: i32,
    pub compilation_not_passed: i32,
}

#[derive(Debug, Clone)]
pub struct SynthesisTrial {
    pub id: i64,
    pub test_run_id: i64,
    pub iteration: i32,
    pub gas_of_implementation: Option<i64>,
    pub result_type: String,
    pub not_proved_invariants: i32,
    pub failure_detail: Option<String>,
    pub is_full_synthesis: bool,
}

#[derive(Debug, Clone)]
pub struct Metrics {
    pub median_gas: Option<f64>,
    pub peak_gas: Option<i64>,
    pub compilation_passed: i32,
    pub compilation_not_passed: i32,
    pub total_trials: i32,
    pub proven_invariants: i32,
    pub unproven_invariants: i32,
    pub succeeded_iterations: i32,
}

const VALID_RESULT_TYPES: &[&str] = &[
    "failed_compilation",
    "failed_fuzzing",
    "succeeded_fuzzing",
    "failed_halmos",
    "succeeded_partial",
    "succeeded_full",
];

// ---------------------------------------------------------------------------
// Unified error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum DbError {
    Redis(::redis::RedisError),
    Sqlite(rusqlite::Error),
    InvalidResultType(String),
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Redis(e) => write!(f, "Redis error: {}", e),
            DbError::Sqlite(e) => write!(f, "SQLite error: {}", e),
            DbError::InvalidResultType(s) => write!(f, "Invalid result type: {}", s),
        }
    }
}

impl StdError for DbError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            DbError::Redis(e) => Some(e),
            DbError::Sqlite(e) => Some(e),
            DbError::InvalidResultType(_) => None,
        }
    }
}

impl From<::redis::RedisError> for DbError {
    fn from(e: ::redis::RedisError) -> Self {
        DbError::Redis(e)
    }
}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self {
        DbError::Sqlite(e)
    }
}

// ---------------------------------------------------------------------------
// Shared validation
// ---------------------------------------------------------------------------

pub(crate) fn validate_trial_params(
    result_type: &str,
    not_proved_invariants: i32,
    project_number_invariants: i32,
) -> Result<(), DbError> {
    if !VALID_RESULT_TYPES.contains(&result_type) {
        return Err(DbError::InvalidResultType(format!(
            "result_type must be one of: {:?}, got: {}",
            VALID_RESULT_TYPES, result_type
        )));
    }
    if result_type.starts_with("succeeded") {
        assert!(
            not_proved_invariants <= project_number_invariants,
            "not_proved_invariants ({}) must be <= number_of_invariants ({})",
            not_proved_invariants,
            project_number_invariants
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

pub trait Database: Send {
    fn get_or_create_project(&self, name: &str, number_invariants: i32) -> Result<Project, DbError>;
    fn create_test_run(&self, project_id: i64) -> Result<TestRun, DbError>;
    #[allow(clippy::too_many_arguments)]
    fn record_trial(
        &self,
        test_run_id: i64,
        iteration: i32,
        gas_of_implementation: Option<i64>,
        result_type: &str,
        not_proved_invariants: i32,
        failure_detail: Option<&str>,
        project_number_invariants: i32,
        is_full_synthesis: bool,
    ) -> Result<SynthesisTrial, DbError>;
    fn get_max_iteration(&self, test_run_id: i64) -> Result<i32, DbError>;
    fn increment_compilation_passed(&self, test_run_id: i64) -> Result<(), DbError>;
    fn increment_compilation_not_passed(&self, test_run_id: i64) -> Result<(), DbError>;
    fn get_project(&self, project_id: i64) -> Result<Project, DbError>;
    fn get_metrics(&self, project_id: i64) -> Result<Metrics, DbError>;
}

// ---------------------------------------------------------------------------
// Connection config factory
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum DbConfig {
    Redis { url: String },
    #[deprecated(note = "SQLite backend is deprecated. Use Redis instead.")]
    Sqlite { path: String },
}

impl DbConfig {
    pub fn connect(&self) -> Result<Box<dyn Database>, DbError> {
        match self {
            DbConfig::Redis { url } => {
                let db = RedisDatabase::new(url)?;
                Ok(Box::new(db))
            }
            #[allow(deprecated)]
            DbConfig::Sqlite { path } => {
                let db = SqliteDatabase::new(path)?;
                Ok(Box::new(db))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sub-modules
// ---------------------------------------------------------------------------

mod redis;
mod sqlite;

pub use redis::RedisDatabase;
#[allow(deprecated)]
pub use sqlite::SqliteDatabase;

#[cfg(test)]
mod redis_test;
#[cfg(test)]
mod sqlite_test;
