#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.10"
# dependencies = [
#   "pandas>=1.5",
#   "seaborn>=0.12",
#   "matplotlib>=3.7",
#   "numpy>=1.24",
# ]
# ///
"""
Visualize synthesis experiment data from analysis.json.

Usage:
    uv run scripts/visualize_synthesis.py results/analysis.json results/

Outputs (in <output_dir>):
    gas_boxplot.svg      Box + strip plot with mean/std-dev overlay
    gas_violin.svg       Violin plot for distribution shape
    gas_scatter.svg      Scatter plot colored by group
    gas_histogram.svg    Faceted histogram, one subplot per group
    gas_ecdf.svg         Empirical CDF for direct group comparison
    gas_vs_tokens.svg    Gas vs total tokens with knee point
    gas_vs_cost.svg      Gas vs synthesis cost with knee point
    gas_cost_pareto.svg    Pareto frontier (colorblind-friendly, shape+color)
    gas_tokens_pareto.svg  Token-based Pareto frontier (colorblind-friendly)
"""

import json
import os
import sys

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
import seaborn as sns

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

# Shape cycle so groups distinguishable without color (colorblind-friendly).
GROUP_MARKERS = ["o", "s", "^", "D", "v", "p", "*", "h"]


# ---------------------------------------------------------------------------
# Data loading
# ---------------------------------------------------------------------------

def load_analysis(path: str) -> dict:
    """Load the analysis.json file."""
    with open(path) as f:
        return json.load(f)


def build_dataframe(analysis: dict) -> pd.DataFrame:
    """Flatten groups into a DataFrame with columns for gas, tokens, cost, and metadata."""
    rows = []
    for group in analysis["groups"]:
        for obs in group["observations"]:
            rows.append({
                "group": group["label"],
                "test_run_id": obs["test_run_id"],
                "trial_id": obs["trial_id"],
                "gas": obs["gas"],
                "total_tokens": obs.get("total_tokens", 0),
                "cost_of_synthesis_usd": obs.get("cost_of_synthesis_usd", 0.0),
                "model_name": obs.get("model_name", ""),
                "project_id": obs.get("project_id", 0),
            })
    return pd.DataFrame(rows)


# ---------------------------------------------------------------------------
# Plot functions
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
# New plots: cost / token analysis
# ---------------------------------------------------------------------------

def plot_gas_vs_tokens(analysis: dict, df: pd.DataFrame, output_dir: str) -> str:
    """Scatter plot of gas vs total tokens with knee point highlighted."""
    fig, ax = plt.subplots(figsize=(10, 6))

    plot_df = df[df["total_tokens"] > 0].copy()

    for group in sorted(plot_df["group"].unique()):
        subset = plot_df[plot_df["group"] == group]
        ax.scatter(
            subset["total_tokens"], subset["gas"],
            label=group, s=SCATTER_S, alpha=SCATTER_ALPHA,
            edgecolors="black", linewidth=0.5,
        )

    # Knee point from precomputed analysis.
    knee = analysis.get("knee_analysis", {}).get("gas_vs_tokens")
    if knee:
        ax.scatter(
            knee["knee_x"], knee["knee_y"],
            color="red", s=200, marker="D", zorder=10,
            label=f"Knee ({knee['knee_x']:.0f}, {knee['knee_y']:.0f})",
        )

    ax.set_xlabel("Total Tokens (Input + Output)")
    ax.set_ylabel("Gas")
    ax.set_title("Gas vs Total Tokens", fontsize=16, fontweight="bold")
    ax.legend(title="Group")

    path = os.path.join(output_dir, "gas_vs_tokens.svg")
    fig.savefig(path, bbox_inches="tight")
    plt.close(fig)
    return path


def plot_gas_vs_cost(analysis: dict, df: pd.DataFrame, output_dir: str) -> str:
    """Scatter plot of gas vs synthesis cost with knee point highlighted."""
    fig, ax = plt.subplots(figsize=(10, 6))

    plot_df = df[df["cost_of_synthesis_usd"] > 0].copy()

    for group in sorted(plot_df["group"].unique()):
        subset = plot_df[plot_df["group"] == group]
        ax.scatter(
            subset["cost_of_synthesis_usd"], subset["gas"],
            label=group, s=SCATTER_S, alpha=SCATTER_ALPHA,
            edgecolors="black", linewidth=0.5,
        )

    knee = analysis.get("knee_analysis", {}).get("gas_vs_cost")
    if knee:
        ax.scatter(
            knee["knee_x"], knee["knee_y"],
            color="red", s=200, marker="D", zorder=10,
            label=f"Knee (${knee['knee_x']:.4f}, {knee['knee_y']:.0f})",
        )

    ax.set_xlabel("Synthesis Cost (USD)")
    ax.set_ylabel("Gas")
    ax.set_title("Gas vs Synthesis Cost", fontsize=16, fontweight="bold")
    ax.legend(title="Group")

    path = os.path.join(output_dir, "gas_vs_cost.svg")
    fig.savefig(path, bbox_inches="tight")
    plt.close(fig)
    return path


