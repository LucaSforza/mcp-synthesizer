# Specification: MCP Server (`mcp_synth`)

## Overview

MCP server for Solidity smart contract synthesis. Integrates Foundry toolchain (`forge`, `halmos`) into Claude Code via MCP protocol. Uses rmcp SDK over stdio transport.

Persists synthesis trials (compilation, fuzzing, halmos verification) to Redis via a `Database` trait abstraction.

**Binary name:** `mcp_synth`
**Source:** `src/bin/mcp_synth.rs` + `src/synth/{mod,tools,pipeline,db}.rs`
**Package:** `mcp_synth` (Cargo.toml)

---

## Architecture

```
┌──────────────┐   stdio    ┌────────────────────────────────┐
│  Claude Code │◄─────────►│         mcp_synth              │
│  (MCP host)  │   JSON-RPC │                                │
└──────────────┘           │  ┌──────────┐  ┌────────────┐  │
                            │  │  Tools   │  │  Pipeline  │  │
                            │  │  (4 MCP  │──►  build →   │  │
                            │  │  tools)  │  │  test →    │  │
                            │  └────┬─────┘  │  halmos    │  │
                            │       │        └──────┬─────┘  │
                            │       │               │        │
                            │  ┌────▼───────────────▼─────┐  │
                            │  │     Database trait       │  │
                            │  │  Box<dyn Database>       │  │
                            │  └────┬──────────────┬──────┘  │
                            └───────┼──────────────┼─────────┘
                                    │              │
                           ┌────────▼──┐           │
                           │   Redis   │           │
                           │ (default) │           │
                           └───────────┘           │
                                    └──────────────┘
                                    (Database trait)
```

Four MCP tools. `forge_install`, `forge_build`, `forge_test` are individual commands. `run_synthesis` is the full pipeline orchestration.

---

## CLI Interface

```
mcp_synth --cwd /path/to/foundry-project --project my-contracts \
    [--invariants 5] \
    [--redis-url redis://localhost:6379]
```

### Arguments

| Flag | Required | Default | Description |
|---|---|---|---|
| `--cwd` / `-c` | yes | — | Path to Foundry project directory |
| `--project` / `-p` | yes | — | Project name identifier |
| `--invariants` / `-i` | no | `0` | Number of invariants for halmos verification |
| `--redis-url` / `-u` | no | `redis://localhost:6379` | Redis server URL |

### Startup flow

1. Parse args, build `DbConfig::Redis { url }`
2. Connect to database → `Box<dyn Database>`
3. `get_or_create_project(name, invariants)` → `Project { id, name, number_invariants }`
4. `create_test_run(project_id)` → `TestRun { id, ... }`
5. Serve tools over stdio via rmcp (`service.waiting().await`)

---

## MCP Tools

All four return `Result<String, String>`. Ok/Err carries formatted text.

### `forge_install`

Install Foundry project dependencies.

- **Command:** `forge install` (in `--cwd`)
- **Idempotent:** yes
- **DB recorded:** nothing
- **Returns:** combined stdout+stderr, or error with output on failure

### `forge_build`

Compile with `forge build -vvv`.

- **Command:** `forge build -vvv` (in `--cwd`)
- **Idempotent:** yes
- **DB recorded:** `increment_compilation_passed()` on success, `increment_compilation_not_passed()` on failure
- **Returns:** `"Build passed.\n{output}"` or `"Build failed.\n{output}"`

### `forge_test`

Run unit + fuzz tests with JSON output.

- **Command:** `forge test --json` (in `--cwd`)
- **Idempotent:** yes
- **DB recorded:** trial with `result_type = "succeeded_fuzzing"` or `"failed_fuzzing"`, `is_full_synthesis = false`. Extracts gas from JSON and stores `gas_of_implementation`.
- **Gas extraction:** Parses `forge test --json` output. For each test suite, sums `kind.Unit.gas` and `kind.Fuzz.mean_gas` across all test results. Returns `Option<i64>`.
- **Returns:** `"Tests passed.\n{output}"` or `"Tests failed.\n{output}"`

### `run_synthesis`

Full pipeline: build → test → halmos. Records every trial in DB.

