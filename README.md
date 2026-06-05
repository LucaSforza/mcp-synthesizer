# Solidity Synthesis — MCP Server & Automation Tools

Toolkit for automated Solidity contract synthesis and verification. Part of the `git_diff_checker` project.

Four executables, one crate:

| Binary | Path | Purpose |
|--------|------|---------|
| `mcp_synth` | `src/bin/mcp_synth.rs` | MCP server exposing Foundry + Halmos as tools |
| `queue_controller` | `src/bin/queue_controller.rs` | Automated Slurm synthesis executor |
| `populate_queue` | `src/bin/populate_queue.rs` | Batch enqueue synthesis jobs into Redis |
| `migrate` | `src/bin/migrate_sqlite_to_redis.rs` | SQLite-to-Redis data migration |

---

## 1. `mcp_synth` — Synthesis MCP Server

Exposes Foundry and Halmos to LLM agents (Claude Code, Claude Desktop) via the Model Context Protocol. Orchestrates compilation, fuzzing, and symbolic model checking, persisting trials in Redis or SQLite.

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

### Prerequisites

- **Rust** 1.70+ (edition 2024)
- **Foundry** — `forge` on PATH ([install](https://book.getfoundry.sh/getting-started/installation))
- **Halmos** — `halmos` on PATH ([install](https://github.com/a16z/halmos))
- **Redis** (optional, for Redis backend) — `docker compose up -d`

### Install

```bash
cargo build --release
```

Binary at `target/release/mcp_synth`.

### Usage

```bash
# Redis backend (default)
mcp_synth --cwd ./my-foundry-project --project my-contracts

# SQLite backend (DEPRECATED — use Redis instead)
mcp_synth --cwd ./my-foundry-project --project my-contracts --db-type sqlite
```

### CLI Arguments

| Flag | Short | Description | Default |
|------|-------|-------------|---------|
| `--cwd` | `-c` | Foundry project directory (required) | — |
| `--project` | `-p` | Project name identifier (required) | — |
| `--invariants` | `-i` | Number of Halmos invariants | `0` |
| `--db-type` | | Backend: `redis` (default) or `sqlite` (deprecated) | `redis` |
| `--redis-url` | `-u` | Redis URL | `redis://localhost:6379` |
| `--db-path` | `-l` | SQLite path (deprecated, only with `--db-type sqlite`) | `$HOME/Documents/solidity-synthesis.db` |

### MCP Tools

All tools expose typed annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`) for correct auto-permission in Claude.

| Tool | Annotations | Description |
|------|-------------|-------------|
| `forge_install` | !readOnly, !destructive, idempotent | Install project deps |
| `forge_build` | !readOnly, !destructive, idempotent | Compile, record pass/fail |
| `forge_test` | **readOnly**, !destructive, idempotent | Run test suite |
| `run_synthesis` | !readOnly, !destructive, !idempotent | Full pipeline with DB recording |

### Synthesis Pipeline

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

Six result types: `failed_compilation`, `failed_fuzzing`, `succeeded_fuzzing`, `failed_halmos`, `succeeded_partial`, `succeeded_full`.

### Claude Code Integration

Add project-level:
```bash
claude mcp add --transport stdio --scope project solidity-synthesis \
  "cargo run --manifest-path ./Cargo.toml -- \
    --cwd . --project my-project --invariants 5"
```

Add user-global:
```bash
claude mcp add --transport stdio --scope user solidity-synthesis \
  "cargo run --manifest-path /abs/path/to/mcp-synthesizer/Cargo.toml -- \
    --cwd /abs/path/to/foundry/project --project my-project --invariants 5"
```

Remove:
```bash
claude mcp remove solidity-synthesis
```

### Claude Desktop

Edit `claude_desktop_config.json`:

| Platform | Path |
|----------|------|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |

```json
{
  "mcpServers": {
    "solidity-synthesis": {
      "command": "mcp_synth",
      "args": [
        "--cwd", "/path/to/foundry/project",
        "--project", "my-project",
        "--invariants", "5"
      ]
    }
  }
}
```

---

## 2. `queue_controller` — Automated Slurm Synthesis Executor

Reads synthesis jobs from a Redis priority queue, submits Slurm jobs for model serving, launches Claude Code with MCP integration. Processes sequentially until the queue is empty.

### Redis Schema

```
cluster_runs                    Sorted Set (member="{model}:{job_id}", score=priority)
{model_name}:{job_id}           Hash { seed, project, prompt, model_name }
```

### Usage

```bash
queue_controller \
    --models-path ~/dll/llm/models \
    --project-root ~/dll/projects
```

### CLI Arguments

| Flag | Description | Default |
|------|-------------|---------|
| `--models-path` | GGUF model directory (required) | — |
| `--project-root` | Synthesis projects directory (required) | — |
| `--redis-url` | Redis URL | `redis://localhost:6379` |
| `--model-url` | OpenAI-compatible endpoint | `http://127.0.0.1:8080/v1` |
| `--cluster-host` | SSH hostname for Slurm | `cluster` |
| `--llama-path` | llama-server path on cluster | `/home/sforza_2050030/.local/bin/llama-server` |
| `--tunnel-port` | Port for SSH tunnel | `8080` |
| `--poll-interval` | Slurm status poll interval (s) | `30` |
| `--poll-timeout` | Max wait for RUNNING state (s) | `1800` |

### Processing Loop

1. `ZREVRANGE cluster_runs 0 0 WITHSCORES` → peek (no removal)
2. Parse `{model}:{id}`, HGETALL job hash, validate fields
3. Construct model path, generate sbatch (MODEL_PATH + LLAMA_PATH + SEED)
4. `ssh cluster "sbatch"` via stdin → capture Slurm job ID
5. Poll `squeue --format %T` until RUNNING (sacct fallback for finished jobs)
6. `squeue --format %N` → compute node → `node_name_to_ip` (last 2 digits: node123 → `10.0.0.23`)
7. `ssh -L port:node_ip:port cluster -N` → tunnel (auto-killed on Drop)
8. Inject `mcpServers` into `.claude/settings.local.json` (preserves hooks, env, permissions)
9. `claude -p --output-format json --mcp-config mcp_config.json --strict-mcp-config "prompt"`
   - Overrides `ANTHROPIC_BASE_URL` + `ANTHROPIC_MODEL` env vars
   - Pipes output through `jq`, saves as `{model}_{id}.json`
10. Restore original `.settings.local.json` from backup
11. Check `check_succeeded_full()`: true → ZREM (job consumed); false → bail (job stays in queue)

Fail-fast on any error. Job stays in queue on failure. No retries.

---

## 3. `populate_queue` — Batch Queue Enqueuer

Generates N synthesis jobs with deterministic RNG and enqueues them into Redis. One command replaces manual per-job creation for large campaigns.

### Usage

```bash
populate_queue \
    --model qwen3-solidity-27B-Q6_K.gguf \
    --seed 42 \
    --project my-project \
    --prompt-file prompt.md \
    --iterations 100
```

### CLI Arguments

| Flag | Description | Default |
|------|-------------|---------|
| `--model` | Model filename identifier (required) | — |
| `--seed` | Initial RNG seed (required) | — |
| `--project` | Project name (required) | — |
| `--prompt-file` | Path to synthesis prompt file (required) | — |
| `--iterations` | Number of jobs to generate, must be > 0 (required) | — |
| `--redis-url` | Redis URL | `redis://localhost:6379` |

### Algorithm

```
rng = ChaCha8Rng::seed_from_u64(seed)

for i in 1..=iterations:
    generated_seed = rng.next_u64()
    HSET {model}:{i} { seed, project, prompt, model_name }
    ZADD cluster_runs {score=i, member="{model}:{i}"}
```

Deterministic and reproducible: same input seed always produces the same sequence of generated seeds.

### Validation (all upfront)

- `--model` not empty
- `--project` not empty
- `--prompt-file` exists and is not empty
- `--iterations` > 0

---

## 4. `migrate` — SQLite to Redis Migration

Standalone utility for migrating synthesis data from SQLite to Redis.

### Usage

```bash
cargo run --bin migrate -- \
    --sqlite-path /path/to/solidity-synthesis.db \
    --redis-url redis://localhost:6379
```

---

## Database Schema

### Redis Keys (Synthesis Telemetry)

```
project:ids                                         INCR counter
project:{id}                                        Hash { name, number_invariants, created_at }
project:name:{name}                                 String (project ID, uniqueness check)
test_run:ids                                        INCR counter
test_run:{id}                                       Hash { project_id, compilation_passed, compilation_not_passed, created_at }
test_run:by_project:{project_id}                    Set of test_run IDs
synthesis_trial:ids                                 INCR counter
synthesis_trial:{id}                                Hash { test_run_id, iteration, gas_of_implementation, result_type, not_proved_invariants, failure_detail, is_full_synthesis, created_at }
synthesis_trial:by_test_run:{test_run_id}           Sorted Set (member=trial_id, score=iteration)
synthesis_trial:by_project:{project_id}             Set of trial IDs
synthesis_trial:gas:by_project:{project_id}         Sorted Set (member=trial_id, score=gas)
```

### Redis Keys (Queue + Automation)

```
cluster_runs                                        Sorted Set (member="{model}:{id}", score=priority)
{model_name}:{job_id}                               Hash { seed, project, prompt }
```

### SQLite Schema

Three tables: `project`, `test_run`, `synthesis_trial`.

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

---

## Project Structure

```
src/
├── main.rs                     # mcp_synth entry point
├── bin/
│   ├── queue_controller.rs     # queue_controller entry point
│   ├── populate_queue.rs       # populate_queue entry point
│   └── migrate_sqlite_to_redis.rs  # migrate entry point
├── queue_controller/
│   ├── mod.rs                  # Orchestrator loop + Args
│   ├── queue.rs                # Redis queue client
│   ├── slurm.rs                # Sbatch gen + Slurm SSH
│   └── claude.rs               # MCP settings + Claude Code launch
├── db/
│   ├── mod.rs                  # Database trait, DbError, DbConfig, data structs
│   ├── redis.rs                # RedisDatabase implementation
│   ├── sqlite.rs               # SqliteDatabase implementation
│   ├── redis_test.rs           # Redis tests (FLUSHDB on DB 1)
│   └── sqlite_test.rs          # SQLite tests (:memory:)
├── tools.rs                    # MCP tools (forge_install, forge_build, forge_test, run_synthesis)
├── pipeline.rs                 # Build → test → halmos orchestration
└── pipeline_test.rs            # Pipeline tests with mock commands
```

## Development

```bash
# Build all binaries
cargo build --release

# Build specific binary
cargo build --bin queue_controller
cargo build --bin populate_queue

# Test (Redis required)
docker compose up -d
TEST_REDIS_URL=redis://localhost:6379/1 cargo test -- --test-threads 1

# Justfile shortcuts
just build            # release build
just queue-controller # build queue_controller only
just populate-queue   # build populate_queue only
just redis-up         # start Redis
just test             # redis-up + test + redis-down
```

## Dependencies

- `rmcp` — MCP Rust SDK (git dep)
- `tokio` — async runtime
- `serde`/`serde_json` — JSON parsing
- `clap` — CLI arg parsing
- `redis` — Redis client (pure Rust, no system dep)
- `rusqlite` — SQLite, bundled feature embeds libsqlite3
- `chrono` — ISO 8601 timestamps
- `rand_chacha`/`rand_core` — deterministic RNG for seed generation
- `anyhow` — error handling

## License

GNU General Public License v3.0
