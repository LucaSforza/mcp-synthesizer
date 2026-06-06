# Queue Controller Health Check System - High Level Design

## Context

The `queue_controller` currently assumes that its environment is healthy and only discovers problems when a specific phase fails (Redis, SSH, Git push, Slurm, etc.).

We want to introduce a structured health-check system that:

1. Validates critical dependencies before entering the main controller loop.
2. Re-validates important dependencies before processing each new job.
3. Runs job-specific preflight checks before allocating expensive resources.
4. Fails early with clear diagnostics instead of discovering problems after a long synthesis run.

This document is intentionally high-level because it is based only on the information available in `CLAUDE.md`. It should not assume complete project knowledge.

---

# Design Goals

## Goals

* Fail fast when infrastructure is misconfigured.
* Detect broken SSH/Git credentials before spending cluster resources.
* Keep `queue_controller::mod.rs` readable.
* Centralize environment validation logic.
* Make health checks reusable and testable.

## Non Goals

* Implementing every detail immediately.
* Refactoring existing orchestration logic.
* Changing synthesis behavior.

---

# Proposed Architecture

Create a dedicated module:

```text
src/queue_controller/
├── health.rs
└── runtime_checks.rs
```

or a single module if preferred:

```text
src/queue_controller/health.rs
```

The important part is separating orchestration from validation logic.

---

# Health Check Categories

## 1. Startup Checks

These run once before entering the controller loop.

If any of them fail, the controller should exit immediately.

### Redis Connectivity

Purpose:

* Queue access
* Usage persistence
* Job removal

Possible API:

```rust
check_redis_connection(...)
```

Validation:

* Connect to Redis
* Execute PING or a lightweight command

Failure example:

```text
Cannot connect to Redis
```

---

### Cluster SSH Connectivity

Purpose:

Verify that the cluster is reachable and authentication works.

Possible API:

```rust
check_cluster_connectivity(...)
```

Validation:

```bash
ssh cluster hostname
```

or

```bash
ssh cluster true
```

---

### Git SSH Authentication

Purpose:

Ensure that Git push will work before a synthesis starts.

Possible API:

```rust
check_git_ssh_auth(...)
```

Validation should use the same authentication path as `git_persistence.rs`.

Preferred test:

```bash
git ls-remote origin
```

Failure example:

```text
Git SSH authentication failed
```

---

### Claude Availability

Purpose:

Ensure Claude Code is installed and executable.

Possible API:

```rust
check_claude_binary(...)
```

Validation:

```bash
claude --version
```

---

### Slurm Availability

Purpose:

Verify cluster scheduler availability.

Possible API:

```rust
check_slurm_available(...)
```

Validation examples:

```bash
ssh cluster sinfo
```

or

```bash
ssh cluster squeue
```

---

### Models Directory Validation

Purpose:

Verify that the configured models directory exists on the cluster.

Possible API:

```rust
check_models_directory(...)
```

Validation:

```bash
ssh cluster test -d <models-path>
```

---

### Llama Server Validation

Purpose:

Verify that the configured llama executable exists.

Possible API:

```rust
check_cluster_llama_path(...)
```

Validation:

```bash
ssh cluster test -x <llama-path>
```

---

### Project Root Validation

Purpose:

Verify that the configured local project root exists.

Possible API:

```rust
check_project_root(...)
```

Validation:

* Directory exists
* Readable

---

# 2. Loop Checks

These run before processing each new job.

The goal is to detect infrastructure degradation while the controller is running.

Possible API:

```rust
run_loop_health_checks(...)
```

---

### Redis Still Reachable

Lightweight connectivity verification.

---

### Cluster Still Reachable

Validation:

```bash
ssh cluster true
```

---

### Git Authentication Still Works

Validation:

```bash
git ls-remote origin
```

Reasons:

* SSH agent expired
* Key removed
* Permissions changed

---

### Claude Binary Still Available

Very lightweight executable check.

---

# 3. Job Preflight Checks

These run after a job is loaded but before expensive resources are allocated.

Possible API:

```rust
run_job_preflight_checks(...)
```

---

### Project Exists

After queue metadata is loaded:

```rust
check_project_exists(...)
```

Validation:

```text
<project_root>/<project_name>
```

exists.

---

### Prompt Exists

Before project preparation:

```rust
check_prompt_file(...)
```

Validation:

```text
prompt.md
```

exists and is readable.

---

### Git Repository Validation

Before branch creation and synthesis.

Possible API:

```rust
check_git_repository(...)
```

Validation:

* `.git` exists
* HEAD is readable
* origin remote exists

---

# 4. Runtime Checks

Checks performed after resources have already been allocated.

---

### Model Endpoint Health Check

This is likely the most important missing validation.

Current flow creates:

* Slurm job
* Tunnel

But the controller should also verify that the model server is actually responding before launching Claude.

Possible API:

```rust
verify_model_endpoint(...)
```

Suggested location:

Immediately after:

```rust
wait_and_create_tunnel()
```

Possible validations:

```http
GET /health
```

or

OpenAI-compatible endpoint probe.

Failure should abort before Claude starts.

---

### Slurm Job Monitoring

Already handled by:

```rust
SynthesisMonitor
```

No major architectural changes expected.

---

### SSH Tunnel Monitoring

Already implicitly handled by:

```rust
SynthesisMonitor
```

No major architectural changes expected.

---

# Suggested Public API

## health.rs

```rust
pub fn run_startup_checks(...)
pub fn run_loop_checks(...)
```

Possible internal functions:

```rust
check_redis_connection(...)
check_cluster_connectivity(...)
check_git_ssh_auth(...)
check_claude_binary(...)
check_slurm_available(...)
check_models_directory(...)
check_cluster_llama_path(...)
check_project_root(...)
```

---

## runtime_checks.rs

```rust
pub fn run_job_preflight_checks(...)
```

Possible internal functions:

```rust
check_project_exists(...)
check_prompt_file(...)
check_git_repository(...)
verify_model_endpoint(...)
```

---

# Integration Points

Desired flow:

```rust
fn main() {
    run_startup_checks()?;

    loop {
        run_loop_checks()?;

        let job = peek_and_load_job()?;

        run_job_preflight_checks(&job)?;

        process_job(job)?;
    }
}
```

---

# Additional Consideration

Not every check has the same cost.

Checks should be classified as:

## Cheap

Can run every loop iteration.

Examples:

* Redis ping
* Claude executable check
* Project existence

## Medium

Can run before each job.

Examples:

* SSH connectivity
* Git authentication

## Expensive

Should run only at startup or when required.

Examples:

* Deep cluster validation
* Full model endpoint probing
* Remote filesystem checks

This classification may be useful if the controller eventually processes thousands of jobs continuously.

---

# Implementation Priority

Recommended implementation order:

1. Redis connectivity check
2. Cluster SSH connectivity check
3. Git SSH authentication check
4. Slurm availability check
5. Model endpoint verification after tunnel creation
6. Project/Git repository validation

These checks are expected to eliminate the majority of avoidable failures before a long synthesis run begins.

