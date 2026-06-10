#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.10"
# dependencies = [
#   "pandas>=1.5",
#   "seaborn>=0.12",
#   "matplotlib>=3.7",
#   "numpy>=1.24",
#   "scikit-learn>=1.3",
# ]
# ///
"""
Visualize synthesis experiment data from analysis.json.

Usage:
    uv run scripts/visualize_synthesis.py results/analysis.json results/

Outputs (in <output_dir>):
    gas_boxplot.svg         Box + strip plot with mean/std-dev overlay
    gas_violin.svg          Violin plot for distribution shape
    gas_scatter.svg         Scatter plot colored by group
    gas_histogram.svg       Faceted histogram, one subplot per group
    gas_ecdf.svg            Empirical CDF for direct group comparison
    gas_elbow.svg           Elbow method (K-means inertia vs k) on tokens-vs-gas
    cost_vs_gas.svg         Cost vs gas scatter with per-model regression
    tokens_vs_gas.svg       Total tokens vs gas scatter with per-model regression
    regression_summary.csv  Per-model regression statistics (optional)
"""

import csv
import json
import os
import sys
from itertools import cycle

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
import seaborn as sns
from sklearn.cluster import KMeans
from sklearn.linear_model import LinearRegression
from sklearn.metrics import r2_score

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

# Seaborn style
sns.set_theme(style="whitegrid", palette="muted", font_scale=1.1)

BOXPLOT_COLOR = "lightblue"
MEAN_COLOR = "red"
MEAN_STD_CAPSIZE = 5
MEAN_STD_CAPTHICK = 1.5
SCATTER_ALPHA = 0.8
SCATTER_S = 50

# B&W-safe visual encoding: grayscale palette for up to 5 models, cycling
GRAYSCALE_PALETTE = ["black", "dimgray", "gray", "darkgray", "silver"]

MARKER_SHAPES = ["o", "s", "^", "D", "P", "*", "v", "<", ">", "p", "h", "X"]

LINE_STYLES = ["-", "--", "-.", ":", (0, (3, 1, 1, 1)), (0, (5, 1)), (0, (3, 1))]

ELBOW_MAX_K = 15


# ---------------------------------------------------------------------------
# Data loading
# ---------------------------------------------------------------------------


def load_analysis(path: str) -> dict:
    """Load the analysis.json file."""
    with open(path) as f:
        return json.load(f)


def build_dataframe(analysis: dict) -> pd.DataFrame:
    """Flatten groups into a DataFrame with all observation fields."""
    rows = []
    for group in analysis["groups"]:
        for obs in group["observations"]:
            row = {
                "group": group["label"],
                "test_run_id": obs["test_run_id"],
                "trial_id": obs["trial_id"],
                "gas": obs["gas"],
                "synth_time_seconds": obs.get("synth_time_seconds"),
                "model_name": obs.get("model_name"),
                "cost_usd": obs.get("cost_usd"),
                "input_tokens": obs.get("input_tokens"),
                "output_tokens": obs.get("output_tokens"),
            }
            # Compute total_tokens when both are available.
            inp = row["input_tokens"]
            out = row["output_tokens"]
            if inp is not None and out is not None:
                row["total_tokens"] = inp + out
            else:
                row["total_tokens"] = None
            rows.append(row)
    return pd.DataFrame(rows)


# ---------------------------------------------------------------------------
# Visual encoding helpers
# ---------------------------------------------------------------------------


def assign_model_visuals(models: list[str]) -> dict:
    """Assign a (marker, color, linestyle) tuple to each model, cycling through
    B&W-safe alternatives."""
    n = len(models)
    marker_cycle = cycle(MARKER_SHAPES)
    color_cycle = cycle(GRAYSCALE_PALETTE)
    ls_cycle = cycle(LINE_STYLES)

    # Use deterministic seed so output is stable.
    rng = np.random.default_rng(42)
    assigned = {}
    # Shuffle assignment order for visual variety when models share prefixes.
    indices = list(range(n))
    rng.shuffle(indices)

    for idx in indices:
        assigned[models[idx]] = (
            next(marker_cycle),
            next(color_cycle),
            next(ls_cycle),
        )
    return assigned


