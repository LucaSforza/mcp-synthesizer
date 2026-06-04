# CLAUDE.md

MCP server per Solidity smart contract synthesis. Parte del progetto `git_diff_checker` — protected LLM agent environment per Foundry development.

## Parent Project Context

This repo lives inside `git_diff_checker` as git submodule (`git@github.com:LucaSforza/mcp-synthesizer.git`). The parent project protects "Golden Commit" lines from LLM agent modifications via:

- **PreToolUse hook** — directory whitelist (only `src/`), blocks `forge` commands directly (must use this MCP server)
- **PostToolUse hook** — selective revert of original lines after each tool call
- **Stop hook** — blocks session stop unless `coverage.info` exists

This MCP server is the **authorized path** for forge operations and synthesis. When parent stop hook blocks due to missing coverage, it tells agent to use `run_synthesis` here.

## Database Backends

Two backends via `Database` trait. Selected at runtime via `--db-type` flag. Both compiled unconditionally.

| Backend | CLI flag | Connection param |
|---------|----------|------------------|
| Redis (default) | `--db-type redis` | `--redis-url redis://localhost:6379` |
| SQLite | `--db-type sqlite` | `--db-path <path>` |

### Redis Databases

| DB | Usage | Notes |
|----|-------|-------|
| 0  | Real runs | `--redis-url redis://localhost:6379` (default) |
| 1  | Tests only | `TEST_REDIS_URL=redis://localhost:6379/1` (default in test code) |

Tests use `FLUSHDB` on DB 1 per module — never touch DB 0 real data.

### SQLite Storage

Default path: `$HOME/Documents/solidity-synthesis.db`. SQLite backend has schema migrations in `SqliteDatabase::run_migrations()` — handles CREATE TABLE and CHECK constraint expansion for `succeeded_fuzzing`.

### Persistence

Redis saves data to `/data/dump.rdb` inside container. `docker-compose.yml` maps `redis-data` volume there. Container has `restart: unless-stopped`.

```bash
docker compose up -d   # start + auto-restart on boot
```

## Build & Run

```bash
cargo build                       # both backends always compiled
cargo build --release
cargo run -- --cwd /path/to/foundry-project --project my-contracts
cargo run -- --cwd . --project test --invariants 5
cargo run -- --cwd . --project test --db-type sqlite
```

**Flags:** `--cwd` (required), `--project` (required), `--invariants` (default 0),
`--db-type` (default `redis`), `--redis-url` (default `redis://localhost:6379`),
`--db-path` (used with `--db-type sqlite`)

**Package:** `mcp_synth` (Cargo.toml name). Three binaries:

| Binary | Path | Purpose |
|--------|------|---------|
| `mcp_synth` | `src/main.rs` | MCP server for Solidity synthesis |
| `migrate` | `src/bin/migrate_sqlite_to_redis.rs` | SQLite-to-Redis data migration |
| `queue_controller` | `src/bin/queue_controller.rs` | Automated Slurm synthesis executor |
| `populate_queue` | `src/bin/populate_queue.rs` | Batch enqueue synthesis jobs into Redis |

**MCP Inspector:** `npx @modelcontextprotocol/inspector --transport stdio -- cargo run -- --cwd /tmp --project test`

**Runtime deps:** Redis server (for Redis backend), `forge` and `halmos` on PATH. `halmos` requires python venv. Start Redis via `docker compose up -d` or `just redis-up`.

**Musl target:** `Cargo.toml` configures `x86_64-unknown-linux-musl` for fully static binary.

## Build & Install (justfile)

```bash
just build            # or: just b  — builds with LLD if available
just install          # or: just i  — copies mcp_synth to ~/.local/bin/
just queue-controller # build queue_controller binary only
just populate-queue   # build populate_queue binary only
just redis-up         # start Redis via docker compose
just test             # redis-up + cargo test -- --test-threads 1 + redis-down
```

`just install` uses md5sum to skip copy if binary unchanged.

## Run

```bash
docker compose up -d   # start Redis
cargo run -- --cwd /path/to/foundry-project --project my-contracts
cargo run -- --cwd . --project test --invariants 5 --redis-url redis://localhost:6379
cargo run -- --cwd . --project test --db-type sqlite
```

## Tests

Tests run on Redis DB `1` (never touches DB `0` real data). Uses `FLUSHDB` per module for isolation. SQLite tests use `:memory:` — naturally isolated. Run with `--test-threads 1`:

