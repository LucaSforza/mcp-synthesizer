# Task: Ensure Full Experiment Reproducibility

## Background

Currently `populate_queue` creates `N` jobs and assigns a single seed to each job.

That seed is passed to `llama-server`, which makes model generation reproducible.

However, the overall synthesis pipeline is still **not fully reproducible** because Foundry fuzzing is not seeded. As a result, rerunning the same job may produce different fuzzing outcomes.

The goal of this task is to make the entire experiment reproducible from a single job seed.

---

## Required Changes

### 1. Derive Two Independent Seeds Per Job

`populate_queue` currently generates a single seed `S` for each test.

For every generated test:

- Let `S` be the job seed.
- Deterministically derive two independent seeds:
  - `S1` → used for `llama-server`
  - `S2` → used for fuzzing reproducibility

The derivation must be deterministic so that the same job seed always produces the same `(S1, S2)` pair.

---

### 2. Pass the Fuzz Seed to MCP Synth

Add a new optional parameter to `mcp_synth`:

```rust
fuzz_seed: Option<u64>
```

`queue_controller` should pass `S2` when launching `mcp_synth`.

---

### 3. Create a Deterministic RNG

When `fuzz_seed` is provided:

- Initialize a deterministic RNG from `S2`.
- The RNG must remain alive for the entire synthesis session.
- Do not recreate the RNG before every fuzzing invocation.

Example:

```rust
let rng = ChaCha8Rng::seed_from_u64(S2);
```

Any deterministic RNG already used in the codebase is acceptable.

---

### 4. Seed Every Forge Fuzzing Run

Every time fuzzing is executed, obtain a new seed from the RNG and pass it to Foundry using:

```bash
forge ... --fuzz-seed <generated_seed>
```

This applies to every fuzzing invocation performed by the synthesis pipeline.

Example:

```rust
let forge_seed = rng.next_u64();
```

then:

```bash
forge test --fuzz-seed <forge_seed>
```

---

## Expected Result

Given the same original job seed `S`:

- `llama-server` receives the same seed (`S1`)
- the fuzzing RNG is initialized with the same seed (`S2`)
- every generated Foundry fuzz seed is reproduced in the same order

Therefore the complete synthesis experiment becomes reproducible end-to-end.
