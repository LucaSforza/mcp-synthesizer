# Support External API Endpoints in Queue Controller

## Objective

The system currently supports only models served on the cluster through Slurm, SSH tunnels, and `llama-server`.

Add support for user-provided external API endpoints so that `queue_controller` can use those endpoints directly without launching any cluster infrastructure.

---

# Functional Requirements

## 1. Populate Queue

Extend `populate_queue` to support two execution modes.

### Cluster Mode (existing behavior)

```bash
populate_queue \
    --model qwen3-solidity-27B-Q6_K.gguf \
    ...
```

### API Mode

```bash
populate_queue \
    --api-url https://example.com/v1 \
    ...
```

---

## 2. Validation Rules

Exactly one of the following options must be provided:

* `--model`
* `--api-url`

Validation requirements:

* If both are specified → return an error.
* If neither is specified → return an error.

---

## 3. Redis Persistence

Jobs must contain an explicit execution mode.

Add the following fields to job metadata:

```text
execution_mode
api_url
```

Valid values:

```text
execution_mode = cluster
```

or

```text
execution_mode = api
```

### Cluster Mode

```text
execution_mode = cluster
model present
api_url absent
```

### API Mode

```text
execution_mode = api
api_url present
model not required
```

---

## 4. Backward Compatibility

Existing Redis jobs that do not contain `execution_mode` must be interpreted as:

```text
execution_mode = cluster
```

This avoids any Redis migration and preserves compatibility with already queued jobs.

---

# Queue Controller Changes

## 5. Job Loading

When loading a job, read:

```text
execution_mode
api_url
```

and build an explicit internal representation.

Example:

```rust
enum ExecutionMode {
    Cluster,
    Api,
}
```

---

## 6. Cluster Mode

Cluster mode must preserve the current behavior unchanged:

```text
submit Slurm job
↓
wait for RUNNING
↓
create SSH tunnel
↓
configure Claude
↓
run Claude
↓
cleanup
```

No regressions are allowed.

---

## 7. API Mode

When:

```text
execution_mode = api
```

the controller must NOT:

* submit Slurm jobs
* create SSH tunnels
* poll Slurm
* perform model server recovery
* use `llama-server`

Instead it must execute:

```text
load job
↓
use api_url as model endpoint
↓
configure Claude
↓
run Claude
↓
cleanup
```

---

## 8. Unified Model Endpoint

Generalize the concept of a model endpoint.

Introduce a shared structure representing the endpoint used by Claude Code regardless of its origin.

Example:

```rust
struct ModelEndpoint {
    url: String,
}
```

### Cluster Origin

```text
http://127.0.0.1:<tunnel-port>/v1
```

### API Origin

```text
api_url
```

All downstream controller logic should consume only `ModelEndpoint` and should not need to know whether it originated from a cluster deployment or an external API.

---

## 9. Claude Configuration

Claude Code configuration must be generated from `ModelEndpoint`.

There should be a single MCP configuration path.

The only difference between execution modes should be how the endpoint is obtained.

---

## 10. Cleanup

Cleanup must continue to work in both modes.

In API mode, no Slurm-related or tunnel-related cleanup operations should be executed.

Cleanup must remain idempotent.

---

## 11. Health Checks

Separate cluster-specific checks from generic checks.

### Cluster Mode

Keep existing validations:

* cluster SSH connectivity
* Slurm availability
* models directory existence
* `llama-server` availability

### API Mode

Do not execute cluster-related validations.

Only validate:

* `api_url` is present
* `api_url` is a valid URL

Optional endpoint reachability checks may be added later but are not required in this task.

---

# Architecture Guidelines

## Prefer Explicit Execution Modes

Avoid behavior such as:

```rust
if api_url.is_some()
```

Use an explicit mode instead:

```rust
enum ExecutionMode {
    Cluster,
    Api,
}
```

This keeps the behavior deterministic and easier to maintain.

---

## Reuse Existing Cleanup Infrastructure

Do not introduce a separate cleanup path.

Continue using the existing resource lifecycle management based on optional resources:

```rust
Option<JobId>
Option<TunnelHandle>
```

Resources that were never created should naturally require no cleanup.

---

## Minimize Branching

The desired flow is:

```text
load job

if Cluster:
    create endpoint through Slurm + tunnel

if Api:
    create endpoint from api_url

setup Claude(endpoint)
run Claude()
```

All downstream logic should be shared.

---

# Acceptance Criteria

## Scenario 1: Cluster Mode

Given:

```text
execution_mode = cluster
```

the controller must:

* launch Slurm
* create the SSH tunnel
* run Claude Code
* complete the synthesis workflow

with behavior identical to the current implementation.

---

## Scenario 2: API Mode

Given:

```text
execution_mode = api
api_url = https://example.com/v1
```

the controller must:

* not launch Slurm
* not create an SSH tunnel
* not use `llama-server`
* pass the endpoint directly to Claude Code
* complete the synthesis workflow successfully

---

## Scenario 3: Backward Compatibility

Given an existing Redis job without:

```text
execution_mode
```

the controller must treat it as:

```text
execution_mode = cluster
```

and preserve the current behavior without requiring any migration.