```bash
docker compose up -d
TEST_REDIS_URL=redis://localhost:6379/1 cargo test -- --test-threads 1

# Solo SQLite
cargo test sqlite_tests -- --test-threads 1

# Solo Redis
TEST_REDIS_URL=redis://localhost:6379/1 cargo test redis_tests -- --test-threads 1

## Architecture

Single-threaded MCP server via stdio transport (`rmcp` SDK from git):

### `src/main.rs` — CLI entry point
Parses `--cwd`, `--project`, `--invariants`, `--db-type`, `--redis-url`, `--db-path` with clap. Builds `DbConfig` from args, calls `DbConfig::connect()` to get `Box<dyn Database>`. Creates/loads project, creates test run. Serves `SynthesisTools` over rmcp stdio transport. Debug logging via `eprintln!` (`[DEBUG]` prefix).

### `src/db/` — Database trait, Redis + SQLite implementations

**`src/db/mod.rs`** — Shared definitions:
- Data structs: `Project`, `TestRun`, `SynthesisTrial`, `Metrics`
- `DbError` enum — `Redis(::redis::RedisError)`, `Sqlite(rusqlite::Error)`, `InvalidResultType(String)`. Implements `Display` + `std::error::Error` + `From` for both error types.
- `Database` trait — 10 methods, all `&self`, bound `Send`:
  `get_or_create_project`, `create_test_run`, `record_trial`, `get_max_iteration`,
  `increment_compilation_passed`, `increment_compilation_not_passed`, `get_project`, `get_metrics`
- `DbConfig` factory enum — `Redis { url }` + `Sqlite { path }`. `connect() -> Result<Box<dyn Database>, DbError>`.
- `validate_trial_params()` — shared validation for result_type + invariant constraints

**`src/db/redis.rs`** — `RedisDatabase` struct + `impl Database`. Redis key schema:

```
project:ids                                         -> INCR counter
project:{id}                                        -> Hash { name, number_invariants, created_at }
project:name:{name}                                 -> String (id, for uniqueness check)
test_run:ids                                        -> INCR counter
test_run:{id}                                       -> Hash { project_id, compilation_passed, compilation_not_passed, created_at }
test_run:by_project:{project_id}                    -> Set of test_run_ids
synthesis_trial:ids                                 -> INCR counter
synthesis_trial:{id}                                -> Hash { test_run_id, iteration, gas_of_implementation, result_type, not_proved_invariants, failure_detail, is_full_synthesis, created_at }
synthesis_trial:by_test_run:{test_run_id}           -> Sorted Set (member=trial_id, score=iteration)
synthesis_trial:by_project:{project_id}             -> Set of trial_ids
synthesis_trial:gas:by_project:{project_id}         -> Sorted Set (member=trial_id, score=gas)
```

**`src/db/sqlite.rs`** — `SqliteDatabase` struct + `impl Database`. Same schema in SQL tables. `run_migrations()` handles CREATE TABLE + CHECK constraint expansion for `succeeded_fuzzing`.

**Note:** `get_max_iteration` scoped per `test_run_id` (not per project). Multiple test runs in same project each track their own iteration counter.

**Trial result types** (6 values, validated in Rust):
`failed_compilation` | `failed_fuzzing` | `succeeded_fuzzing` | `failed_halmos` | `succeeded_partial` | `succeeded_full`

Constraints: succeeded_* must be last trial in test_run; `not_proved_invariants <= number_invariants`.

**`is_full_synthesis`** flag distinguishes standalone `forge_test` calls from full pipeline runs.

**No Redis migrations needed** — schema-less. `created_at` stored as ISO 8601 string.

**Metrics:** `get_metrics()` aggregates in Rust from Redis indices (no SQL GROUP BY) or via SQL queries for SQLite.

**Tests:**
- `src/db/redis_test.rs` — 15 tests, `FLUSHDB` on DB 1 per module
- `src/db/sqlite_test.rs` — 15 tests, `:memory:` per module

### `src/tools.rs` — MCP tool definitions
4 tools via `rmcp` `#[tool]` + `#[tool_router]` macros:

| Tool | Description | Idempotent |
|------|-------------|------------|
| `forge_install` | Run `forge install` in project dir | yes |
| `forge_build` | Compile with `forge build -vvv`, records compilation telemetry | yes |
| `forge_test` | Run `forge test --json`, parses gas from JSON output, records `succeeded_fuzzing`/`failed_fuzzing` trial | yes |
| `run_synthesis` | Full pipeline (build → test → halmos), records detailed trial | no |

`SynthesisTools` wraps `Mutex<Box<dyn Database>>` and `Mutex<Option<SynthesisPipeline>>`. `DbConfig` stored for lazy pipeline init. Pipeline is lazily initialized on first `run_synthesis` call via `DbConfig::connect()`.

### `src/pipeline.rs` — Synthesis pipeline
`SynthesisPipeline::run()` three-phase gating:

```
forge build -vvv → fail → record failed_compilation
    │ pass
    ▼
forge test --json → fail → record failed_fuzzing
    │ pass
    ▼
halmos → counterexample → record failed_halmos
    │ timeout/partial → record succeeded_partial (accepted under partial model checking)
    │ all proven → record succeeded_full
```

`db` field is `Box<dyn Database>` — trait dispatch to either backend.

Pauses for iteration increments between phases. Each call to `run()` increments iteration counter (resumed from DB max). Records every trial with typed result.

