# Experimental Synthesis Analysis Binary

## Goal

Implement a new binary dedicated to statistical analysis of synthesis experiments stored in Redis.

The primary use case is comparing **multiple groups of test runs**.

Each group represents a range of test runs and will become a separate box plot in the final visualization.

Example:

```bash
cargo run --bin stats_plot -- \
  --redis-url redis://localhost:6379 \
  --range baseline=1:50 \
  --range qwen=51:100 \
  --range deepseek=101:150 \
  --output results/
```

This should generate a comparison between three experiment groups:

```text
baseline
qwen
deepseek
```

Each group is represented as a separate box plot.

The goal is to visually compare gas distributions between different experimental conditions, prompts, models, datasets, or synthesis strategies.

---

# Binary

Create:

```text
src/bin/stats_plot.rs
```

Create reusable modules:

```text
src/stats/
├── mod.rs
├── loader.rs
├── statistics.rs
├── plots.rs
├── report.rs
├── parser.rs
└── types.rs
```

Follow existing project architecture and coding style.

---

# CLI

Required:

```bash
--redis-url <url>
--output <directory>
```

One or more ranges:

```bash
--range <label>=<start:end>
```

Examples:

```bash
--range baseline=1:50
--range qwen=51:100
--range deepseek=101:150
```

The tool must support an arbitrary number of ranges.

Examples:

```bash
--range experiment_a=37:46
```

```bash
--range experiment_a=37:46 \
--range experiment_b=47:56
```

```bash
--range prompt_v1=1:100 \
--range prompt_v2=101:200 \
--range prompt_v3=201:300
```

---

# Data Model

Each range becomes a dataset.

Example:

```bash
--range qwen=37:46
```

Produces:

```rust
pub struct ExperimentGroup {
    pub label: String,
    pub observations: Vec<GasObservation>,
}
```

Where:

```rust
pub struct GasObservation {
    pub test_run_id: u64,
    pub trial_id: u64,
    pub gas: u64,
}
```

---

# Redis Extraction Logic

For each test run:

1. Read:

```text
synthesis_trial:by_test_run:{id}
```

2. Find the final:

```text
result_type = succeeded_full
```

3. Extract:

```text
gas_of_implementation
```

4. Store as a GasObservation.

Ignore all failed runs.

Only succeeded_full participates in analysis.

---

# Statistics

Compute statistics separately for each group.

Implement:

```rust
pub struct GroupStatistics {
    pub label: String,

    pub count: usize,

    pub mean: f64,
    pub median: f64,

    pub variance: f64,
    pub std_dev: f64,

    pub min: u64,
    pub max: u64,

    pub q1: f64,
    pub q3: f64,

    pub iqr: f64,

    pub coefficient_of_variation: f64,

    pub outliers: Vec<Outlier>,
}
```

Definitions:

## Mean

Arithmetic average.

## Variance

Population variance.

## Standard deviation

Population standard deviation.

## Quartiles

25th percentile and 75th percentile.

## IQR

```text
IQR = Q3 - Q1
```

## Coefficient of variation

```text
CV = std_dev / mean
```

Express as percentage in reports.

---

# Outlier Detection

Use standard box plot rules.

Definitions:

```text
lower_bound = Q1 - 1.5 * IQR

upper_bound = Q3 + 1.5 * IQR
```

Any value outside the interval is an outlier.

Represent:

```rust
pub struct Outlier {
    pub test_run_id: u64,
    pub trial_id: u64,
    pub gas: u64,
}
```

---

# Main Visualization

## Multi-Group Box Plot

File:

```text
gas_boxplot.svg
```

This is the primary output of the tool.

### X Axis

Experiment groups.

Example:

```text
baseline
qwen
deepseek
```

Each label corresponds to one:

```bash
--range label=start:end
```

argument.

### Y Axis

```text
gas_of_implementation
```

### For each group display

Standard box plot:

- lower whisker
- Q1
- median
- Q3
- upper whisker

Outliers as points.

Additionally overlay:

- mean
- mean - std_dev
- mean + std_dev

This is important because the standard box plot does not directly visualize standard deviation.

Suggested styling:

```text
box           blue
median        black
mean          red
std-dev       dashed red
outliers      black points
```

---

# Example Visualization

If the user runs:

```bash
--range baseline=1:50
--range qwen=51:100
--range deepseek=101:150
```

The chart should resemble:

```text
             gas

1.2M ┤

1.0M ┤        ○

800k ┤     ┌─────┐
     │     │     │
600k ┤ ┌───┘     └───┐
     │ │             │
400k ┤ │             │
     │ │             │
200k ┤ └─────────────┘

      baseline  qwen  deepseek
```

Real SVG generated via Plotters.

---

# Secondary Visualizations

Generate additional supporting plots.

---

## Scatter Plot

File:

```text
gas_scatter.svg
```

Purpose:

Show individual runs.

X axis:

```text
test_run_id
```

Y axis:

```text
gas
```

Color/group by experiment group.

Each point represents one synthesis result.

---

## Histogram

File:

```text
gas_histogram.svg
```

Purpose:

Visualize gas distribution.

Can either:

### Option A

Generate one histogram per group.

or

### Option B

Overlay histograms.

Option A is preferred for readability.

---

# JSON Output

Generate:

```text
summary.json
```

Structure:

```json
{
  "groups": [
    {
      "label": "baseline",
      "count": 50,
      "mean": 500000,
      "std_dev": 100000,
      "median": 480000,
      "q1": 420000,
      "q3": 580000,
      "iqr": 160000,
      "cv": 0.20
    }
  ]
}
```

---

# CSV Output

Generate:

```text
summary.csv
```

One row per observation.

Example:

```csv
group,test_run_id,trial_id,gas
baseline,37,50,709942
baseline,38,52,1019344
...
```

---

# Markdown Report

Generate:

```text
report.md
```

Structure:

```markdown
# Experimental Synthesis Analysis

## Groups

- baseline (1-50)
- qwen (51-100)
- deepseek (101-150)

## Group Statistics

| Group | Mean | Median | Std Dev | CV |
|---------|---------|---------|---------|---------|
| baseline | ... | ... | ... | ... |
| qwen | ... | ... | ... | ... |
| deepseek | ... | ... | ... | ... |

## Outliers

### baseline

...

### qwen

...

### deepseek

...

## Interpretation

Group with lowest mean gas:
...

Group with lowest median gas:
...

Group with lowest variance:
...

Group with lowest coefficient of variation:
...

Most stable group:
...

Most gas-efficient group:
...
```

---

# Redis Safety

Requirements:

- Read-only only.
- No writes.
- No schema modifications.
- No migrations.
- No FLUSHDB.
- No mutations.

---

# Tests

Add unit tests for:

- mean
- variance
- standard deviation
- median
- quartiles
- IQR
- coefficient of variation
- outlier detection
- range parser
- group aggregation

Tests should not require Redis.

Use deterministic datasets.

---

# Acceptance Criteria

The binary is complete when:

1. Multiple experiment groups can be specified with repeated `--range`.
2. Each group is loaded independently from Redis.
3. Statistics are computed per group.
4. Outliers are detected per group.
5. A multi-group box plot is generated.
6. Mean and standard deviation overlays are visible on the box plot.
7. Scatter plot and histogram are generated.
8. JSON, CSV and Markdown reports are generated.
9. The tool is completely read-only.
10. Unit tests cover all statistical calculations.
