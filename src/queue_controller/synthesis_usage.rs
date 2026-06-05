//! Parse Claude Code stream-json output for model usage metrics and write to
//! the synthesis `test_run` record in Redis.
//!
//! Called as a post-processing step after successful git push.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

/// Aggregated model-usage counters for one synthesis run.
#[derive(Debug, Default, Clone, Copy)]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cost_usd: f64,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Read the last line of a stream-json output file and extract usage metrics.
///
/// The last non-empty line is the end-of-stream summary with camelCase totals:
/// ```json
/// {"inputTokens":49800,"outputTokens":3939,"cacheReadInputTokens":568458,"costUSD":0.63}
/// ```
/// Returns all-zeros if absent or unparseable.
pub fn parse_output_file(path: &Path) -> Result<UsageTotals> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("failed to read {path:?}"))?;

    let Some(line) = content.lines().rev().find(|l| !l.trim().is_empty()) else {
        return Ok(UsageTotals::default());
    };
    let line = line.trim();

    let Ok(val) = serde_json::from_str::<Value>(line) else {
        return Ok(UsageTotals::default());
    };

    Ok(try_parse_summary_usage(&val).unwrap_or_default())
}

/// Extract camelCase totals from a JSON object.
///
/// Checks root first, then nested under a single key (e.g. `{"result":{...}}`).
fn try_parse_summary_usage(val: &Value) -> Option<UsageTotals> {
    extract_camel_case_totals(val).or_else(|| {
        val.as_object()?
            .values()
            .find_map(extract_camel_case_totals)
    })
}

fn extract_camel_case_totals(val: &Value) -> Option<UsageTotals> {
    let obj = val.as_object()?;
    Some(UsageTotals {
        input_tokens: obj.get("inputTokens")?.as_u64()?,
        output_tokens: obj.get("outputTokens")?.as_u64()?,
        cache_read_input_tokens: obj.get("cacheReadInputTokens")?.as_u64()?,
        cost_usd: obj.get("costUSD")?.as_f64()?,
    })
}

// ---------------------------------------------------------------------------
// Redis persistence
// ---------------------------------------------------------------------------

