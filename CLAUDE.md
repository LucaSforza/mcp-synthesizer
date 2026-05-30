# CLAUDE.md

MCP server per Solidity smart contract synthesis. Parte del progetto `git_diff_checker` — protected LLM agent environment per Foundry development.

## Parent Project Context

This repo lives inside `git_diff_checker` as git submodule (`git@github.com:LucaSforza/mcp-synthesizer.git`). The parent project protects "Golden Commit" lines from LLM agent modifications via:

- **PreToolUse hook** — directory whitelist (only `src/`), blocks `forge` commands directly (must use this MCP server)
- **PostToolUse hook** — selective revert of original lines after each tool call
- **Stop hook** — blocks session stop unless `coverage.info` exists

This MCP server is the **authorized path** for forge operations and synthesis. When parent stop hook blocks due to missing coverage, it tells agent to use `run_synthesis` here.

## Build & Run

```bash
cargo build
cargo build --release
cargo run -- --cwd /path/to/foundry-project --project my-contracts
cargo run -- --cwd . --project test --invariants 5 --db-path /tmp/test.db
```

**Flags:** `--cwd` (required), `--project` (required), `--invariants` (default 0), `--db-path` (default `$HOME/Documents/solidity-synthesis.db`)

**Package:** `mcp_synth` (Cargo.toml name). Binary output: `mcp_synth`.

**MCP Inspector:** `npx @modelcontextprotocol/inspector --transport stdio -- cargo run -- --cwd /tmp --project test`

**Runtime deps:** `forge` and `halmos` on PATH (not needed for compile). `halmos` requires python venv.

**Musl target:** `Cargo.toml` configures `x86_64-unknown-linux-musl` with `rust-lld` + `crt-static` for fully static binary.

## Build & Install (justfile)

```bash
just build     # or: just b  — builds with LLD if available
just install   # or: just i  — copies mcp_synth to ~/.local/bin/
```

`just install` uses md5sum to skip copy if binary unchanged.

## Architecture

6 source files, single-threaded MCP server via stdio transport (`rmcp` SDK from git):

### `src/main.rs` — CLI entry point
Parses `--cwd`, `--project`, `--invariants`, `--db-path` with clap. Initializes SQLite DB, creates/loads project, creates test run. Serves `SynthesisTools` over rmcp stdio transport. Debug logging via `eprintln!` (`[DEBUG]` prefix).

### `src/db.rs` — SQLite persistence
Schema:

```
project (id, name, number_invariants, created_at)
test_run (id, project_id, compilation_passed, compilation_not_passed, created_at)
synthesis_trial (id, test_run_id, iteration, gas_of_implementation,
                 result_type, not_proved_invariants, failure_detail,
                 is_full_synthesis, created_at)
```

Exposes `Database::new()`, `get_or_create_project()`, `create_test_run()`, `record_trial()`, `get_max_iteration(test_run_id)`, `increment_compilation_passed/not_passed()`, `get_project()`, `get_metrics()`.

**Note:** `get_max_iteration` scoped per `test_run_id` (not per project). Multiple test runs in same project each track their own iteration counter.

**Trial result types** (6 values, CHECK constraint):
`failed_compilation` | `failed_fuzzing` | `succeeded_fuzzing` | `failed_halmos` | `succeeded_partial` | `succeeded_full`

Constraints: succeeded_* must be last trial in test_run; `not_proved_invariants <= number_invariants`.

**`is_full_synthesis`** flag distinguishes standalone `forge_test` calls from full pipeline runs.

**Migrations:** `run_migrations()` auto-adds `is_full_synthesis` column and expands CHECK constraint if DB was created under old schema.

**Metrics:** `get_metrics()` computes median/peak gas, compilation stats, total trials, proven/unproven invariants, succeeded iteration count via SQL aggregation.

### `src/tools.rs` — MCP tool definitions
4 tools via `rmcp` `#[tool]` + `#[tool_router]` macros:

| Tool | Description | Idempotent |
|------|-------------|------------|
| `forge_install` | Run `forge install` in project dir | yes |
| `forge_build` | Compile with `forge build -vvv`, records compilation telemetry | yes |
| `forge_test` | Run `forge test --json`, parses gas from JSON output, records `succeeded_fuzzing`/`failed_fuzzing` trial | yes |
| `run_synthesis` | Full pipeline (build → test → halmos), records detailed trial | no |

`SynthesisTools` wraps `Mutex<Database>` and `Mutex<Option<SynthesisPipeline>>` for rmcp async handler model. Pipeline is lazily initialized on first `run_synthesis` call.

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

Pauses for iteration increments between phases. Each call to `run()` increments iteration counter (resumed from DB max). Records every trial with typed result.

**Gas extraction** (important — differs from earlier docs):
Extracts gas from `forge test --json` structured output, NOT from regex on stdout. `extract_forge_gas_json()` parses JSON into `HashMap<String, ForgeSuite>` then extracts:
- Unit tests → `kind.Unit.gas`
- Fuzz tests → `kind.Fuzz.mean_gas`
Sums all values into `forge_gas` field, stored in DB trial records.

Halmos flags: `--solver-threads 16`, `--early-exit`, `--print-full-model`, `--solver-timeout-branching 1s`, `--solver-timeout-assertion 1s`.

**Testing:** `pipeline_test.rs` via `#[path = "pipeline_test.rs"] mod tests;`. Uses `mock_commands: Vec<Result<...>>` for deterministic stage mocking. 10 tests covering: build pass/fail, test fail, halmos full/partial/counterexample, multi-iteration loop, gas JSON extraction, not_proved parsing, iteration resume from DB, full metrics.

### `src/db_test.rs` — Database unit tests
Separate test module via `#[path = "db_test.rs"] mod tests;`. Tests DB operations directly.

## Key Patterns

- `rmcp` `#[tool(description, annotations(...))]` on `impl` blocks with `#[tool_router(server_handler)]`
- `std::process::Command` for forge/halmos subprocess calls
- `#[cfg(test)] pub mock_commands: Option<Vec<Result<(String, bool), String>>>` for pipeline test mocking
- `rusqlite` with `bundled` feature (ships own SQLite — no system dep)
- `anyhow::Result` for main; `Result<String, String>` for MCP tool convention
- `eprintln!` debug logging with `[DEBUG]` prefix throughout
- Edition 2024, musl target for static linking

## Dependencies

- `rmcp` — MCP Rust SDK from `https://github.com/modelcontextprotocol/rust-sdk` (git dep)
- `tokio` — async runtime
- `serde`/`serde_json` — forge test JSON parsing
- `clap` — CLI arg parsing
- `rusqlite` (bundled) — embedded SQLite
- `tempfile` (dev) — test temporary directories
