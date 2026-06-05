# Specification: Queue Controller

## Overview

Binary that automates Solidity synthesis execution on a Slurm cluster.

Reads synthesis jobs from Redis priority queue, submits Slurm jobs for model serving (llama.cpp), establishes SSH tunnel to compute node, launches Claude Code with MCP integration. Processes jobs sequentially until queue empty.

**Binary name:** `queue_controller`
**Source:** `src/bin/queue_controller.rs` → `src/queue_controller/{mod,claude,slurm,queue}.rs`

---

## Architecture

```
┌─────────────┐     ┌──────────────┐     ┌─────────────────┐
│   Redis     │◄────│  Controller  │────►│  Slurm Cluster   │
│ cluster_runs│     │  (local)     │     │  ┌───────────┐  │
│ job hashes  │     │              │     │  │llama-server│  │
└─────────────┘     │ peek + ZREM  │     │  │ (GGUF)    │  │
                    │ on success   │     │  └─────┬─────┘  │
                    │              │     │        │ port    │
                    │  SSH tunnel  │◄────│────────┘ 8080    │
                    │  (local)     │     └─────────────────┘
                    │       │
                    │  launches
                    │       │
                    │  ┌────▼──────────┐
                    │  │  Claude Code  │
                    │  │  + mcp_synth  │
                    │  └───────────────┘
                    └──────────────────────
```

Single-threaded. One job at a time. Fail-fast on error.

---

## Redis Data Model

### Priority Queue

```
key: cluster_runs
type: Sorted Set
member format: {model_name}:{job_id}
score (priority): higher = higher priority
```

Example:

```
ZADD cluster_runs 100 qwen3-solidity-27B-Q6_K.gguf:1
ZADD cluster_runs  50 qwen3-solidity-27B-Q6_K.gguf:2
```

### Job Metadata Hash

```
key: {model_name}:{job_id}
type: Hash
fields:
  - seed      (string, required) — llama.cpp random seed
  - project   (string, required) — synthesis project name, maps to directory under --project-root
  - prompt    (string, required) — synthesis prompt text for Claude Code
```

`model_name` is **not** stored in the hash. It is extracted from the queue member key (`{model_name}:{job_id}`).

Example:

```
HSET qwen3-solidity-27B-Q6_K.gguf:1 \
  seed "183746192" \
  project "my-project" \
  prompt "Write a Solidity contract..."
```

### Synthesis Results (read-only by controller)

```
key: project:name:{project_name}           → project ID
key: project:{project_id}                  → Hash { name, number_invariants, created_at }
key: synthesis_trial:by_project:{pid}      → Set of trial_ids
key: synthesis_trial:{max_id}              → Hash { result_type, ... }
```

Controller checks `result_type == "succeeded_full"` to decide whether to remove job from queue.

---

## Processing Flow

### 1. Peek Job

```
ZREVRANGE cluster_runs 0 0 WITHSCORES
```

Read highest-priority job. **Do not remove** — removal happens only on successful completion.

If queue empty → exit with success.

### 2. Parse Member

Member format: `{model_name}:{job_id}`

Use `rsplitn(2, ':')` because model names may contain colons (e.g. `Qwen3:27B-Q6_K.gguf:1`).

### 3. Load Job Metadata

```
HGETALL {model_name}:{job_id}
```

Extract `seed`, `project`, `prompt`. Validate all exist.

If missing → fatal error, controller terminates. Job stays in queue.

### 4. Construct Model Path

```
{models_path}/{model_name}
```

Where `--models-path` is a CLI flag (base directory of GGUF models on the cluster).

### 5. Generate sbatch and Submit via SSH

Generate sbatch script with model path, llama-server path, and seed as parameters. Everything else hardcoded.

Pipe content via stdin to:

```
ssh cluster sbatch
```

No temporary file on disk.

llama-server flags in sbatch:

```
--model MODEL_PATH
--seed SEED
--models-max 1
-t 8
-ngl 99
-c 256000
--host 0.0.0.0
--cache-reuse 256
--temp 0.6
--top-p 0.95
--top-k 20
--min-p 0.0
--presence-penalty 0.0
--repeat-penalty 1.0
```

### 6. Poll Until RUNNING

```
ssh cluster squeue --job JOB_ID --noheader --format %T
```

Loop polling at configurable interval until state is one of:

| State | Action |
|---|---|
| RUNNING | Continue to step 6b |
| COMPLETED | Fatal (model server exited before use) |
| FAILED | Fatal |
| CANCELLED | Fatal |
| TIMEOUT | Fatal |
| PENDING / CONFIGURING / SUSPENDED / other | Retry |
| Not found in squeue | Fatal (job disappeared) |

Timeout configurable. If never reaches RUNNING → fatal.

### 6b. Resolve Compute Node and Establish SSH Tunnel

After job reaches RUNNING:

1. Get compute node hostname: `ssh cluster squeue --job JOB_ID --format %N` (fallback: `sacct`)
2. Convert node hostname to IP: last 2 digits of numeric suffix → `10.0.0.{digits}`
   - Example: `node123` → `10.0.0.23`
3. Establish SSH tunnel: `ssh -L PORT:10.0.0.XX:PORT cluster -N`
   - Tunnel auto-closed on Drop (Rust Drop impl kills child process)

### 7. Validate Project Directory