/// Find the latest `test_run` for `project_name` and write usage totals to
/// its hash (`test_run:{id}`).
pub fn write_usage_to_test_run(
    conn: &mut redis::Connection,
    project_name: &str,
    totals: &UsageTotals,
) -> Result<()> {
    let project_id: Option<String> = redis::cmd("GET")
        .arg(format!("project:name:{project_name}"))
        .query(conn)
        .context("failed to look up project by name")?;
    let project_id = project_id.context("project not found")?;

    let ids: Vec<String> = redis::cmd("SMEMBERS")
        .arg(format!("test_run:by_project:{project_id}"))
        .query(conn)?;
    let max_id = ids
        .iter()
        .filter_map(|id| id.parse::<i64>().ok())
        .max()
        .context("no test runs found for project")?;

    let key = format!("test_run:{max_id}");
    redis::pipe()
        .cmd("HSET")
        .arg(&key)
        .arg("totalInputTokens")
        .arg(totals.input_tokens)
        .ignore()
        .cmd("HSET")
        .arg(&key)
        .arg("totalOutputTokens")
        .arg(totals.output_tokens)
        .ignore()
        .cmd("HSET")
        .arg(&key)
        .arg("cost_of_synthesis_USD")
        .arg(totals.cost_usd)
        .ignore()
        .query::<()>(conn)
        .with_context(|| format!("failed to HSET usage on {key}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_summary_top_level() {
        let line = r#"{"inputTokens":49800,"outputTokens":3939,"cacheReadInputTokens":568458,"costUSD":0.63}"#;
        let v: Value = serde_json::from_str(line).unwrap();
        let t = try_parse_summary_usage(&v).unwrap();
        assert_eq!(
            (t.input_tokens, t.output_tokens, t.cache_read_input_tokens),
            (49800, 3939, 568458)
        );
        assert!((t.cost_usd - 0.63).abs() < 1e-4);
    }

    #[test]
    fn test_parse_summary_nested() {
        let line = r#"{"result":{"inputTokens":5,"outputTokens":4,"cacheReadInputTokens":3,"costUSD":0.02}}"#;
        let v: Value = serde_json::from_str(line).unwrap();
        let t = try_parse_summary_usage(&v).unwrap();
        assert_eq!((t.input_tokens, t.output_tokens), (5, 4));
    }

    #[test]
    fn test_parse_summary_partial() {
        let v: Value = serde_json::from_str(r#"{"inputTokens":10,"outputTokens":5}"#).unwrap();
        assert!(try_parse_summary_usage(&v).is_none());
    }

    #[test]
    fn test_parse_file_last_line() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp.as_file(), r#"{{"type":"ping"}}"#).unwrap();
        writeln!(
            tmp.as_file(),
            r#"{{"inputTokens":999,"outputTokens":88,"cacheReadInputTokens":50,"costUSD":1.23}}"#
        )
        .unwrap();
        tmp.flush().unwrap();
        let t = parse_output_file(tmp.path()).unwrap();
        assert_eq!(
            (t.input_tokens, t.output_tokens, t.cache_read_input_tokens),
            (999, 88, 50)
        );
        assert!((t.cost_usd - 1.23).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_file_empty() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let t = parse_output_file(tmp.path()).unwrap();
        assert_eq!(t.input_tokens, 0);
    }

    #[test]
    fn test_parse_file_no_summary() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp.as_file(), r#"{{"type":"ping"}}"#).unwrap();
        tmp.flush().unwrap();
        let t = parse_output_file(tmp.path()).unwrap();
        assert_eq!(t.input_tokens, 0);
    }

    fn test_redis_conn() -> Option<redis::Connection> {
        let url =
            std::env::var("TEST_REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379/1".into());
        let client = redis::Client::open(url.as_str()).ok()?;
        let mut conn = client.get_connection().ok()?;
        let _: () = redis::cmd("FLUSHDB").query(&mut conn).ok()?;
        Some(conn)
    }

    fn seed(conn: &mut redis::Connection, name: &str) -> (i64, i64) {
        let pid: i64 = redis::cmd("INCR").arg("project:ids").query(conn).unwrap();
        redis::cmd("SET")
            .arg(format!("project:name:{name}"))
            .arg(pid)
            .query::<()>(conn)
            .unwrap();
        redis::cmd("HSET")
            .arg(format!("project:{pid}"))
            .arg("name")
            .arg(name)
            .arg("number_invariants")
            .arg(5)
            .arg("created_at")
            .arg("now")
            .query::<()>(conn)
            .unwrap();
        let trid: i64 = redis::cmd("INCR").arg("test_run:ids").query(conn).unwrap();
        redis::cmd("HSET")
            .arg(format!("test_run:{trid}"))
            .arg("project_id")
            .arg(pid)
            .arg("compilation_passed")
            .arg(0)
            .arg("compilation_not_passed")
            .arg(0)
            .arg("created_at")
            .arg("now")
            .query::<()>(conn)
            .unwrap();
        redis::cmd("SADD")
            .arg(format!("test_run:by_project:{pid}"))
            .arg(trid)
            .query::<()>(conn)
            .unwrap();
        (pid, trid)
    }

    #[test]
    fn test_write_usage() {
        let Some(mut conn) = test_redis_conn() else {
            return;
        };
        let (_, trid) = seed(&mut conn, "p");
        let totals = UsageTotals {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_input_tokens: 30,
            cost_usd: 0.25,
        };
        write_usage_to_test_run(&mut conn, "p", &totals).unwrap();
        let fields: std::collections::HashMap<String, String> = redis::cmd("HGETALL")
            .arg(format!("test_run:{trid}"))
            .query(&mut conn)
            .unwrap();
        assert_eq!(fields.get("totalInputTokens").unwrap(), "100");
        assert_eq!(fields.get("cost_of_synthesis_USD").unwrap(), "0.25");
    }

    #[test]
    fn test_write_usage_not_found() {
        let Some(mut conn) = test_redis_conn() else {
            return;
        };
        let result = write_usage_to_test_run(&mut conn, "nonexistent", &UsageTotals::default());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("project not found")
        );
    }
}
