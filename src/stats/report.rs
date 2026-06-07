use std::io::Write;

use anyhow::{Context, Result};

use crate::stats::statistics::compute_statistics;
use crate::stats::types::{ExperimentGroup, GroupStatistics};

/// Generate all report outputs: analysis.json, summary.json, CSV, and Markdown.
pub fn generate_all_reports(
    groups: &[ExperimentGroup],
    output_dir: &std::path::Path,
) -> Result<()> {
    let stats: Vec<GroupStatistics> = groups.iter().map(compute_statistics).collect();

    generate_analysis_json(groups, &stats, output_dir)?;
    generate_json(&stats, output_dir)?;
    generate_csv(groups, output_dir)?;
    generate_markdown(groups, &stats, output_dir)?;

    Ok(())
}

/// Generate `analysis.json` — canonical dataset for Python visualization.
fn generate_analysis_json(
    groups: &[ExperimentGroup],
    stats: &[GroupStatistics],
    output_dir: &std::path::Path,
) -> Result<()> {
    use serde::Serialize;

    let path = output_dir.join("analysis.json");

    #[derive(Serialize)]
    struct AnalysisOutlier {
        test_run_id: u64,
        trial_id: u64,
        gas: u64,
    }

    #[derive(Serialize)]
    struct AnalysisObservation {
        test_run_id: u64,
        trial_id: u64,
        gas: u64,
    }

    #[derive(Serialize)]
    struct AnalysisStats {
        count: usize,
        mean: f64,
        median: f64,
        variance: f64,
        std_dev: f64,
        q1: f64,
        q3: f64,
        iqr: f64,
        min: u64,
        max: u64,
        coefficient_of_variation: f64,
    }

    #[derive(Serialize)]
    struct AnalysisGroup<'a> {
        label: &'a str,
        #[serde(rename = "range")]
        range_str: String,
        statistics: AnalysisStats,
        observations: Vec<AnalysisObservation>,
        outliers: Vec<AnalysisOutlier>,
    }

    let groups_out: Vec<AnalysisGroup> = groups
        .iter()
        .zip(stats.iter())
        .map(|(g, s)| {
            let observations: Vec<AnalysisObservation> = g
                .observations
                .iter()
                .map(|o| AnalysisObservation {
                    test_run_id: o.test_run_id,
                    trial_id: o.trial_id,
                    gas: o.gas,
                })
                .collect();

            let outliers: Vec<AnalysisOutlier> = s
                .outliers
                .iter()
                .map(|o| AnalysisOutlier {
                    test_run_id: o.test_run_id,
                    trial_id: o.trial_id,
                    gas: o.gas,
                })
                .collect();

            AnalysisGroup {
                label: &s.label,
                range_str: format!("{}:{}", g.test_run_start, g.test_run_end),
                statistics: AnalysisStats {
                    count: s.count,
                    mean: s.mean,
                    median: s.median,
                    variance: s.variance,
                    std_dev: s.std_dev,
                    q1: s.q1,
                    q3: s.q3,
                    iqr: s.iqr,
                    min: s.min,
                    max: s.max,
                    coefficient_of_variation: s.coefficient_of_variation,
                },
                observations,
                outliers,
            }
        })
        .collect();

    let json = serde_json::to_string_pretty(&serde_json::json!({ "groups": groups_out }))
        .context("failed to serialize analysis JSON")?;

    std::fs::write(&path, &json).with_context(|| format!("failed to write {path:?}"))?;
    eprintln!("[DEBUG] Analysis JSON saved to {}", path.display());
    Ok(())
}

/// Generate `summary.json`.
fn generate_json(stats: &[GroupStatistics], output_dir: &std::path::Path) -> Result<()> {
    let path = output_dir.join("summary.json");

    #[derive(serde::Serialize)]
    struct GroupJson<'a> {
        label: &'a str,
        count: usize,
        mean: f64,
        median: f64,
        variance: f64,
        std_dev: f64,
        #[serde(rename = "min")]
        min_val: u64,
        #[serde(rename = "max")]
        max_val: u64,
        q1: f64,
        q3: f64,
        iqr: f64,
        cv: f64,
    }

    let items: Vec<GroupJson> = stats
        .iter()
        .map(|s| GroupJson {
            label: &s.label,
            count: s.count,
            mean: s.mean,
            median: s.median,
            variance: s.variance,
            std_dev: s.std_dev,
            min_val: s.min,
            max_val: s.max,
            q1: s.q1,
            q3: s.q3,
            iqr: s.iqr,
            cv: s.coefficient_of_variation,
        })
        .collect();

    let json = serde_json::to_string_pretty(&serde_json::json!({ "groups": items }))
        .context("failed to serialize JSON")?;

    std::fs::write(&path, &json).with_context(|| format!("failed to write {path:?}"))?;
    eprintln!("[DEBUG] JSON summary saved to {}", path.display());
    Ok(())
}

