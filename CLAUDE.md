# CLAUDE.md

## CRITICAL: Never delete TODO comments
TODO comments in code are intentional technical-debt markers. They document planned improvements, known limitations, and future work. Deleting them without explicit user request is destructive — it erases context the user depends on. Never remove, rephrase, or resolve a TODO unless the user says "remove TODO" or "resolve TODO".

## Style: run cargo fmt occasionally
Not required before every commit, but run `cargo fmt` from time to time to keep formatting consistent. Avoid committing large formatting-only diffs mixed with logic changes — do fmt in its own commit.

MCP server per Solidity smart contract synthesis. Parte del progetto `git_diff_checker` — protected LLM agent environment per Foundry development.

## Parent Project Context

This repo lives inside `git_diff_checker` as git submodule (`git@github.com:LucaSforza/mcp-synthesizer.git`). The parent project protects "Golden Commit" lines from LLM agent modifications via:

- **PreToolUse hook** — directory whitelist (only `src/`), blocks `forge` commands directly (must use this MCP server)
- **PostToolUse hook** — selective revert of original lines after each tool call
- **Stop hook** — blocks session stop unless `coverage.info` exists

This MCP server is the **authorized path** for forge operations and synthesis. When parent stop hook blocks due to missing coverage, it tells agent to use `run_synthesis` here.

## Database Backends

Two backends via `Database` trait. Selected at runtime via `--db-type` flag. Both compiled unconditionally.

| Backend | CLI flag | Connection param | Status |
|---------|----------|------------------|--------|
| Redis (default) | `--db-type redis` | `--redis-url redis://localhost:6379` | ✅ Supported |
| SQLite | `--db-type sqlite` | `--db-path <path>` | 🚫 Deprecated — use Redis |

### Redis Databases

| DB | Usage | Notes |
|----|-------|-------|
| 0  | Real runs | `--redis-url redis://localhost:6379` (default) |
| 1  | Tests only | `TEST_REDIS_URL=redis://localhost:6379/1` (default in test code) |

Tests use `FLUSHDB` on DB 1 per module — never touch DB 0 real data.

### SQLite Storage (DEPRECATED)

Default path: `$HOME/Documents/solidity-synthesis.db`. SQLite backend has schema migrations in `SqliteDatabase::run_migrations()` — handles CREATE TABLE and CHECK constraint expansion for `succeeded_fuzzing`.

**SQLite is deprecated. Use Redis instead.**

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
`--db-path` (used with `--db-type sqlite`), `--model-name` (optional, persisted to test_run)

**Package:** `mcp_synth` (Cargo.toml name). Five binaries:

| Binary | Path | Purpose |
|--------|------|---------|
| `mcp_synth` | `src/main.rs` | MCP server for Solidity synthesis |
| `migrate` | `src/bin/migrate_sqlite_to_redis.rs` | SQLite-to-Redis data migration |
| `queue_controller` | `src/bin/queue_controller.rs` | Automated Slurm synthesis executor |
| `populate_queue` | `src/bin/populate_queue.rs` | Batch enqueue synthesis jobs into Redis |
| `stats_export` | `src/bin/stats_export.rs` | Statistical analysis of synthesis experiments |

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

### `src/bin/mcp_synth.rs` — CLI entry point
Parses `--cwd`, `--project`, `--invariants`, `--db-type`, `--redis-url`, `--db-path`, `--model-name` with clap. Builds `DbConfig` from args, calls `DbConfig::connect()` to get `Box<dyn Database>`. Creates/loads project, creates test run. Optionally persists `model_name` via `SynthesisTools`. Serves `SynthesisTools` over rmcp stdio transport. Debug logging via `eprintln!` (`[DEBUG]` prefix).

### `src/synth/db/` — Database trait, Redis + SQLite implementations

**`src/synth/db/mod.rs`** — Shared definitions:
- Data structs: `Project`, `TestRun`, `SynthesisTrial`, `Metrics`
- `DbError` enum — `Redis(::redis::RedisError)`, `Sqlite(rusqlite::Error)`, `InvalidResultType(String)`. Implements `Display` + `std::error::Error` + `From` for both error types.
- `Database` trait — 11 methods, all `&self`, bound `Send`:
  `get_or_create_project`, `create_test_run`, `set_test_run_model_name` (default no-op),
  `record_trial`, `get_max_iteration`,
  `increment_compilation_passed`, `increment_compilation_not_passed`, `get_project`, `get_metrics`
- `DbConfig` factory enum — `Redis { url }` + `Sqlite { path }`. `connect() -> Result<Box<dyn Database>, DbError>`.
- `validate_trial_params()` — shared validation for result_type + invariant constraints