def fit_regression(
    x: np.ndarray, y: np.ndarray
) -> tuple[float, float, float, object]:
    """Fit linear regression, return (slope, intercept, r_squared, model)."""
    mask = ~(np.isnan(x) | np.isnan(y))
    x_clean = x[mask].reshape(-1, 1)
    y_clean = y[mask]
    if len(x_clean) < 2:
        return 0.0, 0.0, 0.0, None
    reg = LinearRegression()
    reg.fit(x_clean, y_clean)
    y_pred = reg.predict(x_clean)
    r2 = r2_score(y_clean, y_pred)
    return float(reg.coef_[0]), float(reg.intercept_), r2, reg


# ---------------------------------------------------------------------------
# Original plot functions (group-based)
# ---------------------------------------------------------------------------


def plot_boxplot(df: pd.DataFrame, stats: list, output_dir: str) -> str:
    """Multi-group boxplot with strip overlay, mean markers, and std-dev intervals."""
    fig, ax = plt.subplots(figsize=(10, 6))

    sns.boxplot(data=df, x="group", y="gas", ax=ax, color=BOXPLOT_COLOR)
    sns.stripplot(
        data=df, x="group", y="gas", ax=ax,
        color="black", alpha=0.5, size=6, jitter=True,
    )

    # Overlay mean +- std_dev from statistics
    for i, s in enumerate(stats):
        mean = s["mean"]
        std = s["std_dev"]
        ax.scatter(i, mean, color=MEAN_COLOR, s=80, zorder=10, marker="D")
        ax.errorbar(
            i, mean, yerr=std, fmt="none",
            ecolor=MEAN_COLOR, capsize=MEAN_STD_CAPSIZE,
            capthick=MEAN_STD_CAPTHICK, zorder=9,
        )

    ax.set_title("Gas Distribution by Group", fontsize=16, fontweight="bold")
    ax.set_ylabel("Gas")
    ax.set_xlabel("")

    path = os.path.join(output_dir, "gas_boxplot.svg")
    fig.savefig(path, bbox_inches="tight")
    plt.close(fig)
    return path


def plot_violin(df: pd.DataFrame, output_dir: str) -> str:
    """Violin plot to reveal distribution shape (multimodality, skew)."""
    fig, ax = plt.subplots(figsize=(10, 6))

    sns.violinplot(data=df, x="group", y="gas", ax=ax, inner="quartile")

    ax.set_title("Gas Violin Plot", fontsize=16, fontweight="bold")
    ax.set_ylabel("Gas")
    ax.set_xlabel("")

    path = os.path.join(output_dir, "gas_violin.svg")
    fig.savefig(path, bbox_inches="tight")
    plt.close(fig)
    return path


def plot_scatter(df: pd.DataFrame, output_dir: str) -> str:
    """Scatter plot colored by group."""
    fig, ax = plt.subplots(figsize=(10, 6))

    for group in sorted(df["group"].unique()):
        subset = df[df["group"] == group]
        ax.scatter(
            subset["test_run_id"], subset["gas"],
            label=group, s=SCATTER_S, alpha=SCATTER_ALPHA,
            edgecolors="black", linewidth=0.5,
        )

    ax.set_xlabel("Test Run ID")
    ax.set_ylabel("Gas")
    ax.set_title("Gas by Test Run", fontsize=16, fontweight="bold")
    ax.legend(title="Group")

    path = os.path.join(output_dir, "gas_scatter.svg")
    fig.savefig(path, bbox_inches="tight")
    plt.close(fig)
    return path


def plot_histogram(df: pd.DataFrame, output_dir: str) -> str:
    """One histogram subplot per group."""
    groups = sorted(df["group"].unique())
    n = len(groups)

    fig, axes = plt.subplots(1, n, figsize=(5 * n, 4), squeeze=False)

    for ax, group in zip(axes[0], groups):
        subset = df[df["group"] == group]["gas"]
        ax.hist(subset, bins="auto", edgecolor="black", alpha=0.7, color="steelblue")
        ax.set_title(f"Histogram — {group}")
        ax.set_xlabel("Gas")
        ax.set_ylabel("Frequency")

    fig.tight_layout()
    path = os.path.join(output_dir, "gas_histogram.svg")
    fig.savefig(path, bbox_inches="tight")
    plt.close(fig)
    return path


def plot_ecdf(df: pd.DataFrame, output_dir: str) -> str:
    """Empirical CDF for direct distribution comparison across groups."""
    fig, ax = plt.subplots(figsize=(10, 6))

    sns.ecdfplot(data=df, x="gas", hue="group", ax=ax, linewidth=2)

    ax.set_title("Gas ECDF by Group", fontsize=16, fontweight="bold")
    ax.set_ylabel("ECDF")
    ax.set_xlabel("Gas")

    path = os.path.join(output_dir, "gas_ecdf.svg")
    fig.savefig(path, bbox_inches="tight")
    plt.close(fig)
    return path


