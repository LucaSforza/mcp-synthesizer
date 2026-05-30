#![allow(dead_code)]

use std::fmt;
use std::error::Error as StdError;

use chrono::Utc;
use redis::Commands;

use rusqlite::{params, Connection as SqliteConnection};

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
    Redis(redis::RedisError),
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

impl From<redis::RedisError> for DbError {
    fn from(e: redis::RedisError) -> Self {
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

fn validate_trial_params(
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
    Sqlite { path: String },
}

impl DbConfig {
    pub fn connect(&self) -> Result<Box<dyn Database>, DbError> {
        match self {
            DbConfig::Redis { url } => {
                let db = RedisDatabase::new(url)?;
                Ok(Box::new(db))
            }
            DbConfig::Sqlite { path } => {
                let db = SqliteDatabase::new(path)?;
                Ok(Box::new(db))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Redis implementation
// ---------------------------------------------------------------------------

pub struct RedisDatabase {
    pub client: redis::Client,
}

impl RedisDatabase {
    pub fn new(redis_url: &str) -> Result<Self, DbError> {
        eprintln!("[DEBUG] RedisDatabase::new redis_url=\"{}\"", redis_url);
        let client = redis::Client::open(redis_url)?;
        eprintln!("[DEBUG] RedisDatabase::new::ok");
        Ok(Self { client })
    }
}

impl Database for RedisDatabase {
    fn get_or_create_project(
        &self,
        name: &str,
        number_invariants: i32,
    ) -> Result<Project, DbError> {
        eprintln!(
            "[DEBUG] RedisDatabase::get_or_create_project name=\"{}\" number_invariants={}",
            name, number_invariants
        );
        let mut conn = self.client.get_connection()?;
        let name_key = format!("project:name:{}", name);
        let existing_id: Option<i64> = conn.get(&name_key)?;

        match existing_id {
            Some(id) => {
                let project_key = format!("project:{}", id);
                let stored_name: String = conn.hget(&project_key, "name")?;
                let inv_str: String = conn.hget(&project_key, "number_invariants")?;
                let inv = inv_str.parse::<i32>().unwrap_or(0);
                eprintln!("[DEBUG] RedisDatabase::get_or_create_project::found id={}", id);
                Ok(Project {
                    id,
                    name: stored_name,
                    number_invariants: inv,
                })
            }
            None => {
                let id: i64 = conn.incr("project:ids", 1)?;
                let project_key = format!("project:{}", id);
                let now = Utc::now().to_rfc3339();
                let _: bool = conn.hset(&project_key, "name", name)?;
                let _: bool = conn.hset(
                    &project_key,
                    "number_invariants",
                    &number_invariants.to_string(),
                )?;
                let _: bool = conn.hset(&project_key, "created_at", &now)?;
                let _: () = conn.set(&name_key, id)?;
                eprintln!("[DEBUG] RedisDatabase::get_or_create_project::created id={}", id);
                Ok(Project {
                    id,
                    name: name.to_string(),
                    number_invariants,
                })
            }
        }
    }

    fn create_test_run(&self, project_id: i64) -> Result<TestRun, DbError> {
        eprintln!("[DEBUG] RedisDatabase::create_test_run project_id={}", project_id);
        let mut conn = self.client.get_connection()?;
        let id: i64 = conn.incr("test_run:ids", 1)?;
        let key = format!("test_run:{}", id);
        let now = Utc::now().to_rfc3339();
        let _: bool = conn.hset(&key, "project_id", &project_id.to_string())?;
        let _: bool = conn.hset(&key, "compilation_passed", "0")?;
        let _: bool = conn.hset(&key, "compilation_not_passed", "0")?;
        let _: bool = conn.hset(&key, "created_at", &now)?;
        let _: i64 = conn.sadd(format!("test_run:by_project:{}", project_id), id)?;
        eprintln!("[DEBUG] RedisDatabase::create_test_run::ok id={}", id);
        Ok(TestRun {
            id,
            project_id,
            compilation_passed: 0,
            compilation_not_passed: 0,
        })
    }

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
    ) -> Result<SynthesisTrial, DbError> {
        eprintln!(
            "[DEBUG] RedisDatabase::record_trial test_run_id={} iteration={} gas={:?} result_type=\"{}\" not_proved={} project_invariants={} is_full_synthesis={}",
            test_run_id,
            iteration,
            gas_of_implementation,
            result_type,
            not_proved_invariants,
            project_number_invariants,
            is_full_synthesis
        );

        validate_trial_params(result_type, not_proved_invariants, project_number_invariants)?;

        let mut conn = self.client.get_connection()?;
        let id: i64 = conn.incr("synthesis_trial:ids", 1)?;
        let trial_key = format!("synthesis_trial:{}", id);
        let now = Utc::now().to_rfc3339();

        let _: bool = conn.hset(&trial_key, "test_run_id", &test_run_id.to_string())?;
        let _: bool = conn.hset(&trial_key, "iteration", &iteration.to_string())?;
        let _: bool = conn.hset(&trial_key, "result_type", result_type)?;
        let _: bool = conn.hset(
            &trial_key,
            "not_proved_invariants",
            &not_proved_invariants.to_string(),
        )?;
        let _: bool = conn.hset(
            &trial_key,
            "is_full_synthesis",
            &(is_full_synthesis as i32).to_string(),
        )?;
        let _: bool = conn.hset(&trial_key, "created_at", &now)?;
        if let Some(gas) = gas_of_implementation {
            let _: bool = conn.hset(&trial_key, "gas_of_implementation", &gas.to_string())?;
        }
        if let Some(detail) = failure_detail {
            let _: bool = conn.hset(&trial_key, "failure_detail", detail)?;
        }

        // Index by test_run (sorted by iteration)
        let _: i64 = conn.zadd(
            format!("synthesis_trial:by_test_run:{}", test_run_id),
            id,
            iteration as f64,
        )?;

        // Look up project_id for project-level indices
        let tr_key = format!("test_run:{}", test_run_id);
        let pid_str: String = conn.hget(&tr_key, "project_id")?;
        let project_id: i64 = pid_str.parse().unwrap_or(0);

        let _: i64 = conn.sadd(
            format!("synthesis_trial:by_project:{}", project_id),
            id,
        )?;
        if let Some(gas) = gas_of_implementation {
            let _: i64 = conn.zadd(
                format!("synthesis_trial:gas:by_project:{}", project_id),
                id,
                gas as f64,
            )?;
        }

        eprintln!("[DEBUG] RedisDatabase::record_trial::ok trial_id={}", id);
        Ok(SynthesisTrial {
            id,
            test_run_id,
            iteration,
            gas_of_implementation,
            result_type: result_type.to_string(),
            not_proved_invariants,
            failure_detail: failure_detail.map(|s| s.to_string()),
            is_full_synthesis,
        })
    }

    fn get_max_iteration(&self, test_run_id: i64) -> Result<i32, DbError> {
        let mut conn = self.client.get_connection()?;
        let key = format!("synthesis_trial:by_test_run:{}", test_run_id);
        let entries: Vec<(String, f64)> = conn.zrevrange_withscores(&key, 0, 0)?;
        let max = if entries.is_empty() {
            0
        } else {
            entries[0].1 as i32
        };
        eprintln!(
            "[DEBUG] RedisDatabase::get_max_iteration test_run_id={} max={}",
            test_run_id, max
        );
        Ok(max)
    }

    fn increment_compilation_passed(&self, test_run_id: i64) -> Result<(), DbError> {
        eprintln!(
            "[DEBUG] RedisDatabase::increment_compilation_passed test_run_id={}",
            test_run_id
        );
        let mut conn = self.client.get_connection()?;
        let _: i64 =
            conn.hincr(format!("test_run:{}", test_run_id), "compilation_passed", 1)?;
        Ok(())
    }

    fn increment_compilation_not_passed(&self, test_run_id: i64) -> Result<(), DbError> {
        eprintln!(
            "[DEBUG] RedisDatabase::increment_compilation_not_passed test_run_id={}",
            test_run_id
        );
        let mut conn = self.client.get_connection()?;
        let _: i64 = conn.hincr(
            format!("test_run:{}", test_run_id),
            "compilation_not_passed",
            1,
        )?;
        Ok(())
    }

    fn get_project(&self, project_id: i64) -> Result<Project, DbError> {
        eprintln!("[DEBUG] RedisDatabase::get_project project_id={}", project_id);
        let mut conn = self.client.get_connection()?;
        let key = format!("project:{}", project_id);
        let name: String = conn.hget(&key, "name")?;
        let inv_str: String = conn.hget(&key, "number_invariants")?;
        let number_invariants = inv_str.parse::<i32>().unwrap_or(0);
        eprintln!(
            "[DEBUG] RedisDatabase::get_project::ok id={} name=\"{}\" invariants={}",
            project_id, name, number_invariants
        );
        Ok(Project {
            id: project_id,
            name,
            number_invariants,
        })
    }

    fn get_metrics(&self, project_id: i64) -> Result<Metrics, DbError> {
        eprintln!("[DEBUG] RedisDatabase::get_metrics project_id={}", project_id);
        let mut conn = self.client.get_connection()?;

        // Gas metrics from gas sorted set
        let gas_key = format!("synthesis_trial:gas:by_project:{}", project_id);
        let gas_entries: Vec<(String, f64)> = conn.zrange_withscores(&gas_key, 0, -1)?;
        let gas_values: Vec<i64> = gas_entries.iter().map(|(_, s)| *s as i64).collect();

        let peak_gas = gas_values.last().copied();
        let median_gas = if gas_values.is_empty() {
            None
        } else {
            let mid = gas_values.len() / 2;
            if gas_values.len() % 2 == 1 {
                Some(gas_values[mid] as f64)
            } else {
                Some((gas_values[mid - 1] + gas_values[mid]) as f64 / 2.0)
            }
        };

        // Compilation stats from test runs
        let tr_ids: Vec<String> =
            conn.smembers(format!("test_run:by_project:{}", project_id))?;
        let mut comp_passed: i32 = 0;
        let mut comp_not_passed: i32 = 0;
        for tid_str in &tr_ids {
            let tr_key = format!("test_run:{}", tid_str);
            let p: String = conn
                .hget(&tr_key, "compilation_passed")
                .unwrap_or_default();
            let np: String = conn
                .hget(&tr_key, "compilation_not_passed")
                .unwrap_or_default();
            comp_passed += p.parse::<i32>().unwrap_or(0);
            comp_not_passed += np.parse::<i32>().unwrap_or(0);
        }

        // Trial-level aggregation
        let trial_ids: Vec<String> =
            conn.smembers(format!("synthesis_trial:by_project:{}", project_id))?;
        let total_trials = trial_ids.len() as i32;

        let mut proven: i32 = 0;
        let mut unproven: i32 = 0;
        let mut min_succeeded_iter: Option<i32> = None;

        // Read project_number_invariants for proven calculation
        let project_key = format!("project:{}", project_id);
        let inv_str: String = conn
            .hget(&project_key, "number_invariants")
            .unwrap_or_default();
        let project_number_invariants = inv_str.parse::<i32>().unwrap_or(0);

        for tid_str in &trial_ids {
            let trial_key = format!("synthesis_trial:{}", tid_str);
            let rtype: String = match conn.hget(&trial_key, "result_type") {
                Ok(v) => v,
                Err(_) => continue,
            };

            if rtype == "succeeded_full" {
                let npi_str: String = conn
                    .hget(&trial_key, "not_proved_invariants")
                    .unwrap_or_default();
                let npi = npi_str.parse::<i32>().unwrap_or(0);
                proven += project_number_invariants - npi;
            }

            if rtype.starts_with("succeeded") {
                let npi_str: String = conn
                    .hget(&trial_key, "not_proved_invariants")
                    .unwrap_or_default();
                let npi = npi_str.parse::<i32>().unwrap_or(0);
                unproven += npi;

                let iter_str: String = conn
                    .hget(&trial_key, "iteration")
                    .unwrap_or_default();
                let iter = iter_str.parse::<i32>().unwrap_or(0);
                if min_succeeded_iter.is_none() || iter < min_succeeded_iter.unwrap() {
                    min_succeeded_iter = Some(iter);
                }
            }
        }

        let metrics = Metrics {
            median_gas,
            peak_gas,
            compilation_passed: comp_passed,
            compilation_not_passed: comp_not_passed,
            total_trials,
            proven_invariants: proven,
            unproven_invariants: unproven,
            succeeded_iterations: min_succeeded_iter.unwrap_or(0),
        };
        eprintln!(
            "[DEBUG] RedisDatabase::get_metrics::ok project_id={} median_gas={:?} peak_gas={:?} comp_passed={} comp_not_passed={} total_trials={} proven={} unproven={} succeeded_at_iter={}",
            project_id,
            metrics.median_gas,
            metrics.peak_gas,
            metrics.compilation_passed,
            metrics.compilation_not_passed,
            metrics.total_trials,
            metrics.proven_invariants,
            metrics.unproven_invariants,
            metrics.succeeded_iterations
        );
        Ok(metrics)
    }
}

// ---------------------------------------------------------------------------
// SQLite implementation
// ---------------------------------------------------------------------------

pub struct SqliteDatabase {
    conn: SqliteConnection,
}

impl SqliteDatabase {
    pub fn new(path: &str) -> Result<Self, DbError> {
        eprintln!("[DEBUG] SqliteDatabase::new path=\"{}\"", path);
        let conn = SqliteConnection::open(path)?;
        let db = Self { conn };
        db.run_migrations()?;
        eprintln!("[DEBUG] SqliteDatabase::new::ok path=\"{}\"", path);
        Ok(db)
    }

    fn run_migrations(&self) -> Result<(), DbError> {
        eprintln!("[DEBUG] SqliteDatabase::run_migrations");
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS project (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                number_invariants INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS test_run (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL REFERENCES project(id),
                compilation_passed INTEGER NOT NULL DEFAULT 0,
                compilation_not_passed INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS synthesis_trial (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                test_run_id INTEGER NOT NULL REFERENCES test_run(id),
                iteration INTEGER NOT NULL,
                gas_of_implementation INTEGER,
                result_type TEXT NOT NULL CHECK(result_type IN ('failed_compilation', 'failed_fuzzing', 'succeeded_fuzzing', 'failed_halmos', 'succeeded_partial', 'succeeded_full')),
                not_proved_invariants INTEGER DEFAULT 0,
                failure_detail TEXT,
                is_full_synthesis INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            ",
        )?;

        // Migration: add is_full_synthesis column if missing
        if self
            .conn
            .prepare("SELECT is_full_synthesis FROM synthesis_trial LIMIT 0")
            .is_err()
        {
            self.conn.execute_batch(
                "ALTER TABLE synthesis_trial ADD COLUMN is_full_synthesis INTEGER NOT NULL DEFAULT 0;"
            )?;
            eprintln!("[DEBUG] SqliteDatabase::run_migrations add_column=is_full_synthesis");
        }

        // Migration: expand CHECK constraint to include 'succeeded_fuzzing'
        let check_ok = self.conn.execute(
            "INSERT INTO synthesis_trial (test_run_id, iteration, result_type, is_full_synthesis) VALUES (-999, 0, 'succeeded_fuzzing', 0)",
            [],
        );
        if check_ok.is_err() {
            eprintln!("[DEBUG] SqliteDatabase::run_migrations recreate_table=old_check_constraint_detected");
            self.conn.execute_batch(
                "ALTER TABLE synthesis_trial RENAME TO synthesis_trial_old;

                CREATE TABLE synthesis_trial (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    test_run_id INTEGER NOT NULL REFERENCES test_run(id),
                    iteration INTEGER NOT NULL,
                    gas_of_implementation INTEGER,
                    result_type TEXT NOT NULL CHECK(result_type IN ('failed_compilation', 'failed_fuzzing', 'succeeded_fuzzing', 'failed_halmos', 'succeeded_partial', 'succeeded_full')),
                    not_proved_invariants INTEGER DEFAULT 0,
                    failure_detail TEXT,
                    is_full_synthesis INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );

                INSERT INTO synthesis_trial
                    SELECT id, test_run_id, iteration, gas_of_implementation,
                           result_type, not_proved_invariants, failure_detail,
                           is_full_synthesis, created_at
                    FROM synthesis_trial_old;

                DROP TABLE synthesis_trial_old;"
            )?;
            eprintln!("[DEBUG] SqliteDatabase::run_migrations::check_constraint_expanded");
        } else {
            self.conn
                .execute("DELETE FROM synthesis_trial WHERE test_run_id = -999", [])?;
        }

        eprintln!("[DEBUG] SqliteDatabase::run_migrations::ok tables=[project,test_run,synthesis_trial]");
        Ok(())
    }
}

impl Database for SqliteDatabase {
    fn get_or_create_project(
        &self,
        name: &str,
        number_invariants: i32,
    ) -> Result<Project, DbError> {
        eprintln!(
            "[DEBUG] SqliteDatabase::get_or_create_project name=\"{}\" number_invariants={}",
            name, number_invariants
        );
        let existing = self.conn.query_row(
            "SELECT id, name, number_invariants FROM project WHERE name = ?1",
            params![name],
            |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    number_invariants: row.get(2)?,
                })
            },
        );

        match existing {
            Ok(project) => {
                eprintln!("[DEBUG] SqliteDatabase::get_or_create_project::found id={}", project.id);
                Ok(project)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                self.conn.execute(
                    "INSERT INTO project (name, number_invariants) VALUES (?1, ?2)",
                    params![name, number_invariants],
                )?;
                let id = self.conn.last_insert_rowid();
                eprintln!("[DEBUG] SqliteDatabase::get_or_create_project::created id={}", id);
                Ok(Project {
                    id,
                    name: name.to_string(),
                    number_invariants,
                })
            }
            Err(e) => {
                eprintln!("[DEBUG] SqliteDatabase::get_or_create_project::err error=\"{}\"", e);
                Err(DbError::Sqlite(e))
            }
        }
    }

    fn create_test_run(&self, project_id: i64) -> Result<TestRun, DbError> {
        eprintln!("[DEBUG] SqliteDatabase::create_test_run project_id={}", project_id);
        self.conn.execute(
            "INSERT INTO test_run (project_id) VALUES (?1)",
            params![project_id],
        )?;
        let id = self.conn.last_insert_rowid();
        eprintln!("[DEBUG] SqliteDatabase::create_test_run::ok id={}", id);
        Ok(TestRun {
            id,
            project_id,
            compilation_passed: 0,
            compilation_not_passed: 0,
        })
    }

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
    ) -> Result<SynthesisTrial, DbError> {
        eprintln!(
            "[DEBUG] SqliteDatabase::record_trial test_run_id={} iteration={} gas={:?} result_type=\"{}\" not_proved={} project_invariants={} is_full_synthesis={}",
            test_run_id,
            iteration,
            gas_of_implementation,
            result_type,
            not_proved_invariants,
            project_number_invariants,
            is_full_synthesis
        );

        validate_trial_params(result_type, not_proved_invariants, project_number_invariants)?;

        self.conn.execute(
            "INSERT INTO synthesis_trial (test_run_id, iteration, gas_of_implementation, result_type, not_proved_invariants, failure_detail, is_full_synthesis) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                test_run_id,
                iteration,
                gas_of_implementation,
                result_type,
                not_proved_invariants,
                failure_detail,
                is_full_synthesis as i32,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        eprintln!("[DEBUG] SqliteDatabase::record_trial::ok trial_id={}", id);
        Ok(SynthesisTrial {
            id,
            test_run_id,
            iteration,
            gas_of_implementation,
            result_type: result_type.to_string(),
            not_proved_invariants,
            failure_detail: failure_detail.map(|s| s.to_string()),
            is_full_synthesis,
        })
    }

    fn get_max_iteration(&self, test_run_id: i64) -> Result<i32, DbError> {
        let max: i32 = self.conn.query_row(
            "SELECT COALESCE(MAX(st.iteration), 0)
             FROM synthesis_trial st
             WHERE st.test_run_id = ?1",
            params![test_run_id],
            |row| row.get(0),
        )?;
        eprintln!(
            "[DEBUG] SqliteDatabase::get_max_iteration test_run_id={} max={}",
            test_run_id, max
        );
        Ok(max)
    }

    fn increment_compilation_passed(&self, test_run_id: i64) -> Result<(), DbError> {
        eprintln!(
            "[DEBUG] SqliteDatabase::increment_compilation_passed test_run_id={}",
            test_run_id
        );
        self.conn.execute(
            "UPDATE test_run SET compilation_passed = compilation_passed + 1 WHERE id = ?1",
            params![test_run_id],
        )?;
        Ok(())
    }

    fn increment_compilation_not_passed(&self, test_run_id: i64) -> Result<(), DbError> {
        eprintln!(
            "[DEBUG] SqliteDatabase::increment_compilation_not_passed test_run_id={}",
            test_run_id
        );
        self.conn.execute(
            "UPDATE test_run SET compilation_not_passed = compilation_not_passed + 1 WHERE id = ?1",
            params![test_run_id],
        )?;
        Ok(())
    }

    fn get_project(&self, project_id: i64) -> Result<Project, DbError> {
        eprintln!("[DEBUG] SqliteDatabase::get_project project_id={}", project_id);
        let result = self.conn.query_row(
            "SELECT id, name, number_invariants FROM project WHERE id = ?1",
            params![project_id],
            |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    number_invariants: row.get(2)?,
                })
            },
        );
        match &result {
            Ok(p) => eprintln!(
                "[DEBUG] SqliteDatabase::get_project::ok id={} name=\"{}\" invariants={}",
                p.id, p.name, p.number_invariants
            ),
            Err(e) => eprintln!("[DEBUG] SqliteDatabase::get_project::err error=\"{}\"", e),
        }
        result.map_err(DbError::Sqlite)
    }

    fn get_metrics(&self, project_id: i64) -> Result<Metrics, DbError> {
        eprintln!("[DEBUG] SqliteDatabase::get_metrics project_id={}", project_id);
        let _project = self.get_project(project_id)?;

        // Aggregate gas metrics across all trials for this project's test runs
        let peak_gas: Option<i64> = self.conn.query_row(
            "SELECT MAX(gas_of_implementation)
             FROM synthesis_trial st
             JOIN test_run tr ON st.test_run_id = tr.id
             WHERE tr.project_id = ?1 AND gas_of_implementation IS NOT NULL",
            params![project_id],
            |row| row.get(0),
        )?;

        // Median gas: fetch all non-null values, compute in Rust
        let median_gas = {
            let mut stmt = self.conn.prepare(
                "SELECT gas_of_implementation
                 FROM synthesis_trial st
                 JOIN test_run tr ON st.test_run_id = tr.id
                 WHERE tr.project_id = ?1 AND gas_of_implementation IS NOT NULL
                 ORDER BY gas_of_implementation",
            )?;
            let rows = stmt.query_map(params![project_id], |row| row.get::<_, i64>(0))?;
            let vals: Vec<i64> = rows.filter_map(|r| r.ok()).collect();
            if vals.is_empty() {
                None
            } else {
                let mid = vals.len() / 2;
                if vals.len() % 2 == 1 {
                    Some(vals[mid] as f64)
                } else {
                    Some((vals[mid - 1] + vals[mid]) as f64 / 2.0)
                }
            }
        };

        // Compilation stats
        let comp_stats = self.conn.query_row(
            "SELECT COALESCE(SUM(compilation_passed), 0), COALESCE(SUM(compilation_not_passed), 0)
             FROM test_run WHERE project_id = ?1",
            params![project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        // Total trials
        let total_trials: i32 = self.conn.query_row(
            "SELECT COUNT(*) FROM synthesis_trial st JOIN test_run tr ON st.test_run_id = tr.id WHERE tr.project_id = ?1",
            params![project_id],
            |row| row.get(0),
        )?;

        // Invariant stats — count succeeded_full vs succeeded_partial vs unproven
        let proven: i32 = self.conn.query_row(
            "SELECT COALESCE(SUM(p.number_invariants - st.not_proved_invariants), 0)
             FROM synthesis_trial st
             JOIN test_run tr ON st.test_run_id = tr.id
             JOIN project p ON tr.project_id = p.id
             WHERE tr.project_id = ?1 AND st.result_type = 'succeeded_full'",
            params![project_id],
            |row| row.get(0),
        )?;

        let unproven: i32 = self.conn.query_row(
            "SELECT COALESCE(SUM(not_proved_invariants), 0)
             FROM synthesis_trial st JOIN test_run tr ON st.test_run_id = tr.id
             WHERE tr.project_id = ?1 AND st.result_type LIKE 'succeeded%'",
            params![project_id],
            |row| row.get(0),
        )?;

        // Synthesis efficiency — iterations needed to reach a succeeded state
        let succeeded_iters: i32 = self.conn.query_row(
            "SELECT COALESCE(MIN(iteration), 0)
             FROM synthesis_trial st JOIN test_run tr ON st.test_run_id = tr.id
             WHERE tr.project_id = ?1 AND st.result_type LIKE 'succeeded%'",
            params![project_id],
            |row| row.get(0),
        )?;

        let metrics = Metrics {
            median_gas,
            peak_gas,
            compilation_passed: comp_stats.0,
            compilation_not_passed: comp_stats.1,
            total_trials,
            proven_invariants: proven,
            unproven_invariants: unproven,
            succeeded_iterations: succeeded_iters,
        };
        eprintln!(
            "[DEBUG] SqliteDatabase::get_metrics::ok project_id={} median_gas={:?} peak_gas={:?} comp_passed={} comp_not_passed={} total_trials={} proven={} unproven={} succeeded_at_iter={}",
            project_id,
            metrics.median_gas,
            metrics.peak_gas,
            metrics.compilation_passed,
            metrics.compilation_not_passed,
            metrics.total_trials,
            metrics.proven_invariants,
            metrics.unproven_invariants,
            metrics.succeeded_iterations
        );
        Ok(metrics)
    }
}

// ---------------------------------------------------------------------------
// Tests — Redis
// ---------------------------------------------------------------------------

#[cfg(test)]
mod redis_tests {
    use super::*;

    fn setup_db() -> RedisDatabase {
        let url = std::env::var("TEST_REDIS_URL")
            .unwrap_or_else(|_| "redis://localhost:6379/1".into());
        let db = RedisDatabase::new(&url).expect("Failed to connect to Redis");
        let mut conn = db.client.get_connection().expect("conn");
        let _: () = redis::cmd("FLUSHDB").query(&mut conn).expect("flushdb");
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
                tr.id, 1, None, "failed_compilation", 0, Some("err"),
                proj.number_invariants, false,
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
                tr.id, 1, Some(50000), "succeeded_full", 0, None,
                proj.number_invariants, true,
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
                tr.id, 1, Some(30000), "succeeded_partial", 2, None,
                proj.number_invariants, false,
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
                tr.id, 1, None, "succeeded_fuzzing", 0, None,
                proj.number_invariants, false,
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
        db.record_trial(
            tr.id, 1, None, "succeeded_partial", 5, None,
            proj.number_invariants, false,
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

        let tr1 = db.create_test_run(proj.id).unwrap();
        let tr2 = db.create_test_run(proj.id).unwrap();

        db.increment_compilation_passed(tr1.id).unwrap();
        db.increment_compilation_passed(tr1.id).unwrap();
        db.increment_compilation_not_passed(tr1.id).unwrap();
        db.record_trial(
            tr1.id, 1, None, "failed_compilation", 0, Some("err"), 5, false,
        )
        .unwrap();

        db.record_trial(
            tr2.id, 1, Some(100000), "succeeded_full", 0, None, 5, false,
        )
        .unwrap();
        db.record_trial(
            tr2.id, 2, Some(90000), "succeeded_partial", 2, None, 5, false,
        )
        .unwrap();

        let metrics = db.get_metrics(proj.id).unwrap();
        assert_eq!(metrics.compilation_passed, 2);
        assert_eq!(metrics.compilation_not_passed, 1);
        assert_eq!(metrics.total_trials, 3);
        assert_eq!(metrics.median_gas.unwrap() as i64, 95000);
        assert_eq!(metrics.peak_gas.unwrap(), 100000);
        assert_eq!(metrics.proven_invariants, 5);
        assert_eq!(metrics.unproven_invariants, 2);
        assert_eq!(metrics.succeeded_iterations, 1);
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

        db.record_trial(
            tr1.id, 1, None, "failed_compilation", 0, Some("err"), 3, false,
        )
        .unwrap();
        db.record_trial(
            tr1.id, 2, Some(50000), "succeeded_full", 0, None, 3, false,
        )
        .unwrap();

        db.record_trial(
            tr2.id, 5, None, "failed_fuzzing", 0, Some("fail"), 3, false,
        )
        .unwrap();

        assert_eq!(db.get_max_iteration(tr1.id).unwrap(), 2);
        assert_eq!(db.get_max_iteration(tr2.id).unwrap(), 5);
    }

    #[test]
    fn test_result_type_check_constraint() {
        let db = setup_db();
        let proj = db.get_or_create_project("p", 1).unwrap();
        let tr = db.create_test_run(proj.id).unwrap();
        let result = db.record_trial(
            tr.id, 1, None, "invalid_type", 0, None, 1, false,
        );
        assert!(result.is_err());
    }
}

// ---------------------------------------------------------------------------
// Tests — SQLite
// ---------------------------------------------------------------------------

#[cfg(test)]
mod sqlite_tests {
    use super::*;

    fn setup_db() -> SqliteDatabase {
        SqliteDatabase::new(":memory:").expect("Failed to create in-memory DB")
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
                tr.id, 1, None, "failed_compilation", 0, Some("err"),
                proj.number_invariants, false,
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
                tr.id, 1, Some(50000), "succeeded_full", 0, None,
                proj.number_invariants, true,
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
                tr.id, 1, Some(30000), "succeeded_partial", 2, None,
                proj.number_invariants, false,
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
                tr.id, 1, None, "succeeded_fuzzing", 0, None,
                proj.number_invariants, false,
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
        db.record_trial(
            tr.id, 1, None, "succeeded_partial", 5, None,
            proj.number_invariants, false,
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

        let tr1 = db.create_test_run(proj.id).unwrap();
        let tr2 = db.create_test_run(proj.id).unwrap();

        db.increment_compilation_passed(tr1.id).unwrap();
        db.increment_compilation_passed(tr1.id).unwrap();
        db.increment_compilation_not_passed(tr1.id).unwrap();
        db.record_trial(
            tr1.id, 1, None, "failed_compilation", 0, Some("err"), 5, false,
        )
        .unwrap();

        db.record_trial(
            tr2.id, 1, Some(100000), "succeeded_full", 0, None, 5, false,
        )
        .unwrap();
        db.record_trial(
            tr2.id, 2, Some(90000), "succeeded_partial", 2, None, 5, false,
        )
        .unwrap();

        let metrics = db.get_metrics(proj.id).unwrap();
        assert_eq!(metrics.compilation_passed, 2);
        assert_eq!(metrics.compilation_not_passed, 1);
        assert_eq!(metrics.total_trials, 3);
        assert_eq!(metrics.median_gas.unwrap() as i64, 95000);
        assert_eq!(metrics.peak_gas.unwrap(), 100000);
        assert_eq!(metrics.proven_invariants, 5);
        assert_eq!(metrics.unproven_invariants, 2);
        assert_eq!(metrics.succeeded_iterations, 1);
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

        db.record_trial(
            tr1.id, 1, None, "failed_compilation", 0, Some("err"), 3, false,
        )
        .unwrap();
        db.record_trial(
            tr1.id, 2, Some(50000), "succeeded_full", 0, None, 3, false,
        )
        .unwrap();

        db.record_trial(
            tr2.id, 5, None, "failed_fuzzing", 0, Some("fail"), 3, false,
        )
        .unwrap();

        assert_eq!(db.get_max_iteration(tr1.id).unwrap(), 2);
        assert_eq!(db.get_max_iteration(tr2.id).unwrap(), 5);
    }

    #[test]
    fn test_result_type_check_constraint() {
        let db = setup_db();
        let proj = db.get_or_create_project("p", 1).unwrap();
        let tr = db.create_test_run(proj.id).unwrap();
        let result = db.record_trial(
            tr.id, 1, None, "invalid_type", 0, None, 1, false,
        );
        assert!(result.is_err());
    }
}
