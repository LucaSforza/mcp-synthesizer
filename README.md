# Solidity Synthesis MCP Server

An MCP (Model Context Protocol) server for automated Solidity code synthesis and verification. Orchestrates **Foundry** compilation/fuzzing and **Halmos** symbolic model checking, persisting every trial in **SQLite** for metrics and analytics.

```
 LLM / Agent          MCP Server                CLI Tools
┌──────────┐   tools   ┌──────────────────┐    ┌──────────┐
│          │ ───────→  │  forge_install   │ →  │  forge   │
│  Claude  │           │  forge_build     │ →  │  forge   │
│   Code   │           │  forge_test      │ →  │  forge   │
│          │           │  run_synthesis   │ →  │ forge →  │
└──────────┘           │                  │    │ halmos   │
                       │  ┌────────────┐  │    └──────────┘
                       │  │  SQLite DB  │  │
                       │  │ (telemetry) │  │
                       │  └────────────┘  │
                       └──────────────────┘
```

## Features

- **`forge_install`** — Install Foundry project dependencies
- **`forge_build`** — Compile with `forge build`, capture success/failure telemetry
- **`forge_test`** — Run unit and fuzzy tests with detailed failure logs
- **`run_synthesis`** — Full automated pipeline: compile → test → Halmos verification, with DB recording
- **SQLite persistence** — Projects, test runs, synthesis trials with type-constrained result tracking
- **Metrics** — Gas consumption (avg/peak), compilation success rate, verification depth, synthesis efficiency
- **Halmos formal verification** — Invariant proof tracking with partial model checking support

## Prerequisites

- **Rust** 1.70+ (edition 2024)
- **Foundry** — `forge` binary on PATH ([install guide](https://book.getfoundry.sh/getting-started/installation))
- **Halmos** — `halmos` binary on PATH ([install guide](https://github.com/a16z/halmos))

## Install

```bash
git clone <repo>
cd my-mcp-server
cargo build --release
```

## Usage

```bash
# Minimal — uses defaults for DB path and invariants
cargo run -- --cwd ./my-foundry-project --project my-contracts

# Full options
cargo run -- \
  --cwd ./my-foundry-project \
  --project my-contracts \
  --invariants 5 \
  --db-path ~/Documents/my-synthesis.db
```

### CLI Arguments

| Flag | Short | Description | Default |
|---|---|---|---|
| `--cwd` | `-c` | Foundry project directory (required) | — |
| `--project` | `-p` | Project name identifier (required) | — |
| `--invariants` | `-i` | Number of Halmos invariants to verify | `0` |
| `--db-path` | `-d` | SQLite database location | `$HOME/Documents/solidity-synthesis.db` |

### Connecting from Claude Code

```bash
claude mcp add --transport stdio --scope project solidity-synthesis \
  "cargo run --manifest-path /path/to/my-mcp-server/Cargo.toml -- \
    --cwd . --project my-project --invariants 5"
```

### Connecting from Claude Desktop / Claude.ai

Add a custom connector with the stdio transport pointing to the compiled binary.

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

Each attempt is recorded as a `synthesis_trial` in SQLite with one of five result types:
- `failed_compilation` — forge build failed
- `failed_fuzzing` — forge test failed
- `failed_halmos` — Halmos found a counterexample
- `succeeded_partial` — Halmos timed out or partially proved (accepted)
- `succeeded_full` — All invariants proven

### Constraints

- Only the last trial in a test run can be `succeeded_*`; all preceding trials must be `failed_*`
- In a succeeded trial: `not_proved_invariants ≤ number_of_invariants`

## Database Schema

The SQLite database (`solidity-synthesis.db`) contains three tables:

- **`project`** — name (unique), number of invariants
- **`test_run`** — belongs to project, tracks compilation pass/fail counts
- **`synthesis_trial`** — belongs to test run, iteration number, gas, result type, failure detail

Query metrics with any SQLite client:

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
├── main.rs       # CLI entry point, arg parsing, server init
├── db.rs         # SQLite schema, CRUD, metrics aggregation
├── tools.rs      # MCP tool definitions (forge_install, forge_build, forge_test, run_synthesis)
└── pipeline.rs   # Build → test → halmos orchestration
```

## Development

Built with the Rust MCP SDK (`rmcp` v1.5.0) using the `#[tool]` and `#[tool_router]` macros. All tools are synchronous and use `std::process::Command` for CLI tool interaction. Database access is wrapped in `Mutex` for thread safety under `rmcp`'s async handler model.

```bash
# Build
cargo build

# Test with MCP Inspector
npx @modelcontextprotocol/inspector --transport stdio -- \
  cargo run -- --cwd /tmp --project test
```

## License

GNU General Public License v3.0
