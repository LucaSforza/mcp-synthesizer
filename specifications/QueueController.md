# Task Specification: Automated Synthesis Queue Controller

## Goal

Implement a new binary that automates synthesis execution on a Slurm cluster.

Today synthesis execution requires:

1. Manually launching Claude Code.
2. Manually selecting the model.
3. Manually providing the synthesis prompt.
4. Waiting for completion before starting the next synthesis.

The objective is to automate this workflow and allow multiple synthesis requests to be executed sequentially through a Redis-backed priority queue.

---

# High Level Architecture

Create a new binary (separate from the existing MCP server) acting as a **queue controller**.

Responsibilities:

1. Read synthesis jobs from Redis.
2. Submit jobs to the Slurm cluster.
3. Monitor execution status.
4. Launch Claude Code once the model server is available.
5. Process jobs sequentially.
6. Stop when the queue becomes empty.

Only one synthesis should run at a time.

---

# Redis Data Model

## Queue

Use a Redis Sorted Set as a priority queue.

Key:

```text
cluster_runs
```

Example:

```text
(
    model_a:job_1, priority=100
)
(
    model_b:job_2, priority=50
)
(
    model_c:job_3, priority=10
)
```

Higher score = higher priority.

The controller always pops the highest-priority job first.

---

## Job Metadata

Each queue entry references a Redis hash.

Key:

```text
{model_name}:{job_id}
```

Example:

```text
qwen3-solidity-27B:123
```

Hash contents:

```text
{
    seed: "104",
    project: "project-id",
    prompt: "synthesis specification",
    model_name: "qwen3-solidity-27B"
}
```

### Required Fields

| Field | Description |
|---------|-------------|
| seed | llama.cpp seed |
| project | synthesis project identifier |
| prompt | synthesis instructions |
| model_name | model filename identifier |

---

# Queue Processing Flow

Assume jobs already exist in Redis.

Controller workflow:

## 1. Pop Job

Remove the highest-priority entry from:

```text
cluster_runs
```

If no job exists:

```text
Exit successfully.
```

---

## 2. Load Job Metadata

Read the corresponding Redis hash.

Extract:

- model_name
- seed
- project
- prompt

Validate that all required fields exist.

If validation fails:

```text
Return error and terminate.
```

---

## 3. Construct Model Path

The executable receives a base model directory:

```bash
--models-path ~/dll/llm/models
```

Example:

```text
model_name = Qwen3-Coder-Next-APEX-I-Compact.gguf
```

Generated path:

```text
~/dll/llm/models/Qwen3-Coder-Next-APEX-I-Compact.gguf
```

---

## 4. Submit Slurm Job

Launch a Slurm job through SSH.

Cluster SSH configuration already exists under:

```text
cluster
```

Submission pattern:

```bash
ssh cluster "sbatch generated_job.sbatch"
```

The controller must dynamically generate the sbatch file using values stored in Redis.

At minimum, the following parameters must be configurable:

- model path
- seed

Everything else remains hardcoded for now.

Add a TODO comment for future sbatch parameterization.

---

## 5. Wait Until Model Server Is Running

After submission:

1. Verify Slurm accepted the job.
2. Poll Slurm status through SSH.
3. Wait until the job reaches RUNNING state.

Possible commands:

```bash
ssh cluster "squeue ..."
```

or

```bash
ssh cluster "sacct ..."
```

Implementation choice is up to the developer.

---

## 6. Failure Handling

If:

- sbatch submission fails
- job enters FAILED state
- job exits unexpectedly
- model server never becomes available

Then:

```text
Terminate the controller with an error.
```

Do not continue with subsequent jobs.

---

## 7. Launch Claude Code

Once the model server is available:

1. Start Claude Code.
2. Configure MCP correctly.
3. Ensure Claude can communicate with the existing MCP server.
4. Pass the synthesis prompt from Redis.

The synthesis should run exactly as if a user launched it manually.

---

## 8. Wait For Completion

Wait until Claude Code finishes.

If synthesis fails:

```text
Return error.
```

Otherwise continue.

---

## 9. Process Next Job

Repeat from step 1.

When the queue becomes empty:

```text
Exit successfully.
```

---

# CLI Interface

Use `clap`.

Required arguments:

```bash
queue_controller \
    --models-path ~/dll/llm/models \
    --project-root ~/dll/projects
```

## Arguments

### --models-path

Base directory containing GGUF models.

Example:

```bash
--models-path ~/dll/llm/models
```

### --project-root

Base directory containing synthesis projects.

Example:

```bash
--project-root ~/dll/projects
```

The controller uses the Redis `project` field to locate the correct project directory.

---

# Sbatch Template

Current reference configuration:

```bash
llama-server \
    --model <MODEL_PATH> \
    --seed <SEED> \
    --models-max 1 \
    -t 8 \
    -ngl 99 \
    -c 256000 \
    --host 0.0.0.0 \
    --cache-reuse 256 \
    --temp 0.6 \
    --top-p 0.95 \
    --top-k 20 \
    --min-p 0.0 \
    --presence-penalty 0.0 \
    --repeat-penalty 1.0
```

Only:

```text
MODEL_PATH
SEED
```

must be parameterized.

All other values remain fixed.

TODO:

```text
Make all llama.cpp parameters configurable through Redis.
```

---

# MCP Integration

The existing MCP server already communicates with Redis.

Reuse the existing implementation.

Requirements:

1. Instantiate the MCP server correctly.
2. Ensure Claude Code can connect to it.
3. Reuse existing Redis configuration where possible.
4. Avoid duplicating Redis access logic already implemented in the project.

---

# Non-Goals

Not required in this task:

- Parallel execution.
- Multiple simultaneous Slurm jobs.
- Multiple concurrent Claude Code sessions.
- Dynamic tuning of llama.cpp parameters.
- Queue persistence outside Redis.
- Distributed scheduling.

---

# Success Criteria

The task is complete when:

1. Jobs can be inserted into `cluster_runs`.
2. The controller pops jobs in priority order.
3. The controller generates and submits Slurm jobs.
4. The correct model is loaded.
5. Claude Code receives the prompt automatically.
6. Synthesis runs without manual intervention.
7. Jobs execute sequentially until the queue is empty.
8. Errors stop execution immediately.
