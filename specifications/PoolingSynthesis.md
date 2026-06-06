# PoolSynthesis / Model Server Recovery

## Goal

Allow long-running synthesis jobs to survive Slurm model-server expiration.

Currently:

- Claude Code runs locally.
- Model server runs inside a Slurm job.
- Access happens through an SSH tunnel.
- Slurm jobs have a maximum runtime (~30 minutes).

When the Slurm job expires, the model server disappears.

Instead of failing the synthesis, automatically recreate the model-serving job and continue.

---

## Desired behavior

While Claude Code is running:

1. Periodically check whether the Slurm job is still alive.
2. If the job is still running:
   - do nothing
   - continue polling

3. If the job has terminated:

   - close the current SSH tunnel
   - submit a new Slurm job using the same sbatch configuration
   - wait until the new job reaches RUNNING state
   - resolve the new compute node
   - establish a new SSH tunnel
   - update the stored Slurm job id
   - update cleanup state
   - continue monitoring

4. If Claude Code exits:
   - stop monitoring
   - proceed to cleanup and result evaluation

---

## Required state

The monitoring loop needs access to:

- model_name
- seed
- models_path
- llama_path
- cluster_host
- tunnel_port
- current_slurm_job_id
- current_tunnel_handle

---

## Refactoring suggestion

Introduce a dedicated component:

```rust
struct SynthesisMonitor
```

Responsibilities:

- monitor Claude process
- monitor Slurm job
- recreate model server when needed
- recreate SSH tunnel when needed
- keep cleanup state synchronized

Possible API:

```rust
impl SynthesisMonitor {
    pub fn wait_for_completion(
        &mut self,
        claude_child: &mut Child,
    ) -> Result<ExitStatus>;
}
```

---

## Detection logic

Every polling interval:

```text
if Claude exited:
    return exit status

if Slurm job running:
    continue

if Slurm job terminated:
    recover model server
```

---

## Recovery procedure

### Step 1

Close current SSH tunnel.

### Step 2

Submit new Slurm job using the same parameters:

- model path
- llama path
- seed

### Step 3

Wait until RUNNING.

### Step 4

Resolve node hostname.

### Step 5

Create new SSH tunnel.

### Step 6

Replace:

- tunnel handle
- slurm job id

### Step 7

Update CleanupState.

---

## Cleanup integration

CleanupState must always contain:

```rust
current_slurm_job_id
```

After recovery:

```rust
cleanup_state.slurm_job_id = new_job_id;
```

so cleanup always cancels the currently active model-serving job.

---

## Constraints

- Do not change existing behavior when jobs do not expire.
- Preserve current cleanup semantics.
- Preserve signal handling semantics.
- Keep the implementation isolated.
- Prefer introducing a dedicated monitoring component rather than expanding run().