```
{project_root}/{job.project_name}
```

Must exist on local filesystem. If missing → fatal.

### 8. Set Up Claude Code MCP Settings

Inject `mcpServers` into `.claude/settings.local.json`:

```json
{
  "mcpServers": {
    "mcp_synth": {
      "command": "mcp_synth",
      "args": ["--cwd", "...", "--project", "...", "--db-type", "redis"],
      "env": {}
    }
  }
}
```

Also write standalone `mcp_config.json` for `--mcp-config` flag.

**Backup:** existing `settings.local.json` saved to `settings.local.json.queue_backup`. If no existing file, `None` → cleanup removes injected file on restore.

### 9. Kill Stale mcp_synth

```
pkill -f mcp_synth
```

Prevents port conflicts from parent Claude session's MCP server.

### 10. Launch Claude Code (blocking)

```
claude -p \
  --output-format stream-json \
  --dangerously-skip-permissions \
  --include-hook-events \
  --verbose \
  --mcp-config {mcp_config.json} \
  --strict-mcp-config \
  "{prompt}"
```

- `current_dir` set to project directory
- Env overrides: `ANTHROPIC_BASE_URL=http://127.0.0.1:8080`, `ANTHROPIC_MODEL={model_name}`
- stdout piped through `jq`, saved to `{model_name}_{job_id}.json` in project directory
- stderr inherited (visible to operator)
- stdin inherited (operator can interrupt)

### 11. Restore Claude Settings

- Remove `mcp_config.json`
- If backup exists: copy back, remove backup
- If no backup (file was created by us): delete `settings.local.json`

### 12. Check Synthesis Result

Query Redis for latest trial of this project:

```
GET project:name:{project_name} → project_id
SMEMBERS synthesis_trial:by_project:{project_id} → trial_ids
max(trial_ids) → HGETALL → result_type
```

If `result_type == "succeeded_full"`: `ZREM cluster_runs {member}` → job consumed.

Otherwise: **fatal error, job stays in queue**. Operator must inspect and decide.

### 13. Loop

Repeat from step 1.

---

## CLI Interface

```
queue_controller \
    --models-path ~/dll/llm/models \
    --project-root ~/dll/projects \
    [--redis-url redis://localhost:6379] \
    [--model-url http://127.0.0.1:8080/v1] \
    [--cluster-host cluster] \
    [--poll-interval 30] \
    [--poll-timeout 1800] \
    [--tunnel-port 8080] \
    [--llama-path /home/sforza_2050030/.local/bin/llama-server]
```

### Arguments

| Flag | Default | Description |
|---|---|---|
| `--models-path` | (required) | Base dir of GGUF model files on cluster |
| `--project-root` | (required) | Base dir of synthesis projects (local) |
| `--redis-url` | `redis://localhost:6379` | Redis server URL |
| `--model-url` | `http://127.0.0.1:8080/v1` | Model server API endpoint |
| `--cluster-host` | `cluster` | SSH hostname for Slurm cluster |
| `--poll-interval` | `30` | Seconds between Slurm status polls |
| `--poll-timeout` | `1800` | Max seconds wait for RUNNING state |
| `--tunnel-port` | `8080` | Port for SSH tunnel to compute node |
| `--llama-path` | `/home/sforza_2050030/.local/bin/llama-server` | llama-server binary path on cluster |

---

## Source Files

| File | Responsibility |
|---|---|
| `src/bin/queue_controller.rs` | Thin entrypoint, calls `app::run()`, exits with code 1 on error |
| `src/queue_controller/mod.rs` | Main loop, orchestrator, CLI args |
| `src/queue_controller/queue.rs` | Redis queue client: `peek_job`, `load_job`, `remove_job`, `check_succeeded_full` |
| `src/queue_controller/slurm.rs` | Sbatch generation, submit via SSH, poll state, node resolution, SSH tunnel |
| `src/queue_controller/claude.rs` | MCP config injection/restore, kill stale mcp_synth, launch Claude Code |

---

## Error Handling

Strict fail-fast. Any error terminates the controller:

| Failure | Effect |
|---|---|
| Redis connection fails | Exit immediately |
| Job metadata missing/empty | Exit immediately |
| sbatch submission fails | Exit immediately |
| Job enters FAILED/CANCELLED/TIMEOUT | Exit immediately |
| Project directory not found | Exit immediately |
| Claude Code exits non-zero | Exit immediately, job stays in queue |
| Synthesis not `succeeded_full` | Exit immediately, job stays in queue |

No retry logic. No skip-to-next-job. Operator intervention required.

---

## Non-Goals

- Parallel execution (one job at a time)
- Multiple simultaneous Slurm jobs
- Multiple concurrent Claude Code sessions
- Dynamic tuning of llama.cpp parameters
- Queue persistence outside Redis
- Distributed scheduling across multiple controllers

---

## Dependencies

- `redis` crate (pure Rust, no system dep)
- `clap` for CLI
- `serde_json` for settings file manipulation
- `rand_chacha` / `rand_core` (only in companion `populate_queue` binary)
- `anyhow` for error handling

Runtime dependencies:
- `ssh` client on PATH
- Slurm cluster accessible via SSH host `cluster`
- Redis server
- `claude` CLI on PATH
- `mcp_synth` binary on PATH
- `llama-server` on cluster PATH
- `pkill` for process management