- **Idempotent:** no
- **Pipeline:** lazily initialized on first call. Uses fresh DB connection (calls `DbConfig::connect()` internally).
- **Returns:** formatted report with stage-by-stage outcome, metrics (if halmos reached).
- **Recording:** all trials have `is_full_synthesis = true`

See "Synthesis Pipeline" section for detailed phase outcomes.

---

## Synthesis Pipeline

Three phases, sequential. Short-circuits on failure.

### Phase A: Build

```
forge build -vvv
```

| Exit code | Action | result_type |
|---|---|---|
| 0 | `increment_compilation_passed()`, proceed to test | — |
| non-zero | `increment_compilation_not_passed()`, record trial, return report | `failed_compilation` |

### Phase A: Test

```
forge test --json
```

| Exit code | Action | result_type |
|---|---|---|
| 0 | Extract gas from JSON, proceed to halmos | — |
| non-zero | Record trial, return report | `failed_fuzzing` |

If tests pass, `forge_gas` is cached for halmos phase.

### Phase B: Halmos

```
halmos \
    --solver-threads 16 \
    --early-exit \
    --print-full-model \
    --solver-timeout-branching 1s \
    --solver-timeout-assertion 1s
```

| Condition | Action | result_type | report.passed |
|---|---|---|---|
| Exit 0 (all proven) | Record trial, fetch metrics | `succeeded_full` | true |
| Counterexample found (output contains "counterexample"/"violated") | Record trial | `failed_halmos` | false |
| Timeout / partial proof (no counterexample) | Parse unproved count, record trial | `succeeded_partial` | true |
| IO error | Record trial | `failed_halmos` | false |

**Unproved invariant parsing:** Scans halmos output for lines containing `"unproved"`, `"unproven"`, or `"not proved"`. Extracts first integer. Falls back to `project_number_invariants`.

**Partial proof acceptance:** `succeeded_partial` is treated as success. This means timeouts don't block the pipeline — halmos partial model checking results are accepted.

### Report format

```
=== Synthesis Pipeline Report ===
Project: {name}
Iteration: {n}
Stage: {build|test|halmos}
Passed: {true|false|
{raw output}

Metrics (if halmos reached):
  Median gas: {value}
  Peak gas: {value}
  Compilation passed: {n}
  Compilation not passed: {n}
  Total trials: {n}
  Proven invariants: {n}
  Unproven invariants: {n}
  Succeeded at iteration: {n}
```

---

## Database Layer

### Trait (`Database`)

Seven methods, all `&self`, returning `Result<..., DbError>`:

| Method | Purpose |
|---|---|
| `get_or_create_project(name, invariants)` | Lookup by name or create new project |
| `create_test_run(project_id)` | Create new test run for project |
| `record_trial(test_run_id, iteration, gas, result_type, not_proved, failure_detail, project_invariants, is_full_synthesis)` | Record synthesis trial outcome |
| `get_max_iteration(test_run_id)` | Get highest iteration number for test run |
| `increment_compilation_passed(test_run_id)` | Increment passed build counter |
| `increment_compilation_not_passed(test_run_id)` | Increment failed build counter |
| `get_project(project_id)` | Get project by ID |
| `get_metrics(project_id)` | Aggregate metrics across all test runs and trials for a project |

### Data Structures

**Project:**
- `id: i64`, `name: String`, `number_invariants: i32`

**TestRun:**
- `id: i64`, `project_id: i64`, `compilation_passed: i32`, `compilation_not_passed: i32`

**SynthesisTrial:**
- `id: i64`, `test_run_id: i64`, `iteration: i32`, `gas_of_implementation: Option<i64>`, `result_type: String`, `not_proved_invariants: i32`, `failure_detail: Option<String>`, `is_full_synthesis: bool`

**Metrics (aggregated):**
- `median_gas: Option<f64>`, `peak_gas: Option<i64>`, `compilation_passed: i32`, `compilation_not_passed: i32`, `total_trials: i32`, `proven_invariants: i32`, `unproven_invariants: i32`, `succeeded_iterations: i32`

### Valid Result Types

Six values, validated at the Rust level (`validate_trial_params`).

```
failed_compilation
failed_fuzzing
succeeded_fuzzing
failed_halmos
succeeded_partial
succeeded_full
```

**Constraints:** `succeeded_*` must be the last trial in a test run (convention, not enforced). `not_proved_invariants <= number_invariants` (assert at runtime).