**`src/db/redis.rs`** — `RedisDatabase` struct + `impl Database`. Redis key schema:

```
project:ids                                         -> INCR counter
project:{id}                                        -> Hash { name, number_invariants, created_at }
project:name:{name}                                 -> String (id, for uniqueness check)
test_run:ids                                        -> INCR counter
test_run:{id}                                       -> Hash { project_id, compilation_passed, compilation_not_passed, model_name (optional), created_at }
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
- `src/db/redis_test.rs` — 17 tests, `FLUSHDB` on DB 1 per module
- `src/db/sqlite_test.rs` — 15 tests, `:memory:` per module

### `src/synth/tools.rs` — MCP tool definitions
4 tools via `rmcp` `#[tool]` + `#[tool_router]` macros:

| Tool | Description | Idempotent |
|------|-------------|------------|
| `forge_install` | Run `forge install` in project dir | yes |
| `forge_build` | Compile with `forge build -vvv`, records compilation telemetry | yes |
| `forge_test` | Run `forge test --json`, parses gas from JSON output, records `succeeded_fuzzing`/`failed_fuzzing` trial | yes |
| `run_synthesis` | Full pipeline (build → test → halmos), records detailed trial | no |

`SynthesisTools` wraps `Mutex<Box<dyn Database>>` and `Mutex<Option<SynthesisPipeline>>`. `DbConfig` stored for lazy pipeline init. Pipeline is lazily initialized on first `run_synthesis` call via `DbConfig::connect()`. On construction, if `model_name` is provided, calls `db.set_test_run_model_name()` to persist it in Redis via HSET.

### `src/synth/pipeline.rs` — Synthesis pipeline
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

**Module structure** (under `src/queue_controller/`):

| File | Responsibility | Lines |
|------|----------------|-------|
| `mod.rs` | Orchestration: `run()` + 11 step functions (Steps 1-14) | ~480 |
| `cleanup.rs` | Resource lifecycle: `CleanupState`, `CLEANUP`, `do_cleanup`, `CleanupGuard`, `cleanup_and_reset` | ~90 |
| `claude.rs` | Claude Code subprocess + settings management | ~160 |
| `git_persistence.rs` | Git branch, commit, push via `git2` | ~270 |
| `queue.rs` | Redis queue client (`QueueClient`) | ~120 |
| `health.rs` | Startup, loop, and job preflight health checks | ~200 |
| `slurm.rs` | Sbatch generation, polling, SSH tunnel, node resolution | ~320 |
| `synthesis_usage.rs` | Claude Code output parsing + usage metrics | ~270 |
| `synthesis_monitor.rs` | Slurm job recovery: polls Claude + Slurm, recreates server on expiry | ~155 |

**Design principles:**
- `mod.rs` reads like an execution script — `run()` + step functions are the flow
- `cleanup.rs` owns all resource lifecycle (isolated from orchestration)
- `health.rs` owns environment validation (separated from orchestration)
- `Args` (CLI definition) lives in the binary file, not in `mod.rs`
- All step functions call into lower modules; they never implement infrastructure

**Important architecture:** queue_controller runs **locally**. Model files and Slurm are on **cluster**. SSH tunnel forwards cluster compute node port to localhost so Claude Code (local) can reach the model server.

**Test projects path (local):** `/home/softdream/Programming/gits/git_diff_checker/test/`
- test2 at `/home/softdream/Programming/gits/git_diff_checker/test/test2`
- test3, test4, test5 in same dir

**Cluster access:** `ssh cluster` (configured in `~/.ssh/config`). Model files at `~/dll/llm/models/` on cluster. Project dirs on cluster filesystem too (NAS), but local copy needed for Claude Code to run.

**Common commands on cluster:**
```bash
# Check Slurm queue
ssh cluster squeue -u $USER

# Check model files
ssh cluster ls ~/dll/llm/models/

# Submit interactive job
ssh cluster srun --gpus=1 --mem=41G ...
```

**Queue Redis schema:**

```
cluster_runs                                         -> Sorted Set (member="{model}:{job_id}", score=priority)
{model_name}:{job_id}                                -> Hash { seed, project, prompt }
```

**Processing phases (14 steps + 3 health check gates, 7 phases):**