**Gas extraction:** Parses `forge test --json` structured output into `HashMap<String, ForgeSuite>`:
- Unit tests → `kind.Unit.gas`
- Fuzz tests → `kind.Fuzz.mean_gas`
Sums all values into `forge_gas` field.

Halmos flags: `--solver-threads 16`, `--early-exit`, `--print-full-model`, `--solver-timeout-branching 1s`, `--solver-timeout-assertion 1s`.

**Testing:** `pipeline_test.rs` via `#[path = "pipeline_test.rs"] mod tests;`. Uses `mock_commands: Vec<Result<...>>` for deterministic stage mocking. 12 tests.

### `docker-compose.yml` — Redis for dev/testing
```bash
docker compose up -d      # or: just redis-up
docker compose down       # or: just redis-down
```

### `src/bin/migrate_sqlite_to_redis.rs` — Data migration
Standalone binary:
```bash
cargo run --bin migrate -- --sqlite-path <path> --redis-url redis://localhost:6379
```

### `src/bin/queue_controller.rs` — Automated Slurm synthesis executor

Reads synthesis jobs from Redis priority queue, submits Slurm jobs for model serving, launches Claude Code with MCP integration. Processes sequentially until queue empty.

**Queue Redis schema:**

```
cluster_runs                                         -> Sorted Set (member="{model}:{job_id}", score=priority)
{model_name}:{job_id}                                -> Hash { seed, project, prompt, model_name }
```

**Processing loop:**
1. ZPOPMAX `cluster_runs` → empty → exit 0
2. Parse `{model}:{id}`, HGETALL job hash → validate fields
3. Construct model path, generate sbatch (MODEL_PATH + SEED parameterized)
4. `ssh cluster "sbatch"` via stdin pipe → capture job ID
5. Poll `squeue --format %T` until RUNNING (default 30s interval, 30m timeout)
6. Generate `.claude/settings.json` with MCP server + model provider (backup existing)
7. `claude --prompt "..." --cd {project_dir}` — blocking
8. Restore original settings, repeat

```bash
queue_controller \
    --models-path ~/dll/llm/models \
    --project-root ~/dll/projects \
    [--redis-url redis://localhost:6379] \
    [--model-url http://127.0.0.1:8080/v1] \
    [--cluster-host cluster] \
    [--poll-interval 30] \
    [--poll-timeout 1800]
```

Strict fail-fast: any error (Redis conn, missing model, sbatch fail, Slurm FAILED/CANCELLED/TIMEOUT, Claude Code non-zero) terminates immediately. No retries.

### `src/bin/populate_queue.rs` — Batch synthesis job enqueuer

Generates N synthesis jobs via deterministic RNG and enqueues them into Redis for the queue controller. One command replaces manual per-job creation.

```bash
populate_queue \
    --model qwen3-solidity-27B-Q6_K.gguf \
    --seed 42 \
    --project my-project \
    --prompt-file prompt.md \
    --iterations 100 \
    [--redis-url redis://localhost:6379]
```

**Algorithm:** `ChaCha8Rng::seed_from_u64(seed)` → `rng.next_u64()` per iteration → HSET job hash (seed, project, prompt, model_name) → ZADD `cluster_runs` with priority = iteration index.

Validation upfront: model/project non-empty, prompt file exists and non-empty, iterations > 0.

## Key Patterns

- `rmcp` `#[tool(description, annotations(...))]` on `impl` blocks with `#[tool_router(server_handler)]`
- `std::process::Command` for forge/halmos subprocess calls
- `#[cfg(test)] pub mock_commands: Option<Vec<Result<(String, bool), String>>>` for pipeline test mocking
- `redis` with `tokio-comp` feature (pure Rust Redis client, no system dep)
- `rusqlite` with `bundled` feature (embeds libsqlite3, no system dep)
- `DbConfig` factory enum for polymorphic backend creation
- `Box<dyn Database>` trait objects in `Mutex` for rmcp compatibility
- `::redis::Commands` / `::redis::RedisError` qualified paths (edition 2024 module name collision with `db::redis` submodule)
- `anyhow::Result` for main; `Result<String, String>` for MCP tool convention
- `eprintln!` debug logging with `[DEBUG]` prefix throughout
- Edition 2024, musl target for static linking

## Dependencies

- `rmcp` — MCP Rust SDK from `https://github.com/modelcontextprotocol/rust-sdk` (git dep)
- `tokio` — async runtime
- `serde`/`serde_json` — forge test JSON parsing
- `clap` — CLI arg parsing
- `redis` — Redis client (pure Rust, no system dep)
- `chrono` — ISO 8601 timestamps for created_at
- `rusqlite` — SQLite, `bundled` feature embeds libsqlite3 (unconditional)
- `rand_chacha`/`rand_core` — deterministic RNG for `populate_queue` seed generation
- `tempfile` — temp DBs in tests (dev-dependency only)
