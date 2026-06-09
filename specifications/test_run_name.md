# Task: Add Model Name Support to Test Runs

## Objective

Persist the model name associated with a synthesis run inside the Redis `test_run` record and ensure this information is propagated from `queue_controller` to `mcp_synth`.

---

## Background

Currently, a test run is stored in Redis under:

```text
test_run:{id}
```

and contains metadata such as:

```text
{
    project_id,
    compilation_passed,
    compilation_not_passed,
    created_at
}
```

We want to extend this structure so that the model used for the synthesis can also be stored when available.

---

## Requirements

### 1. Extend the TestRun model

Add an optional `model_name` field to the `TestRun` data structure and any related DTOs/interfaces involved in test run creation and retrieval.

Requirements:

- `model_name` must be optional.
- Existing code paths must continue to work without modification.
- Existing test runs without a model name must remain valid.

---

### 2. Persist model_name in Redis

Update the Redis implementation so that `test_run:{id}` can store a new field:

```text
model_name
```

Example:

```text
test_run:{id} -> {
    project_id,
    compilation_passed,
    compilation_not_passed,
    model_name,
    created_at
}
```

Requirements:

- Store `model_name` only when provided.
- Reading older records that do not contain `model_name` must not fail.

---

### 3. Update mcp_synth

Add support for an optional CLI argument:

```bash
--model-name <MODEL_NAME>
```

Requirements:

- Extend the CLI argument parsing.
- Propagate the value through the application layers until test run creation.
- When creating a test run, persist the model name if provided.
- Behaviour must remain unchanged when the argument is omitted.

Example:

```bash
mcp_synth \
  --cwd /path/to/project \
  --project my-project \
  --model-name qwen3-solidity-27B-Q6_K.gguf
```

---

### 4. Update queue_controller

`queue_controller` already knows which model is being used for a queued synthesis job.

Modify it so that when it launches `mcp_synth`, it forwards the model name using the new CLI argument:

```bash
--model-name <MODEL_NAME>
```

Requirements:

- Reuse the model identifier already associated with the queue job.
- Ensure the exact model used for the synthesis is recorded in the corresponding test run.

---

### 5. Backward Compatibility

The implementation must be fully backward compatible.

Requirements:

- Existing Redis data must remain valid.
- Existing test runs without `model_name` must load correctly.
- Existing workflows that do not provide a model name must continue to work unchanged.

---

## Testing

Add or update tests to verify:

1. A test run can be created without a model name.
2. A test run can be created with a model name.
3. The Redis backend correctly persists and retrieves `model_name`.
4. Older Redis records without `model_name` are handled correctly.
5. `mcp_synth` correctly propagates `--model-name` to test run creation.
6. `queue_controller` passes the model name to `mcp_synth`.

---

## Acceptance Criteria

- `mcp_synth` accepts an optional `--model-name` argument.
- `TestRun` supports an optional `model_name` field.
- Redis stores `model_name` when provided.
- Missing `model_name` values do not cause errors.
- `queue_controller` forwards the selected model to `mcp_synth`.
- Existing functionality remains unchanged.
- All relevant tests pass.