# ---------------------------------------------------------------------------
# Elbow method: K-means inertia on (total_tokens, gas)
# ---------------------------------------------------------------------------


def plot_gas_elbow(df: pd.DataFrame, output_dir: str) -> str:
    """
    Elbow method for K-means clustering on the (total_tokens, gas) space.

    Plots inertia vs k. Helps determine the optimal number of clusters in the
    synthesis cost-quality space.
    """
    # Drop rows missing either field.
    valid = df.dropna(subset=["total_tokens", "gas"])
    if len(valid) < 2:
        print("[WARNING] Fewer than 2 complete (total_tokens, gas) observations"
              " — skipping elbow plot")
        return ""

    X = valid[["total_tokens", "gas"]].values.astype(np.float64)
    max_k = min(ELBOW_MAX_K, len(X) - 1)

    inertias = []
    for k in range(1, max_k + 1):
        km = KMeans(n_clusters=k, n_init=10, random_state=42)
        km.fit(X)
        inertias.append(km.inertia_)

    fig, ax = plt.subplots(figsize=(8, 5))
    ks = list(range(1, max_k + 1))
    ax.plot(ks, inertias, marker="o", color="black", linestyle="-", linewidth=2)

    # Annotate the "elbow" — point of max curvature (simplified: 2nd diff peak).
    if max_k >= 3:
        deltas = np.diff(inertias)
        delta2 = np.diff(deltas)
        elbow_k = int(np.argmax(delta2)) + 2  # +2 because double diff shifts by 2
        ax.axvline(x=elbow_k, color="dimgray", linestyle="--", alpha=0.7)
        ax.annotate(
            f"elbow ≈ k={elbow_k}",
            xy=(elbow_k, inertias[elbow_k - 1]),
            xytext=(elbow_k + 0.5, inertias[elbow_k - 1] * 1.05),
            fontsize=11,
            arrowprops=dict(arrowstyle="->", color="dimgray"),
        )

    ax.set_title("Elbow Method: Optimal Clusters (total_tokens, gas)",
                 fontsize=14, fontweight="bold")
    ax.set_xlabel("Number of clusters (k)")
    ax.set_ylabel("Inertia (within-cluster sum of squares)")
    ax.set_xticks(ks)

    path = os.path.join(output_dir, "gas_elbow.svg")
    fig.savefig(path, bbox_inches="tight")
    plt.close(fig)
    return path


# ---------------------------------------------------------------------------
# Multi-model correlation plots (B&W-safe)
# ---------------------------------------------------------------------------


def _build_regression_legend_label(
    model: str, n: int, slope: float, intercept: float, r2: float
) -> str:
    """Build a multi-line legend entry for one model."""
    return (
        f"{model}\n"
        f"  n = {n}\n"
        f"  y = {slope:.4f}x + {intercept:.1f}\n"
        f"  R² = {r2:.4f}"
    )


