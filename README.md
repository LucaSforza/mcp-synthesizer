# Solidity Synthesis MCP Server

An MCP (Model Context Protocol) server for automated Solidity code synthesis and verification. Orchestrates **Foundry** compilation/fuzzing and **Halmos** symbolic model checking, persisting every trial in **Redis** or **SQLite** for metrics and analytics.

```
 LLM / Agent          MCP Server                CLI Tools
┌──────────┐   tools   ┌──────────────────┐    ┌──────────┐
│          │ ───────→  │  forge_install   │ →  │  forge   │
│  Claude  │           │  forge_build     │ →  │  forge   │
│   Code   │           │  forge_test      │ →  │  forge   │
│          │           │  run_synthesis   │ →  │ forge →  │
└──────────┘           │                  │    │ halmos   │
                       │  ┌────────────┐  │    └──────────┘
                       │  │ Redis/SQL  │  │
                       │  │ (telemetry)│  │
                       │  └────────────┘  │
                       └──────────────────┘
```

## Features

- **`forge_install`** — Install Foundry project dependencies
- **`forge_build`** — Compile with `forge build`, capture success/failure telemetry
- **`forge_test`** — Run unit and fuzzy tests with detailed failure logs
- **`run_synthesis`** — Full automated pipeline: compile → test → Halmos verification, with DB recording
- **Redis + SQLite persistence** — Projects, test runs, synthesis trials with type-constrained result tracking. Switch via `--db-type` at runtime.
- **Metrics** — Gas consumption (median/peak), compilation success rate, verification depth, synthesis efficiency
- **Halmos formal verification** — Invariant proof tracking with partial model checking support

## Prerequisites

- **Rust** 1.70+ (edition 2024)
- **Foundry** — `forge` binary on PATH ([install guide](https://book.getfoundry.sh/getting-started/installation))
- **Halmos** — `halmos` binary on PATH ([install guide](https://github.com/a16z/halmos))
- **Redis** (optional) — for Redis backend. Start via `docker compose up -d`.

## Install

```bash
cargo build --release
```

## Usage

```bash
# Redis backend (default)
cargo run -- --cwd ./my-foundry-project --project my-contracts

# SQLite backend
cargo run -- --cwd ./my-foundry-project --project my-contracts --db-type sqlite
```

### CLI Arguments

| Flag | Short | Description | Default |
|---|---|---|---|
| `--cwd` | `-c` | Foundry project directory (required) | — |
| `--project` | `-p` | Project name identifier (required) | — |
| `--invariants` | `-i` | Number of Halmos invariants to verify | `0` |
| `--db-type` | | Backend: `redis` or `sqlite` | `redis` |
| `--redis-url` | `-u` | Redis server URL | `redis://localhost:6379` |
| `--db-path` | `-l` | SQLite database file (when `--db-type sqlite`) | `$HOME/Documents/solidity-synthesis.db` |

### Claude Code

Add project-level (recommended):
```bash
claude mcp add --transport stdio --scope project solidity-synthesis \
  "cargo run --manifest-path ./Cargo.toml -- \
    --cwd . --project my-project --invariants 5"
```

Add user-global (all projects):
```bash
claude mcp add --transport stdio --scope user solidity-synthesis \
  "cargo run --manifest-path /abs/path/to/mcp-synthesizer/Cargo.toml -- \
    --cwd /abs/path/to/foundry/project \
    --project my-project \
    --invariants 5"
```

Remove:
```bash
claude mcp remove solidity-synthesis
```

### Claude Desktop

Edit `claude_desktop_config.json`:

| Platform | Path |
|---|---|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |

```json
{
  "mcpServers": {
    "solidity-synthesis": {
      "command": "cargo",
      "args": [
        "run",
        "--manifest-path", "/path/to/mcp-synthesizer/Cargo.toml",
        "--",
        "--cwd", "/path/to/foundry/project",
        "--project", "my-project",
        "--invariants", "5"
      ]
    }
  }
}
```

After editing, restart Claude Desktop.

## Tools

All tools expose typed annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`) for correct auto-permission behavior in Claude.

| Tool | Annotations | Description |
|---|---|---|
| `forge_install` | !readOnly, !destructive, idempotent | Install project dependencies |
| `forge_build` | !readOnly, !destructive, idempotent | Compile project, records pass/fail in DB |
| `forge_test` | **readOnly**, !destructive, idempotent | Run forge test suite |
| `run_synthesis` | !readOnly, !destructive, !idempotent | Full pipeline with DB recording |

## Synthesis Pipeline

When `run_synthesis` is called, the server executes this gated pipeline:

```
forge build ──fail──→ return compiler error
    │ pass
    ▼
forge test ──fail──→ return failure trace
    │ pass
    ▼
halmos ──counterexample──→ return violation trace
    │ partial/timeout
    ▼
accept (partial model checking)
    │ all proven
    ▼
synthesis successful ✓
```

Each attempt is recorded as a `synthesis_trial` with one of six result types:
- `failed_compilation` — forge build failed
- `failed_fuzzing` — forge test failed
- `succeeded_fuzzing` — forge test passed (standalone, no halmos)
- `failed_halmos` — Halmos found a counterexample
- `succeeded_partial` — Halmos timed out or partially proved (accepted)
- `succeeded_full` — All invariants proven

### Constraints

- Only the last trial in a test run can be `succeeded_*`; all preceding trials must be `failed_*`
- In a succeeded trial: `not_proved_invariants ≤ number_of_invariants`

## Database Schema

### Redis key schema

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

### SQLite schema

Three tables: `project`, `test_run`, `synthesis_trial`. Query with any SQLite client:

```sql
-- All trials for a project
SELECT * FROM synthesis_trial st
JOIN test_run tr ON st.test_run_id = tr.id
WHERE tr.project_id = 1
ORDER BY st.iteration;

-- Compilation success rate per project
SELECT p.name,
       SUM(tr.compilation_passed) AS passed,
       SUM(tr.compilation_not_passed) AS failed
FROM project p
JOIN test_run tr ON tr.project_id = p.id
GROUP BY p.name;
```

## Project Structure

```
src/
├── main.rs          # CLI entry point, arg parsing, server init
├── db/
│   ├── mod.rs       # Database trait, DbError, DbConfig, data structs
│   ├── redis.rs     # RedisDatabase implementation
│   ├── sqlite.rs    # SqliteDatabase implementation with migrations
│   ├── redis_test.rs  # Redis unit tests (FLUSHDB on DB 1)
│   └── sqlite_test.rs # SQLite unit tests (:memory:)
├── tools.rs         # MCP tool definitions (forge_install, forge_build, forge_test, run_synthesis)
├── pipeline.rs      # Build → test → halmos orchestration
└── pipeline_test.rs # Pipeline tests with mock commands
```

## Development

Built with the Rust MCP SDK (`rmcp`) using the `#[tool]` and `#[tool_router]` macros. All tools are synchronous and use `std::process::Command` for CLI tool interaction. Database access is wrapped in `Mutex<Box<dyn Database>>` for thread safety under `rmcp`'s async handler model.

```bash
# Build
cargo build

# Test
TEST_REDIS_URL=redis://localhost:6379/1 cargo test -- --test-threads 1

# Test with MCP Inspector
npx @modelcontextprotocol/inspector --transport stdio -- \
  cargo run -- --cwd /tmp --project test
```

## License

GNU General Public License v3.0
