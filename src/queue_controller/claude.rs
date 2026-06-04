//! Claude Code launch and MCP settings management.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Resolve project directory from root + project name.
pub fn resolve_project_dir(project_root: &Path, project_name: &str) -> PathBuf {
    project_root.join(project_name)
}

/// Generate `.claude/settings.json` for project.
/// Backs up existing file if present, returns backup path.
pub fn setup_claude_settings(
    project_dir: &Path,
    model_url: &str,
    mcp_cwd: &str,
    mcp_project: &str,
) -> Result<Option<PathBuf>> {
    let claude_dir = project_dir.join(".claude");
    let settings_path = claude_dir.join("settings.json");

    std::fs::create_dir_all(&claude_dir)?;

    let backup = if settings_path.exists() {
        let backup_path = settings_path.with_extension("settings.json.queue_backup");
        std::fs::copy(&settings_path, &backup_path)?;
        eprintln!("[DEBUG] Backed up settings to {backup_path:?}");
        Some(backup_path)
    } else {
        None
    };

    let settings = serde_json::json!({
        "primaryModel": "queue-synth-model",
        "provider": {
            "id": "openai",
            "config": {
                "baseUrl": model_url,
                "apiKey": "not-needed"
            }
        },
        "models": {
            "queue-synth-model": {
                "provider": "openai",
                "model": "queue-synth-model"
            }
        },
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

    let content = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, content)?;
    eprintln!("[DEBUG] Wrote MCP settings to {settings_path:?}");
    Ok(backup)
}

/// Restore original settings or remove temp file.
pub fn restore_claude_settings(project_dir: &Path, backup: Option<PathBuf>) -> Result<()> {
    let settings_path = project_dir.join(".claude").join("settings.json");
    match backup {
        Some(ref backup_path) if backup_path.exists() => {
            std::fs::copy(backup_path, &settings_path)?;
            let _ = std::fs::remove_file(backup_path);
            eprintln!("[DEBUG] Restored original settings");
        }
        _ => {
            let _ = std::fs::remove_file(&settings_path);
            eprintln!("[DEBUG] Removed temporary settings");
        }
    }
    Ok(())
}

/// Launch Claude Code with prompt in project directory (blocking).
pub fn launch_claude(project_dir: &Path, prompt: &str) -> Result<()> {
    let status = Command::new("claude")
        .args(["--prompt", prompt, "--cd", &project_dir.to_string_lossy()])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to launch Claude Code")?;

    if !status.success() {
        bail!("Claude Code exited with status {}", status);
    }
    Ok(())
}