def _pareto_plot(
    analysis: dict, df: pd.DataFrame, output_dir: str,
    frontier_key: str, x_col: str, x_label: str, filename: str, title: str,
) -> str:
    """Shared Pareto plot — each group gets a distinct marker shape.

    Dominated points use group shape at normal size.  Frontier points use
    the same group shape but enlarged with a red edge so their status is
    visible even without color.
    """
    fig, ax = plt.subplots(figsize=(10, 6))

    plot_df = df[df[x_col] > 0].copy()
    groups_sorted = sorted(plot_df["group"].unique())

    # Map each group to a distinct marker shape.
    marker_map = {g: GROUP_MARKERS[i % len(GROUP_MARKERS)] for i, g in enumerate(groups_sorted)}

    # Frontier ID set from analysis.json.
    pareto = analysis.get("pareto_frontier", {}).get(frontier_key, [])
    frontier_ids = {(p["test_run_id"], p["trial_id"]) for p in pareto}
    plot_df["is_frontier"] = plot_df.apply(
        lambda r: (r["test_run_id"], r["trial_id"]) in frontier_ids, axis=1,
    )

    # Plot each group — dominated points at normal size.
    for group in groups_sorted:
        subset = plot_df[(plot_df["group"] == group) & (~plot_df["is_frontier"])]
        if subset.empty:
            continue
        ax.scatter(
            subset[x_col], subset["gas"],
            marker=marker_map[group], label=group,
            s=SCATTER_S, alpha=SCATTER_ALPHA,
            edgecolors="black", linewidth=0.5,
        )

    # Plot frontier points — same group shape, enlarged, red edge.
    frontier_df = plot_df[plot_df["is_frontier"]]
    for group in frontier_df["group"].unique():
        subset = frontier_df[frontier_df["group"] == group]
        if subset.empty:
            continue
        ax.scatter(
            subset[x_col], subset["gas"],
            marker=marker_map[group],
            s=140, zorder=6,
            edgecolors="red", linewidth=2.5,
            facecolors="none",
            label=f"{group} (Pareto)",
        )

    # Frontier connecting line.
    if pareto:
        px = [p[x_col] for p in pareto]
        py = [p["gas"] for p in pareto]
        ax.plot(
            px, py,
            color="red", linewidth=3.0, linestyle="--",
            label=f"Pareto Frontier ({len(pareto)} obs)",
        )

    ax.set_xlabel(x_label)
    ax.set_ylabel("Gas")
    ax.set_title(title, fontsize=16, fontweight="bold")
    ax.legend(loc="best")

    path = os.path.join(output_dir, filename)
    fig.savefig(path, bbox_inches="tight")
    plt.close(fig)
    return path


def plot_gas_cost_pareto(analysis: dict, df: pd.DataFrame, output_dir: str) -> str:
    """Scatter plot with cost-based Pareto frontier — colorblind-friendly."""
    return _pareto_plot(
        analysis, df, output_dir,
        frontier_key="cost",
        x_col="cost_of_synthesis_usd",
        x_label="Synthesis Cost (USD)",
        filename="gas_cost_pareto.svg",
        title="Gas-Cost Pareto Frontier",
    )


def plot_gas_tokens_pareto(analysis: dict, df: pd.DataFrame, output_dir: str) -> str:
    """Scatter plot with token-based Pareto frontier — colorblind-friendly."""
    return _pareto_plot(
        analysis, df, output_dir,
        frontier_key="tokens",
        x_col="total_tokens",
        x_label="Total Tokens (Input + Output)",
        filename="gas_tokens_pareto.svg",
        title="Gas-Tokens Pareto Frontier",
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

    paths = [
        plot_boxplot(df, stats, output_dir),
        plot_violin(df, output_dir),
        plot_scatter(df, output_dir),
        plot_histogram(df, output_dir),
        plot_ecdf(df, output_dir),
        plot_gas_vs_tokens(analysis, df, output_dir),
        plot_gas_vs_cost(analysis, df, output_dir),
        plot_gas_cost_pareto(analysis, df, output_dir),
        plot_gas_tokens_pareto(analysis, df, output_dir),
    ]

    for path in paths:
        print(f"[DEBUG] Saved {path}")


if __name__ == "__main__":
    main()
