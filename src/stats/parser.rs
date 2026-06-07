use anyhow::{Context, Result, bail};

/// Parse a `--range` argument of the form `label=start:end`.
///
/// Returns `(label, start, end)` where start and end are 1-based inclusive bounds.
pub fn parse_range(input: &str) -> Result<(String, u64, u64)> {
    let (label, range_str) = input
        .split_once('=')
        .context("range must be in format label=start:end")?;

    if label.is_empty() {
        bail!("range label must not be empty");
    }

    let Some((start_str, end_str)) = range_str.split_once(':') else {
        bail!("range must be in format label=start:end, missing ':'");
    };

    let start: u64 = start_str
        .parse()
        .with_context(|| format!("invalid start value '{start_str}'"))?;
    let end: u64 = end_str
        .parse()
        .with_context(|| format!("invalid end value '{end_str}'"))?;

    if start == 0 {
        bail!("start value must be >= 1");
    }

    if end < start {
        bail!("end ({end}) must be >= start ({start})");
    }

    Ok((label.to_string(), start, end))
}

#[cfg(test)]
#[path = "parser_test.rs"]
mod tests;
