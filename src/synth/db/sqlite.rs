use rusqlite::{Connection as SqliteConnection, params};

use crate::synth::db::{
    Database, DbError, Metrics, Project, SynthesisTrial, TestRun, validate_trial_params,
};

#[deprecated(note = "SQLite backend is deprecated. Use --db-type redis instead.")]
pub struct SqliteDatabase {
    conn: SqliteConnection,
}

#[allow(deprecated)]
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
            eprintln!(
                "[DEBUG] SqliteDatabase::run_migrations recreate_table=old_check_constraint_detected"
            );
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

        eprintln!(
            "[DEBUG] SqliteDatabase::run_migrations::ok tables=[project,test_run,synthesis_trial]"
        );
        Ok(())
    }
}

#[allow(deprecated)]
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
                eprintln!(
                    "[DEBUG] SqliteDatabase::get_or_create_project::found id={}",
                    project.id
                );
                Ok(project)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                self.conn.execute(
                    "INSERT INTO project (name, number_invariants) VALUES (?1, ?2)",
                    params![name, number_invariants],
                )?;
                let id = self.conn.last_insert_rowid();
                eprintln!(
                    "[DEBUG] SqliteDatabase::get_or_create_project::created id={}",
                    id
                );
                Ok(Project {
                    id,
                    name: name.to_string(),
                    number_invariants,
                })
            }
            Err(e) => {
                eprintln!(
                    "[DEBUG] SqliteDatabase::get_or_create_project::err error=\"{}\"",
                    e
                );
                Err(DbError::Sqlite(e))
            }
        }
    }

    fn create_test_run(&self, project_id: i64) -> Result<TestRun, DbError> {
        eprintln!(
            "[DEBUG] SqliteDatabase::create_test_run project_id={}",
            project_id
        );
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

        validate_trial_params(
            result_type,
            not_proved_invariants,
            project_number_invariants,
        )?;

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
        eprintln!(
            "[DEBUG] SqliteDatabase::get_project project_id={}",
            project_id
        );
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
        eprintln!(
            "[DEBUG] SqliteDatabase::get_metrics project_id={}",
            project_id
        );
        let _project = self.get_project(project_id)?;

        let peak_gas: Option<i64> = self.conn.query_row(
            "SELECT MAX(gas_of_implementation)
             FROM synthesis_trial st
             JOIN test_run tr ON st.test_run_id = tr.id
             WHERE tr.project_id = ?1 AND gas_of_implementation IS NOT NULL",
            params![project_id],
            |row| row.get(0),
        )?;

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

        let comp_stats = self.conn.query_row(
            "SELECT COALESCE(SUM(compilation_passed), 0), COALESCE(SUM(compilation_not_passed), 0)
             FROM test_run WHERE project_id = ?1",
            params![project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        let total_trials: i32 = self.conn.query_row(
            "SELECT COUNT(*) FROM synthesis_trial st JOIN test_run tr ON st.test_run_id = tr.id WHERE tr.project_id = ?1",
            params![project_id],
            |row| row.get(0),
        )?;

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
