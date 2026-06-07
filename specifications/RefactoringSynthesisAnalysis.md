# Refactor Plan: Separate Data Collection From Visualization

## Goal

The current implementation mixes:

1. Redis data extraction
2. Statistical computation
3. Plot generation

This makes visualization harder to evolve and prevents leveraging the Python scientific ecosystem.

The goal is to split the system into two independent layers:

```text
Redis
  ↓
Rust Analysis Binary
  ↓
JSON Dataset
  ↓
Python Visualization Script
  ↓
SVG / PDF / PNG / HTML Reports
```

This architecture follows how many ML and research workflows are structured.

---

# High-Level Architecture

## Layer 1: Data Collection + Statistics (Rust)

Responsibilities:

- Connect to Redis
- Load synthesis results
- Compute statistics
- Detect outliers
- Produce a canonical dataset

Must NOT generate plots.

Outputs:

```text
results/
├── analysis.json
├── summary.json
├── summary.csv
└── report.md
```

---

## Layer 2: Visualization (Python)

Responsibilities:

- Read JSON dataset
- Generate publication-quality plots
- Generate optional HTML reports

Inputs:

```text
analysis.json
```

Outputs:

```text
results/
├── gas_boxplot.svg
├── gas_histogram.svg
├── gas_violin.svg
├── gas_scatter.svg
└── gas_boxplot.pdf
```

---

# Why This Separation Is Better

Current design:

```text
Rust
 ├─ Redis
 ├─ Statistics
 └─ Plotting
```

Problem:

- Plotting ecosystem in Rust is limited
- Boxplots require custom rendering
- Styling becomes difficult
- Adding new plot types is expensive

Proposed design:

```text
Rust
 ├─ Redis
 └─ Statistics

Python
 └─ Visualization
```

Benefits:

- Better plots
- Easier experimentation
- Easier maintenance
- Easier publication-quality output
- Easier future dashboards

---

# Rust Responsibilities

## New Binary

```text
src/bin/stats_export.rs
```

Purpose:

Generate a complete experimental dataset.

---

# Output Format

Generate:

```text
analysis.json
```

This becomes the canonical interface between Rust and Python.

---

# JSON Schema

```json
{
  "groups": [
    {
      "label": "baseline",
      "range": "1:50",
      "statistics": {
        "count": 50,
        "mean": 501234,
        "median": 487123,
        "std_dev": 81234,
        "variance": 6599000000,
        "q1": 420000,
        "q3": 560000,
        "iqr": 140000,
        "min": 300000,
        "max": 900000,
        "coefficient_of_variation": 0.16
      },
      "observations": [
        {
          "test_run_id": 1,
          "trial_id": 10,
          "gas": 450000
        }
      ],
      "outliers": [
        {
          "test_run_id": 38,
          "trial_id": 52,
          "gas": 1019344
        }
      ]
    }
  ]
}
```

---

# Rust Statistics

Rust remains responsible for:

- mean
- median
- variance
- std deviation
- quartiles
- IQR
- coefficient of variation
- outlier detection

Reason:

These are part of the experimental analysis.

Visualization should not reimplement statistics.

---

# Python Visualization Package

Create:

```text
scripts/
└── visualize_synthesis.py
```

Usage:

```bash
python scripts/visualize_synthesis.py \
    results/analysis.json \
    results/
```

---

# Python Dependencies

Use:

```text
pandas
seaborn
matplotlib
numpy
```

Optional:

```text
plotly
scienceplots
```

---

# Required Visualizations

## 1. Multi-Group Box Plot

Output:

```text
gas_boxplot.svg
```

X axis:

```text
group label
```

Examples:

```text
baseline
qwen
deepseek
```

Y axis:

```text
gas
```

Show:

- whiskers
- quartiles
- median
- outliers

Overlay:

- mean marker
- std deviation interval

Recommended:

```python
sns.boxplot(...)
sns.stripplot(...)
```

This allows seeing:

- distribution
- quartiles
- individual samples

simultaneously.

---

# 2. Violin Plot

Output:

```text
gas_violin.svg
```

Purpose:

Reveal distribution shape.

Example:

```python
sns.violinplot(...)
```

This often exposes multimodal distributions that boxplots hide.

---

# 3. Scatter Plot

Output:

```text
gas_scatter.svg
```

X axis:

```text
test_run_id
```

Y axis:

```text
gas
```

Colored by:

```text
group
```

Purpose:

Visualize evolution of synthesis quality.

---

# 4. Histogram

Output:

```text
gas_histogram.svg
```

Options:

- one subplot per group
- overlay distributions

Preferred:

one subplot per group.

---

# 5. ECDF Plot

Output:

```text
gas_ecdf.svg
```

Purpose:

Compare distributions directly.

Example:

```python
sns.ecdfplot(...)
```

This is extremely useful for comparing synthesis quality.

Often more informative than histograms.

---

# Optional Future Visualizations

## Pairwise Comparison

```text
gas_pairplot.svg
```

## CDF

```text
gas_cdf.svg
```

## Density Plot

```text
gas_kde.svg
```

## Pareto Frontier

Useful if future metrics include:

- gas
- tokens
- runtime

---

# Report Generation

Rust continues generating:

```text
report.md
```

Python only generates figures.

Reason:

The report contains experiment-specific interpretation and statistics.

Plots are simply attached assets.

---

# Future Evolution

The architecture should support:

```text
Redis
 ↓
Rust Export
 ↓
analysis.json
 ↓
Python Visualization
 ↓
SVG/PDF/HTML
```

without requiring modifications to the Redis layer.

Future metrics can simply be added to:

```json
{
  "gas": ...,
  "token_usage": ...,
  "runtime_seconds": ...,
  "iterations": ...
}
```

and automatically become available for visualization.

---

# Acceptance Criteria

The refactor is complete when:

1. Rust no longer generates plots.
2. Rust exports a canonical analysis.json file.
3. All statistics are computed in Rust.
4. Python consumes analysis.json.
5. Python generates publication-quality SVG plots.
6. Multi-group boxplots work with arbitrary `--range` definitions.
7. Violin plots and ECDF plots are available.
8. Future visualizations can be added without touching Redis code.
