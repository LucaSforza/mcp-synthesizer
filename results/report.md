# Experimental Synthesis Analysis

## Groups

- qwen3.6Sol (test runs 82-91)
- qwen3.6Simple (test runs 95-104)
- deepseek-flash (test runs 114-123)

## Group Statistics

| Group | Count | Mean | Median | Std Dev | CV | Min | Max | Q1 | Q3 | IQR |
|-------|-------|------|--------|---------|----|-----|-----|----|-----|-----|
| qwen3.6Sol | 10 | 651056 | 647338 | 268972 | 41.3% | 246800 | 1071436 | 428051 | 832488 | 404437 |
| qwen3.6Simple | 9 | 695392 | 522255 | 260559 | 37.5% | 411401 | 1060495 | 477252 | 933848 | 456596 |
| deepseek-flash | 10 | 897526 | 984877 | 241356 | 26.9% | 426010 | 1122455 | 942533 | 1030885 | 88352 |

## Outliers

### qwen3.6Sol

No outliers detected.

### qwen3.6Simple

No outliers detected.

### deepseek-flash

| Test Run ID | Trial ID | Gas |
|-------------|----------|-----|
| 115 | 252 | 426010 |
| 119 | 260 | 428502 |

## Interpretation

- **Best mean gas:** qwen3.6Sol (651056)
- **Best median gas:** qwen3.6Simple (522255)
- **Lowest variance:** deepseek-flash (58252518341)
- **Lowest CV (most stable):** deepseek-flash (26.9%)
- **Most gas-efficient:** qwen3.6Sol (mean 651056)