/// Generate `summary.csv`.
fn generate_csv(groups: &[ExperimentGroup], output_dir: &std::path::Path) -> Result<()> {
    let path = output_dir.join("summary.csv");

    let mut file =
        std::fs::File::create(&path).with_context(|| format!("failed to create {path:?}"))?;

    writeln!(file, "group,test_run_id,trial_id,gas")
        .context("failed to write CSV header")?;

    for group in groups {
        for obs in &group.observations {
            writeln!(
                file,
                "{},{},{},{}",
                group.label, obs.test_run_id, obs.trial_id, obs.gas
            )
            .context("failed to write CSV row")?;
        }
    }

    eprintln!("[DEBUG] CSV summary saved to {}", path.display());
    Ok(())
}

/// Generate `report.md`.
fn generate_markdown(
    groups: &[ExperimentGroup],
    stats: &[GroupStatistics],
    output_dir: &std::path::Path,
) -> Result<()> {
    let path = output_dir.join("report.md");

    let mut content = String::new();

    content.push_str("# Experimental Synthesis Analysis\n\n");

    // Groups section.
    content.push_str("## Groups\n\n");
    for group in groups {
        content.push_str(&format!(
            "- {} (test runs {}-{})\n",
            group.label, group.test_run_start, group.test_run_end
        ));
    }
    content.push('\n');

    // Statistics table.
    content.push_str("## Group Statistics\n\n");
    content.push_str("| Group | Count | Mean | Median | Std Dev | CV | Min | Max | Q1 | Q3 | IQR |\n");
    content.push_str("|-------|-------|------|--------|---------|----|-----|-----|----|-----|-----|\n");

    for s in stats {
        content.push_str(&format!(
            "| {} | {} | {:.0} | {:.0} | {:.0} | {:.1}% | {} | {} | {:.0} | {:.0} | {:.0} |\n",
            s.label,
            s.count,
            s.mean,
            s.median,
            s.std_dev,
            s.coefficient_of_variation * 100.0,
            s.min,
            s.max,
            s.q1,
            s.q3,
            s.iqr,
        ));
    }
    content.push('\n');

    // Outliers section.
    content.push_str("## Outliers\n\n");
    for s in stats {
        if s.outliers.is_empty() {
            content.push_str(&format!("### {}\n\nNo outliers detected.\n\n", s.label));
        } else {
            content.push_str(&format!("### {}\n\n", s.label));
            content.push_str("| Test Run ID | Trial ID | Gas |\n");
            content.push_str("|-------------|----------|-----|\n");
            for o in &s.outliers {
                content.push_str(&format!("| {} | {} | {} |\n", o.test_run_id, o.trial_id, o.gas));
            }
            content.push('\n');
        }
    }

    // Interpretation section.
    content.push_str("## Interpretation\n\n");

    if let Some(best_mean) = stats
        .iter()
        .min_by(|a, b| a.mean.partial_cmp(&b.mean).unwrap_or(std::cmp::Ordering::Equal))
    {
        content.push_str(&format!(
            "- **Best mean gas:** {} ({:.0})\n",
            best_mean.label, best_mean.mean
        ));
    }

    if let Some(best_median) = stats
        .iter()
        .min_by(|a, b| a.median.partial_cmp(&b.median).unwrap_or(std::cmp::Ordering::Equal))
    {
        content.push_str(&format!(
            "- **Best median gas:** {} ({:.0})\n",
            best_median.label, best_median.median
        ));
    }

    if let Some(best_variance) = stats
        .iter()
        .min_by(|a, b| a.variance.partial_cmp(&b.variance).unwrap_or(std::cmp::Ordering::Equal))
    {
        content.push_str(&format!(
            "- **Lowest variance:** {} ({:.0})\n",
            best_variance.label, best_variance.variance
        ));
    }

    if let Some(best_cv) = stats
        .iter()
        .min_by(|a, b| {
            a.coefficient_of_variation
                .partial_cmp(&b.coefficient_of_variation)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    {
        content.push_str(&format!(
            "- **Lowest CV (most stable):** {} ({:.1}%)\n",
            best_cv.label,
            best_cv.coefficient_of_variation * 100.0
        ));
    }

    if let Some(best_gas) = stats.iter().min_by(|a, b| {
        a.mean
            .partial_cmp(&b.mean)
            .unwrap_or(std::cmp::Ordering::Equal)
    }) {
        content.push_str(&format!(
            "- **Most gas-efficient:** {} (mean {:.0})\n",
            best_gas.label, best_gas.mean
        ));
    }

    content.push('\n');

    std::fs::write(&path, &content).with_context(|| format!("failed to write {path:?}"))?;
    eprintln!("[DEBUG] Markdown report saved to {}", path.display());
    Ok(())
}