| Phase | Steps | Function | What it does |
|-------|-------|----------|-------------|
| 0 | startup | `health::run_startup_checks` | Once before loop: cluster SSH, Slurm, models dir, llama path, claude binary, project root, SSH key |
| 0 | per-iteration | `health::run_loop_checks` | Each iteration: Redis ping, cluster SSH, claude binary |
| 0 | preflight | `health::run_job_preflight_checks` | After job load, before Slurm: project dir, prompt.md, git repo |
| 1 | 1-3 | `peek_and_load_job` | ZREVRANGE peek, parse `{model}:{id}`, HGETALL metadata |
| 2 | 4-5 | `submit_slurm_job` | Generate sbatch, `ssh sbatch` via stdin pipe |
| 2 | 6 | `wait_and_create_tunnel` | Poll squeue until RUNNING, resolve node IP, `ssh -L` tunnel |
| 3 | 7+7b | `prepare_project_environment` | Resolve project dir, read `prompt.md` |
| 3 | 8+8b | `setup_claude_and_git` | Inject mcpServers (with `--model-name`) in settings.local.json, create git branch |
| 4 | 9 | `run_claude_code` + `SynthesisMonitor` | Kill stale mcp_synth, spawn `claude -p` → monitor loop polls Claude exit + Slurm health, recovers model server on expiry |
| 5 | 10+10b | `cleanup_environment` | Restore settings, `scancel` Slurm job |
| 6 | 11 | `check_claude_result` | Verify exit status + `check_succeeded_full()` |
| 6 | 12 | `remove_job_from_queue` | ZREM from `cluster_runs` |
| 7 | 13 | `persist_usage_to_redis` | Parse stream-json output, write usage to test_run |
| 7 | 14 | `push_synthesis_to_git` | Stage, commit, push, restore original branch |

**Logging:** All debug output uses plain `eprintln!` — no per-job prefix. Separator banners (`===== Job ... =====`) printed at start/end of each job iteration for visual separation.

**Cleanup lifecycle:** `CleanupState` (in `cleanup.rs`) is populated incrementally as each step acquires a resource. Three paths drain it:
- **Happy path**: `cleanup_environment` + `cleanup_and_reset` at end of loop iteration
- **Signal**: SIGINT/SIGTERM → dedicated thread → `do_cleanup` → exit
- **Unwind**: `CleanupGuard::drop` → `do_cleanup`

**Model server recovery (`synthesis_monitor.rs`):** Phase 4 replaces blocking `claude_child.wait()` with `SynthesisMonitor::wait_for_completion()`. The monitor polls both Claude exit status (`try_wait`) and Slurm job state (`get_job_state`) every `poll_interval` seconds. If the Slurm job enters a terminal state (Completed, Failed, Cancelled, Timeout, NotFound), `recover()` resubmits the model-serving job and re-establishes the SSH tunnel:

1. Submit new sbatch with same parameters (model path, llama path, seed)
2. Wait for RUNNING (`poll_job`)
3. Resolve compute node IP
4. Establish new SSH tunnel (`TunnelHandle` replaces old one — `Drop` closes old tunnel)
5. Update `slurm_job_id` and sync to `CleanupState`

On the happy path (job never expires), behavior is identical to the old blocking wait.

**Health checks (`health.rs`):** Three-level validation system to fail fast before spending cluster resources:

- **Startup** (`run_startup_checks`): Runs once before the controller loop. Validates cluster SSH is reachable, `sinfo` responds, models directory exists, `llama-server` is executable, `claude` binary is on PATH, project root exists, and SSH key file has safe permissions. Any failure is fatal.

- **Loop** (`run_loop_checks`): Runs at the start of each loop iteration. Cheap checks: Redis PING, `ssh {host} true`, `which claude`. Catches infrastructure degradation mid-run (e.g., Redis dropped, SSH agent expired).

- **Job preflight** (`run_job_preflight_checks`): Runs after loading job metadata, before submitting a Slurm job. Checks project directory exists, `prompt.md` is present (warn-only), and git repository is valid (HEAD readable, remote origin set — only if `--git-ssh-key` is provided).

Failure at any level produces a `[HEALTH]` prefixed diagnostic message and terminates the controller with a clear error.

```bash
queue_controller \
    --models-path ~/dll/llm/models \
    --project-root /home/softdream/Programming/gits/git_diff_checker/test \
    [--redis-url redis://localhost:6379] \
    [--model-url http://127.0.0.1:8080/v1] \
    [--cluster-host cluster] \
    [--llama-path /home/sforza_2050030/.local/bin/llama-server] \
    [--tunnel-port 8080] \
    [--poll-interval 30] \
    [--poll-timeout 1800] \
    [--git-ssh-key ~/.ssh/id_ed25519]
```

**`--project-root`** must point to local dir containing project subdirs (e.g. `test2/`). Model is on cluster — `--models-path` is the cluster-side path embedded in sbatch script.

Strict fail-fast: any error (Redis conn, sbatch fail, Slurm FAILED, Claude Code non-zero, trial not succeeded_full) terminates immediately. Job stays in queue on failure.

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

