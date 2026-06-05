# Redis Data Model (Shared)

Single source of truth for all Redis keys used across `mcp_synth`, `queue_controller`, and `populate_queue`.

---

## Counters

Auto-incrementing ID counters. Read + write by `mcp_synth` only.

| Key | Type | Written by | Read by |
|---|---|---|---|
| `project:ids` | String (INCR) | `mcp_synth` | `mcp_synth` |
| `test_run:ids` | String (INCR) | `mcp_synth` | `mcp_synth` |
| `synthesis_trial:ids` | String (INCR) | `mcp_synth` | `mcp_synth` |

---

## Queue

Synthesis job queue. Written by `populate_queue`, consumed by `queue_controller`.

### `cluster_runs`

| Field | Value |
|---|---|
| Type | Sorted Set |
| Member format | `{model_name}:{job_id}` (e.g. `qwen3-solidity-27B-Q6_K.gguf:1`) |
| Score | Priority (higher = higher priority) |
| Written by | `populate_queue` (ZADD) |
| Read by | `queue_controller` (ZREVRANGE for peek, ZREM on success) |

Example:

```
ZADD cluster_runs 100 qwen3-solidity-27B-Q6_K.gguf:1
ZADD cluster_runs  50 qwen3-solidity-27B-Q6_K.gguf:2
```

### `{model_name}:{job_id}` (Job Metadata Hash)

| Field | Value |
|---|---|
| Type | Hash |
| Key example | `qwen3-solidity-27B-Q6_K.gguf:1` |
| Written by | `populate_queue` (HSET) |
| Read by | `queue_controller` (HGETALL) |

Fields:

| Hash field | Required | Description |
|---|---|---|
| `seed` | yes | llama.cpp random seed (decimal string) |
| `project` | yes | Synthesis project name, maps to directory under `--project-root` |
| `prompt` | yes | Synthesis prompt text for Claude Code |

`model_name` is **not** stored in the hash. It is encoded in the key `{model_name}:{job_id}` and extracted by `queue_controller` via `rsplitn(2, ':')`.

Example:

```
HSET qwen3-solidity-27B-Q6_K.gguf:1 \
  seed "183746192" \
  project "my-project" \
  prompt "Write a Solidity contract..."
```

---

## Projects

Written and read by `mcp_synth`. Read by `queue_controller` for synthesis result checks.

| Key | Type | Fields / Values | Written by | Read by |
|---|---|---|---|---|
| `project:ids` | String (INCR) | auto-increment counter | `mcp_synth` | `mcp_synth` |
| `project:{id}` | Hash | `name`, `number_invariants`, `created_at` | `mcp_synth` | `mcp_synth`, `queue_controller` |
| `project:name:{name}` | String | `{id}` (project ID as string) | `mcp_synth` | `mcp_synth`, `queue_controller` |

Example:

```
project:1 -> { name: "test2", number_invariants: 3, created_at: "2026-05-29 11:20:04" }
project:name:test2 -> "1"
```

---

## Test Runs

Written and read by `mcp_synth`. Not accessed by `queue_controller` or `populate_queue`.

| Key | Type | Fields / Values | Written by | Read by |
|---|---|---|---|---|
| `test_run:ids` | String (INCR) | auto-increment counter | `mcp_synth` | `mcp_synth` |
| `test_run:{id}` | Hash | `project_id`, `compilation_passed`, `compilation_not_passed`, `created_at` | `mcp_synth` | `mcp_synth` |
| `test_run:by_project:{pid}` | Set | member = `{test_run_id}` | `mcp_synth` | `mcp_synth` |

Example:

```
test_run:1 -> { project_id: 1, compilation_passed: 3, compilation_not_passed: 0, created_at: "..." }
test_run:by_project:1 -> ["1", "2", "3"]
```

---

## Trials

Written and read by `mcp_synth`. Read by `queue_controller` to check `succeeded_full`.

| Key | Type | Fields / Values | Written by | Read by |
|---|---|---|---|---|
| `synthesis_trial:ids` | String (INCR) | auto-increment counter | `mcp_synth` | `mcp_synth` |
| `synthesis_trial:{id}` | Hash | `test_run_id`, `iteration`, `gas_of_implementation`?, `result_type`, `not_proved_invariants`, `failure_detail`?, `is_full_synthesis`, `created_at` | `mcp_synth` | `mcp_synth`, `queue_controller` |
| `synthesis_trial:by_test_run:{trid}` | Sorted Set | member = `{trial_id}`, score = iteration | `mcp_synth` | `mcp_synth` |
| `synthesis_trial:by_project:{pid}` | Set | member = `{trial_id}` | `mcp_synth` | `mcp_synth`, `queue_controller` |
| `synthesis_trial:gas:by_project:{pid}` | Sorted Set | member = `{trial_id}`, score = gas | `mcp_synth` | `mcp_synth` |

`synthesis_trial:{id}` hash fields:

| Hash field | Required | Type in Redis | Description |
|---|---|---|---|
| `test_run_id` | yes | integer string | FK to test_run |
| `iteration` | yes | integer string | Iteration number |
| `gas_of_implementation` | no | integer string | Sum of forge gas values |
| `result_type` | yes | string | One of 6 valid types (see below) |
| `not_proved_invariants` | yes | integer string | Unproved invariant count |
| `failure_detail` | no | string | Full forge/halmos error output |
| `is_full_synthesis` | yes | `"0"` or `"1"` | `1` = full pipeline, `0` = standalone forge_test |
| `created_at` | yes | string | ISO 8601 timestamp |

### Valid result_type values

```
failed_compilation     — forge build failed
failed_fuzzing         — forge test failed
succeeded_fuzzing      — forge test passed (standalone, not full pipeline)
failed_halmos          — halmos found counterexample or errored
succeeded_partial      — halmos timeout/partial, accepted as success
succeeded_full         — halmos proved all invariants
```

Example:

```
synthesis_trial:9 -> {
  test_run_id: "5",
  iteration: "2",
  gas_of_implementation: "198169",
  result_type: "succeeded_full",
  not_proved_invariants: "0",
  is_full_synthesis: "1",
  created_at: "2026-05-29 12:25:59"
}
```

```
synthesis_trial:gas:by_project:1 -> [
  (9, 198169), (8, 199386), (19, 399370), ...
]
```
