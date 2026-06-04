//! Claude Code launch and MCP settings management.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Resolve project directory from root + project name.
pub fn resolve_project_dir(project_root: &Path, project_name: &str) -> PathBuf {
    project_root.join(project_name)
}

/// Inject `mcpServers` into `.claude/settings.local.json` (takes precedence over settings.json).
/// Backs up existing file, returns backup path.
pub fn setup_claude_settings(
    project_dir: &Path,
    _model_url: &str,
    _model_name: &str,
    mcp_cwd: &str,
    mcp_project: &str,
) -> Result<Option<PathBuf>> {
    let claude_dir = project_dir.join(".claude");
    let settings_path = claude_dir.join("settings.local.json");

    std::fs::create_dir_all(&claude_dir)?;

    // Read existing settings, or start empty.
    let mut settings: serde_json::Value = match std::fs::read_to_string(&settings_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or(serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };

    // Backup original.
    let backup_path = claude_dir.join("settings.local.json.queue_backup");
    let backup = if settings_path.exists() {
        std::fs::copy(&settings_path, &backup_path)?;
        eprintln!("[DEBUG] Backed up settings to {backup_path:?}");
        Some(backup_path)
    } else {
        None
    };

    // Inject mcpServers while preserving all existing settings.
    settings["mcpServers"] = serde_json::json!({
        "mcp_synth": {
            "command": "mcp_synth",
            "args": [
                "--cwd", mcp_cwd,
                "--project", mcp_project,
                "--db-type", "redis"
            ],
            "env": {}
        }
    });

    let content = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, content)?;
    // Also write standalone MCP config file for --mcp-config flag.
    let mcp_cfg: serde_json::Value = serde_json::json!({
        "mcpServers": {
            "mcp_synth": {
                "command": "mcp_synth",
                "args": [
                    "--cwd", mcp_cwd,
                    "--project", mcp_project,
                    "--db-type", "redis"
                ],
                "env": {}
            }
        }
    });
    let mcp_path = claude_dir.join("mcp_config.json");
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&mcp_cfg)?)?;
    eprintln!("[DEBUG] Injected mcpServers into {settings_path:?}");
    Ok(backup)
}

/// Restore original `.settings.local.json` from backup.
pub fn restore_claude_settings(project_dir: &Path, backup: Option<PathBuf>) -> Result<()> {
    let settings_path = project_dir.join(".claude").join("settings.local.json");
    // Clean up mcp_config.json if present.
    let mcp_path = project_dir.join(".claude").join("mcp_config.json");
    let _ = std::fs::remove_file(&mcp_path);
    match backup {
        Some(ref backup_path) if backup_path.exists() => {
            std::fs::copy(backup_path, &settings_path)?;
            let _ = std::fs::remove_file(backup_path);
            eprintln!("[DEBUG] Restored original settings.local.json");
        }
        // No backup means we created settings.local.json — remove it.
        Some(_) | None => {
            let _ = std::fs::remove_file(&settings_path);
            eprintln!("[DEBUG] Removed injected settings.local.json");
        }
    }
    Ok(())
}

use std::io::Write;

/// Kill any existing mcp_synth inherited from parent claude session.
/// Spawns a fresh one from our settings when Claude Code launches.
pub fn kill_existing_mcp_synth() {
    let output = Command::new("pkill")
        .args(["-f", "mcp_synth"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(o) if o.status.success() => eprintln!("[DEBUG] Killed existing mcp_synth process"),
        _ => eprintln!("[DEBUG] No existing mcp_synth to kill"),
    }
}

/// Launch Claude Code with prompt in project directory (blocking).
/// Pipes output through `jq` for formatting, writes to `output_path`.
pub fn launch_claude(project_dir: &Path, prompt: &str, output_path: &Path, model_name: &str) -> Result<()> {
    let file = std::fs::File::create(output_path)
        .with_context(|| format!("failed to create output file {output_path:?}"))?;

    // Use --strict-mcp-config to avoid inheriting parent session's MCP servers.
    // Override env vars to ensure correct model URL and name regardless of settings.
    let mcp_config_path = project_dir.join(".claude").join("mcp_config.json");
    let mcp_config_str = mcp_config_path.to_string_lossy().to_string();
    let claude = Command::new("claude")
        .args([
            "-p",
            "--output-format",
            "json",
            "--mcp-config",
            &mcp_config_str,
            "--strict-mcp-config",
            prompt,
        ])
        .current_dir(project_dir)
        .env("ANTHROPIC_BASE_URL", "http://127.0.0.1:8080")
        .env("ANTHROPIC_MODEL", model_name)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .context("failed to launch Claude Code")?;

    if !claude.status.success() {
        let stderr = String::from_utf8_lossy(&claude.stderr);
        bail!("Claude Code exited with status {}: {stderr}", claude.status);
    }

    // Pipe through jq for pretty formatting.
    let mut jq = Command::new("jq")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(file))
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn jq")?;

    if let Some(ref mut stdin) = jq.stdin {
        stdin.write_all(&claude.stdout)?;
    }
    drop(jq.stdin.take());

    let jq_status = jq.wait()?;
    if !jq_status.success() {
        bail!("jq formatting failed with status {jq_status}");
    }
    Ok(())
}
