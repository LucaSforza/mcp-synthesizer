# Task Specification: Queue Population Utility

## Goal

Implement a new binary responsible for populating the synthesis priority queue stored in Redis.

This binary does **not** execute synthesis jobs.

Its only responsibility is to generate a batch of synthesis requests and enqueue them for later execution by the queue controller.

---

# Motivation

Creating synthesis jobs manually is error-prone and tedious.

Given:

- a model name
- an initial seed
- a project
- a prompt
- a number of iterations

the utility should automatically:

1. Generate a deterministic sequence of random seeds.
2. Create one Redis job hash per generated seed.
3. Insert each job into the synthesis priority queue.
4. Assign priorities in insertion order.

This allows large synthesis campaigns to be scheduled with a single command.

---

# CLI Interface

Use `clap`.

Example:

```bash
populate_queue \
    --model qwen3-solidity-27B-Q6_K.gguf \
    --seed 42 \
    --project my-project \
    --prompt-file prompt.md \
    --iterations 100
```

---

## Arguments

### --model

Model filename identifier.

Example:

```bash
--model qwen3-solidity-27B-Q6_K.gguf
```

Stored in Redis and later used by the queue controller to locate the model file.

---

### --seed

Initial seed used to initialize the RNG.

Example:

```bash
--seed 42
```

The same input seed must always generate the same sequence of job seeds.

---

### --project

Target synthesis project.

Example:

```bash
--project my-project
```

Stored in Redis unchanged.

---

### --prompt-file

Path to a file containing the synthesis prompt.

Example:

```bash
--prompt-file prompt.md
```

The entire file contents should be loaded and stored in Redis.

This avoids command-line escaping issues for large prompts.

---

### --iterations

Number of jobs to generate.

Example:

```bash
--iterations 100
```

Must be strictly greater than zero.

---

# Seed Generation

Initialize a deterministic random number generator using the provided seed.

Example:

```text
input seed = 42
```

Generate:

```text
iterations
```

random seeds.

Pseudo-code:

```text
rng = SeedableRng(seed)

for i in 1..=iterations:
    generated_seed = rng.next_u64()
```

Requirements:

- Deterministic.
- Reproducible.
- Same input seed must always generate the same sequence.
- Different input seeds should generate different sequences.

Recommended implementation:

```rust
rand_chacha::ChaCha8Rng
```

or equivalent deterministic RNG.

---

# Redis Data Model

Full key reference in [RedisDataModel.md](RedisDataModel.md). `populate_queue` writes two key types:

### Job Hash

```
key: {model_name}:{i}
type: Hash
fields: seed, project, prompt
```

`model_name` is **not** stored in the hash — it lives in the key. Queue controller extracts it via `rsplitn`.

Example:

```
HSET qwen3-solidity-27B-Q6_K.gguf:1 seed "183746192" project "my-project" prompt "..."
```

### Queue Entry

```
key: cluster_runs
type: Sorted Set
member: {model_name}:{i}
score: i (iteration index, higher = higher priority)
```

Example:

```
ZADD cluster_runs 1 qwen3-solidity-27B-Q6_K.gguf:1
```

---

# Complete Algorithm

Pseudo-code:

```text
load prompt

rng = SeedableRng(user_seed)

for i in 1..=iterations:

    generated_seed = rng.next_u64()

    job_key = "{model_name}:{i}"

    HSET job_key {
        seed = generated_seed
        project = project
        prompt = prompt
    }

    ZADD cluster_runs i job_key
```

---

# Validation

The binary must validate:

## Model

Must not be empty.

---

## Project

Must not be empty.

---

## Prompt

Prompt file must exist.

Prompt contents must not be empty.

---

## Iterations

Must satisfy:

```text
iterations > 0
```

---

# Error Handling

Fail immediately if:

- Redis connection fails.
- Prompt file cannot be read.
- Redis hash creation fails.
- Queue insertion fails.
- Invalid CLI arguments.

Do not partially continue after an unrecoverable error.

Return a descriptive error message.

---

# Redis Reuse

Reuse the existing Redis infrastructure already present in the project.

Avoid introducing a separate Redis client abstraction unless necessary.

Prefer following existing patterns already used by:

```text
src/db/redis.rs
```

---

# Future Extensions (TODO)

Not required for this task.

## Custom Priority Strategy

Allow specifying:

```bash
--priority-offset
```

to enqueue jobs at different priority ranges.

---

## Custom Metadata

Store additional fields:

```text
temperature
top_p
top_k
```

for future llama.cpp parameterization.

---

## UUID-based Job IDs

Current implementation uses:

```text
{model_name}:{i}
```

Future versions may replace this with:

```text
{model_name}:{uuid}
```

to avoid collisions between multiple queue population runs.

---

# Success Criteria

The task is complete when:

1. The user can create N synthesis jobs with a single command.
2. A deterministic sequence of seeds is generated.
3. One Redis hash is created per generated seed.
4. Each hash contains:
	   - seed
	   - project
	   - prompt
	   (model_name is in key `{model}:{i}`, not in hash)
5. Each job is inserted into `cluster_runs`.
6. Queue priority equals the iteration index.
7. The queue controller can immediately consume the generated jobs.
