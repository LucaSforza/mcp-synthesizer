# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
cargo build
cargo build --release
cargo run -- --cwd /path/to/foundry-project --project my-contracts
cargo run -- --cwd . --project test --invariants 5 --db-path /tmp/test.db
```

## Commands

- Build: `cargo build`
- MCP Inspector: `npx @modelcontextprotocol/inspector --transport stdio -- cargo run -- --cwd /tmp --project test`
- Requires `forge` and `halmos` on PATH at runtime (not needed for compile)

## Architecture

4 source files, single-threaded MCP server communicating via stdio transport:

**`src/main.rs`** — CLI entry point. Parses `--cwd`, `--project`, `--invariants`, `--db-path` with clap. Creates `Database` + `SynthesisTools`, serves over stdio transport via `rmcp`. Debug logging via `eprintln!` (`[DEBUG]` prefix) on stderr throughout.

**`src/db.rs`** — SQLite persistence layer. Schema: `project` (name, number_invariants), `test_run` (project_id, compilation stats), `synthesis_trial` (iteration, gas, result_type with CHECK constraint). Exposes `Database::new()`, CRUD for projects/test_runs/trials, and `get_metrics()` for aggregated gas/invariant/success stats.

**`src/tools.rs`** — MCP tool definitions via `#[tool]` and `#[tool_router]` macros from `rmcp`. Four tools: `forge_install`, `forge_build`, `forge_test`, `run_synthesis`. `SynthesisTools` wraps `Database` and `SynthesisPipeline` in `Mutex` for rmcp's async handler model.

**`src/pipeline.rs`** — `SynthesisPipeline` with `run()` method gating: forge build → forge test → halmos verification. Each call increments iteration. Records every trial in DB with typed result. Handles partial Halmos proofs (accepted under partial model checking). **Gas is extracted from forge test stdout** (not halmos — halmos has no gas output) via `extract_forge_gas()` which parses `gas: N` and `μ: N` (fuzz mean) patterns and sums all values. Stored in `forge_gas` field on pipeline.

## Pipeline Flow

```
forge build → fail → record failed_compilation
    │ pass
    ▼
forge test → fail → record failed_fuzzing
    │ pass
    ▼
halmos → counterexample → record failed_halmos
    │ timeout/partial → record succeeded_partial (accepted)
    │ all proven → record succeeded_full
```

## DB Trial Result Types

`failed_compilation` | `failed_fuzzing` | `failed_halmos` | `succeeded_partial` | `succeeded_full`

Constraints: last trial in test_run can be `succeeded_*`; `not_proved_invariants <= number_of_invariants`.

## Key Patterns

- `rmcp` `#[tool(description, annotations(...))]` on `impl` blocks with `#[tool_router(server_handler)]`
- `std::process::Command` for forge/halmos calls
- `rusqlite` with `bundled` feature (ships own SQLite)
- `Mutex<Database>` and `Mutex<Option<SynthesisPipeline>>` for shared state
- Errors returned as `Result<String, String>` per MCP convention
- Edition 2024, musl target for static linking
