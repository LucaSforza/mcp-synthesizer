#!/usr/bin/env python3
"""
Visualize synthesis experiment data from analysis.json.

Usage:
    python scripts/visualize_synthesis.py results/analysis.json results/

Outputs (in <output_dir>):
    gas_boxplot.svg      Box + strip plot with mean/std-dev overlay
    gas_violin.svg       Violin plot for distribution shape
    gas_scatter.svg      Scatter plot colored by group
    gas_histogram.svg    Faceted histogram, one subplot per group
    gas_ecdf.svg         Empirical CDF for direct group comparison
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


# ---------------------------------------------------------------------------
# Data loading
# ---------------------------------------------------------------------------

def load_analysis(path: str) -> dict:
    """Load the analysis.json file."""
    with open(path) as f:
        return json.load(f)


def build_dataframe(analysis: dict) -> pd.DataFrame:
    """Flatten groups into a DataFrame with columns: group, test_run_id, trial_id, gas."""
    rows = []
    for group in analysis["groups"]:
        for obs in group["observations"]:
            rows.append({
                "group": group["label"],
                "test_run_id": obs["test_run_id"],
                "trial_id": obs["trial_id"],
                "gas": obs["gas"],
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
    ]

    for path in paths:
        print(f"[DEBUG] Saved {path}")


if __name__ == "__main__":
    main()
