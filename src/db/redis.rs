use chrono::Utc;
use ::redis::Commands;

use crate::db::{Database, DbError, Metrics, Project, SynthesisTrial, TestRun, validate_trial_params};

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
                    number_invariants.to_string(),
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
        let _: bool = conn.hset(&key, "project_id", project_id.to_string())?;
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

        let _: bool = conn.hset(&trial_key, "test_run_id", test_run_id.to_string())?;
        let _: bool = conn.hset(&trial_key, "iteration", iteration.to_string())?;
        let _: bool = conn.hset(&trial_key, "result_type", result_type)?;
        let _: bool = conn.hset(
            &trial_key,
            "not_proved_invariants",
            not_proved_invariants.to_string(),
        )?;
        let _: bool = conn.hset(
            &trial_key,
            "is_full_synthesis",
            (is_full_synthesis as i32).to_string(),
        )?;
        let _: bool = conn.hset(&trial_key, "created_at", &now)?;
        if let Some(gas) = gas_of_implementation {
            let _: bool = conn.hset(&trial_key, "gas_of_implementation", gas.to_string())?;
        }
        if let Some(detail) = failure_detail {
            let _: bool = conn.hset(&trial_key, "failure_detail", detail)?;
        }

        let _: i64 = conn.zadd(
            format!("synthesis_trial:by_test_run:{}", test_run_id),
            id,
            iteration as f64,
        )?;

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

        let trial_ids: Vec<String> =
            conn.smembers(format!("synthesis_trial:by_project:{}", project_id))?;
        let total_trials = trial_ids.len() as i32;

        let mut proven: i32 = 0;
        let mut unproven: i32 = 0;
        let mut min_succeeded_iter: Option<i32> = None;

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