### DbConfig Factory

```rust
enum DbConfig {
    Redis { url: String },
}
```

`DbConfig::connect()` returns `Box<dyn Database>`.

### Error Type

`DbError` with variants: `Redis(RedisError)`, `InvalidResultType(String)`. Implements `Display`, `std::error::Error`, `From` for inner error types.

---

## Redis Data Model

Full key reference in [RedisDataModel.md](RedisDataModel.md). `mcp_synth` writes all project, test_run, and trial keys; reads them for metrics aggregation.

Metrics aggregation (computed in Rust from Redis indices, no server-side aggregation):

- **Gas:** iterate `synthesis_trial:gas:by_project` sorted set, compute median/peak
- **Compilation:** iterate `test_run:by_project`, sum per-test-run hashes
- **Trial count:** cardinality of `synthesis_trial:by_project`
- **Proven/unproven:** iterate trial hashes, filter by `result_type`, apply formula
- **Succeeded iteration:** minimum iteration among succeeded trials

---

## Testing

### Test structure

Two test files, all using `#[cfg(test)]`:

| File | Tests | Backend | Isolation |
|---|---|---|---|
| `src/db/redis_test.rs` | 15 | Redis DB 1 | `FLUSHDB` per module |
| `src/pipeline_test.rs` | 12 | Mocked commands | In-process mocking |

### Test patterns

**Database tests:** `setup_db()` → call trait method → unwrap → assert on fields.

**Pipeline tests:** `#[cfg(test)] mock_commands: Option<Vec<Result<(String, bool), String>>>` field on `SynthesisPipeline`. Each entry simulates stdout and exit status. Run command pops from front of vec. No real forge/halmos ever executed.

### Running tests

```bash
just redis-up                     # start Redis
TEST_REDIS_URL=redis://localhost:6379/1 cargo test -- --test-threads 1
```

Redis tests use DB `1` to avoid touching production data in DB `0`. `--test-threads 1` prevents concurrent FLUSHDB interference.

---

## Docker Compose (Redis)

```yaml
services:
  redis:
    image: redis:7-alpine
    container_name: mcp-synth-redis
    command: ["redis-server", "--save", "300 1", "--save", "60 10"]
    ports: ["6379:6379"]
    volumes: [redis-data:/data]
    restart: unless-stopped
```

RDB snapshots at 300s/1key or 60s/10keys. Named volume, auto-restart.

---

## Companion Binary: `migrate_sqlite_to_redis`

Legacy migration tool. Standalone binary to migrate existing SQLite data to Redis.

```
cargo run --bin migrate -- --sqlite-path <path> --redis-url redis://localhost:6379
```

Three-phase: projects → test_runs → synthesis_trials. Writes Redis key schema identical to what `RedisDatabase` produces. Sets counter keys (`project:ids`, etc.) to max IDs. Useful for switching from SQLite to Redis backend.

Designed as one-shot data migration, not incremental sync.

---

## Error Handling

Fail-fast in main: any `DbError`, tool failure, or pipeline error returns `Err` which propagates to Claude Code as the MCP tool result.

Errors are descriptive strings returned as `Result<String, String>`. Claude Code sees the failure message and can decide how to respond.

Invalid result types are caught at the Rust level by `validate_trial_params()` before any DB write.

Missing configuration (e.g., `forge` not on `PATH`, `halmos` not installed) surfaces as `std::process::Command` errors at tool invocation time.

---

## Non-Goals

- Parallel synthesis execution (single-threaded server)
- Multiple projects per server instance (one project per process)
- Incremental sync between backends
- Halmos proof replay or counterexample minimization
- Forge source-level debugging
- Distributed coordination across multiple MCP servers
- Dynamic reconfiguration (flags set at startup only)
- Web/HTTP transport (stdio only)

---

## Dependencies

| Crate | Purpose | Notable features |
|---|---|---|
| `rmcp` (git) | MCP SDK | Stdio transport, tool macros |
| `tokio` | Async runtime | Full |
| `clap` | CLI parsing | Derive macros |
| `redis` | Redis client | `tokio-comp` feature, pure Rust |
| `serde` / `serde_json` | Forge JSON parsing | Derive |
| `chrono` | Timestamps | ISO 8601 formatting |
| `tempfile` | Test temp DBs | dev-dependency only |