def _plot_regression_scatter(
    df: pd.DataFrame,
    x_col: str,
    y_col: str,
    xlabel: str,
    ylabel: str,
    title: str,
    filename: str,
    output_dir: str,
) -> str:
    """
    Generic scatter + per-model regression plot.

    Parameters
    ----------
    df : DataFrame — must contain model_name, x_col, y_col
    x_col, y_col : column names for axes
    xlabel, ylabel : axis labels
    title : plot title
    filename : output file name (e.g. "cost_vs_gas.svg")
    output_dir : output directory
    """
    valid = df.dropna(subset=[x_col, y_col, "model_name"])
    if valid.empty:
        print(f"[WARNING] No valid data for {filename} — skipping")
        return ""

    models = sorted(valid["model_name"].unique())
    visuals = assign_model_visuals(models)

    fig, ax = plt.subplots(figsize=(10, 7))

    regression_rows = []  # for CSV export

    for model in models:
        subset = valid[valid["model_name"] == model]
        x = subset[x_col].values.astype(np.float64)
        y = subset[y_col].values.astype(np.float64)
        marker, color, linestyle = visuals[model]

        # Scatter points.
        ax.scatter(
            x, y,
            marker=marker, color=color, s=SCATTER_S,
            alpha=SCATTER_ALPHA, edgecolors="black", linewidth=0.5,
            label=None,  # custom legend below
            zorder=3,
        )

        # Linear regression.
        slope, intercept, r2, reg = fit_regression(x, y)
        n = len(subset)
        regression_rows.append((model, n, slope, intercept, r2))

        if reg is not None and n >= 2:
            x_line = np.linspace(x.min(), x.max(), 200)
            y_line = reg.predict(x_line.reshape(-1, 1))
            ax.plot(
                x_line, y_line,
                color=color, linestyle=linestyle, linewidth=2,
                label=_build_regression_legend_label(model, n, slope, intercept, r2),
                zorder=4,
            )

    ax.set_xlabel(xlabel, fontsize=13)
    ax.set_ylabel(ylabel, fontsize=13)
    ax.set_title(title, fontsize=15, fontweight="bold")

    # Legend outside plot to avoid crowding.
    ax.legend(
        loc="upper left",
        bbox_to_anchor=(1.02, 1),
        frameon=True,
        fontsize=9,
        title="Model Regression",
    )
    fig.tight_layout(rect=(0, 0, 0.82, 1))

    path = os.path.join(output_dir, filename)
    fig.savefig(path, bbox_inches="tight")
    plt.close(fig)

    # Also save regression summary alongside plot.
    csv_path = os.path.join(output_dir, "regression_summary.csv")
    _append_regression_csv(csv_path, x_col, regression_rows)

    print(f"[DEBUG] Saved {path}")
    return path


def _append_regression_csv(
    path: str,
    predictor: str,
    rows: list[tuple[str, int, float, float, float]],
) -> None:
    """Append regression stats to the shared CSV. Creates header on first call."""
    header = ["model_name", "predictor", "samples", "slope", "intercept", "r_squared"]
    # Detect if file exists to write header only once.
    write_header = not os.path.exists(path)
    with open(path, "a", newline="") as f:
        writer = csv.writer(f)
        if write_header:
            writer.writerow(header)
        for model, n, slope, intercept, r2 in rows:
            writer.writerow([model, predictor, n, f"{slope:.6f}",
                             f"{intercept:.6f}", f"{r2:.6f}"])


def plot_cost_vs_gas(df: pd.DataFrame, output_dir: str) -> str:
    """Cost vs Gas scatter with per-model linear regression."""
    return _plot_regression_scatter(
        df=df,
        x_col="cost_usd",
        y_col="gas",
        xlabel="Synthesis Cost (USD)",
        ylabel="Gas (gas_of_implementation)",
        title="Gas Usage vs Synthesis Cost",
        filename="cost_vs_gas.svg",
        output_dir=output_dir,
    )


def plot_tokens_vs_gas(df: pd.DataFrame, output_dir: str) -> str:
    """Total tokens vs Gas scatter with per-model linear regression.

    This is the primary 'elbow' correlation plot: (input_tokens + output_tokens)
    on x-axis, gas on y-axis."""
    return _plot_regression_scatter(
        df=df,
        x_col="total_tokens",
        y_col="gas",
        xlabel="Total Tokens (input + output)",
        ylabel="Gas (gas_of_implementation)",
        title="Gas Usage vs Token Consumption",
        filename="tokens_vs_gas.svg",
        output_dir=output_dir,
    )


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> None:
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <analysis.json> <output_dir>", file=sys.stderr)
        sys.exit(1)

    input_path = sys.argv[1]
    output_dir = sys.argv[2]

    os.makedirs(output_dir, exist_ok=True)

    analysis = load_analysis(input_path)
    df = build_dataframe(analysis)
    stats = [g["statistics"] for g in analysis["groups"]]

    print(f"[DEBUG] Loaded {len(df)} observations across {len(stats)} groups")

    # Original plots
    paths = [
        plot_boxplot(df, stats, output_dir),
        plot_violin(df, output_dir),
        plot_scatter(df, output_dir),
        plot_histogram(df, output_dir),
        plot_ecdf(df, output_dir),
    ]

    # Elbow method (K-means on total_tokens, gas)
    elbow_path = plot_gas_elbow(df, output_dir)
    if elbow_path:
        paths.append(elbow_path)

    # Multi-model correlation plots
    paths.append(plot_cost_vs_gas(df, output_dir))
    paths.append(plot_tokens_vs_gas(df, output_dir))

    for path in paths:
        if path:
            print(f"[DEBUG] Saved {path}")


if __name__ == "__main__":
    main()