**Algorithm:** `ChaCha8Rng::seed_from_u64(seed)` → `rng.next_u64()` per iteration → HSET job hash (seed, project, prompt) → ZADD `cluster_runs` with priority = iteration index. Model name is in the key, not duplicated in hash.

Validation upfront: model/project non-empty, prompt file exists and non-empty, iterations > 0.

### `src/bin/stats_export.rs` — Synthesis experiment analysis

Exports synthesis trial data from Redis and computes statistics per experiment group. Outputs a canonical `analysis.json` dataset consumed by the Python visualization layer.

```bash
cargo run --bin stats_export -- \
  --redis-url redis://localhost:6379 \
  --range experiment_a=37:46 \
  --output results/
```

Multiple groups for comparison:
```bash
cargo run --bin stats_export -- \
  --range group_a=37:41 \
  --range group_b=42:46 \
  --output results/
```

**Outputs:** `analysis.json`, `summary.json`, `summary.csv`, `report.md`

**Module structure** (`src/stats/`):

| File | Responsibility |
|------|----------------|
| `types.rs` | `GasObservation`, `ExperimentGroup`, `GroupStatistics`, `Outlier` |
| `parser.rs` | Range parser (`label=start:end`) |
| `loader.rs` | `RedisLoader` — read-only extraction from Redis |
| `statistics.rs` | Statistical functions: mean, median, variance, std_dev, quartiles, IQR, CV, outlier detection |
| `report.rs` | JSON/CSV/Markdown report generation + `analysis.json` |
| `statistics_test.rs` | 18 tests for statistical computations |
| `parser_test.rs` | 11 tests for range parsing |

### `scripts/visualize_synthesis.py` — Python visualization

Reads `analysis.json` and generates publication-quality plots. Uses **uv** for dependency management (PEP 723 inline metadata).

```bash
# Direct execution (uv auto-installs deps):
./scripts/visualize_synthesis.py results/analysis.json results/

# Or via uv:
uv run scripts/visualize_synthesis.py results/analysis.json results/
```

**Required:** `uv` on PATH (no pip/venv needed).

**Outputs:**

| File | Type | Description |
|------|------|-------------|
| `gas_boxplot.svg` | Box + strip | Distribution with quartiles, mean, std-dev overlay |
| `gas_violin.svg` | Violin | Full distribution shape (reveals multimodality) |
| `gas_scatter.svg` | Scatter | Gas vs test_run_id, colored by group |
| `gas_histogram.svg` | Histogram | One subplot per group (faceted) |
| `gas_ecdf.svg` | ECDF | Empirical CDF for direct group comparison |

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
- `signal-hook::iterator::Signals` for SIGINT/SIGTERM handling in a dedicated thread (self-pipe mechanism)
- `auth-git2::GitAuthenticator` for deterministic SSH key authentication (no ssh-agent)
- Two-phase Git API: `checkout_synthesis_branch()` before Claude Code, `commit_and_push()` after success
- `static Mutex<Option<CleanupState>>` for graceful shutdown state shared between main loop and signal handler
- Separate `cleanup.rs` module for all resource lifecycle management (CleanupState, CLEANUP, do_cleanup, CleanupGuard, cleanup_and_reset)
- `SynthesisMonitor` in `synthesis_monitor.rs` — dedicated component for Claude Code + Slurm job monitoring during synthesis. Owns tunnel handle, polls both processes via `try_wait()`/`get_job_state()`, recovers model server on Slurm expiry via `recover()` (resubmit sbatch + re-establish tunnel + sync CleanupState). Replaces blocking `claude_child.wait()` with polling loop
- `JobState::is_terminal()` on `slurm::JobState` — classifies terminal states (Completed, Failed, Cancelled, Timeout, NotFound) for expiry detection. `get_job_state` made `pub(crate)` for monitor access
- `health.rs` three-level validation: `run_startup_checks` (once before loop), `run_loop_checks` (every iteration), `run_job_preflight_checks` (after job load, before Slurm). Each check is a private function with `[HEALTH]` prefixed logging, called from public orchestrator functions that bail on first failure
- `QueueClient::ping()` — lightweight Redis connectivity check via `PING`, used by `run_loop_checks` to detect dropped connections mid-run
- CLI Args defined in `src/bin/queue_controller.rs`, not in `mod.rs` — binary owns its interface
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
- `signal-hook` — SIGINT/SIGTERM handling for queue_controller graceful shutdown
- `git2`/`auth-git2` — Git persistence (branch, commit, push) for synthesis results
- `tempfile` — temp DBs in tests (dev-dependency only)
