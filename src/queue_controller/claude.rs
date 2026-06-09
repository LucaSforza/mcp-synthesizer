//! Claude Code launch, MCP settings management, and model endpoint abstraction.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// An OpenAI-compatible model endpoint for Claude Code to use.
///
/// Created from either:
/// - **Cluster mode**: tunnel to localhost (e.g., `http://127.0.0.1:8080`)
/// - **API mode**: external URL from job metadata (e.g., `https://api.openai.com/v1`)
///
/// The `url` is the base for `ANTHROPIC_BASE_URL` (without `/v1` suffix).
#[derive(Debug, Clone)]
pub struct ModelEndpoint {
    pub url: String,
    pub model_name: String,
}

/// Resolve project directory from root + project name.
pub fn resolve_project_dir(project_root: &Path, project_name: &str) -> PathBuf {
    project_root.join(project_name)
}

/// Inject `mcpServers` into `.claude/settings.local.json` (takes precedence over settings.json).
/// Also sets `customModel` and `env` to match the target model endpoint.
/// Backs up existing file, returns backup path.
pub fn setup_claude_settings(
    project_dir: &Path,
    model_url: &str,
    model_name: &str,
    api_key: &str,
    mcp_cwd: &str,
    mcp_project: &str,
    fuzz_seed: Option<u64>,
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
    let seed_str = fuzz_seed.map(|s| s.to_string());
    let mut synth_args = vec!["--cwd", mcp_cwd, "--project", mcp_project, "--db-type", "redis", "--model-name", model_name];
    if let Some(ref s) = seed_str {
        synth_args.push("--fuzz-seed");
        synth_args.push(s);
    }
    settings["mcpServers"] = serde_json::json!({
        "mcp_synth": {
            "command": "mcp_synth",
            "args": synth_args,
            "env": {}
        }
    });

    // Override customModel and env to match the actual endpoint.
    // Required for API mode (external URL); also correct for cluster mode
    // (localhost tunnel overwrites the same values the file already has).
    settings["customModel"] = serde_json::json!({
        "apiKey": api_key,
        "modelCapabilities": ["completion"],
        "modelName": model_name,
        "provider": "openai",
        "url": model_url,
    });
    if let Some(env) = settings["env"].as_object_mut() {
        env.insert("ANTHROPIC_BASE_URL".into(), serde_json::json!(model_url));
        env.insert("ANTHROPIC_MODEL".into(), serde_json::json!(model_name));
    } else {
        let mut env = serde_json::Map::new();
        env.insert("ANTHROPIC_BASE_URL".into(), serde_json::json!(model_url));
        env.insert("ANTHROPIC_MODEL".into(), serde_json::json!(model_name));
        settings["env"] = serde_json::Value::Object(env);
    }

    let content = serde_json::to_string_pretty(&settings)?;
    std::fs::write(&settings_path, content)?;
    // Also write standalone MCP config file for --mcp-config flag.
    let seed_str = fuzz_seed.map(|s| s.to_string());
    let mut mcp_args = vec!["--cwd", mcp_cwd, "--project", mcp_project, "--db-type", "redis", "--model-name", model_name];
    if let Some(ref s) = seed_str {
        mcp_args.push("--fuzz-seed");
        mcp_args.push(s);
    }
    let mcp_cfg: serde_json::Value = serde_json::json!({
        "mcpServers": {
            "mcp_synth": {
                "command": "mcp_synth",
                "args": mcp_args,
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

/// Spawn Claude Code process with prompt in project directory.
/// Returns the Child handle so caller can store PID for signal handling and wait.
/// `system_prompt` is passed via `--append-system-prompt` (project's prompt.md).
/// `endpoint` provides `ANTHROPIC_BASE_URL` and `ANTHROPIC_MODEL` dynamically.
pub fn spawn_claude(
    project_dir: &Path,
    prompt: &str,
    system_prompt: &str,
    output_path: &Path,
    endpoint: &ModelEndpoint,
) -> Result<std::process::Child> {
    let file = std::fs::File::create(output_path)
        .with_context(|| format!("failed to create output file {output_path:?}"))?;

    let mcp_config_path = project_dir.join(".claude").join("mcp_config.json");
    let mcp_config_str = mcp_config_path.to_string_lossy().to_string();
    let mut args: Vec<&str> = vec![
        "-p",
        "--output-format",
        "stream-json",
        "--dangerously-skip-permissions",
        // "--include-hook-events",
        "--verbose",
        "--mcp-config",
        &mcp_config_str,
        "--strict-mcp-config",
    ];
    if !system_prompt.is_empty() {
        args.push("--append-system-prompt");
        args.push(system_prompt);
    }
    args.push(prompt);

    eprintln!(
        "[DEBUG] ANTHROPIC_BASE_URL={} ANTHROPIC_MODEL={}",
        endpoint.url, endpoint.model_name,
    );

    let child = Command::new("claude")
        .args(&args)
        .current_dir(project_dir)
        .env("CAVEMAN_DEFAULT_MODE", "wenyan")
        .env("ANTHROPIC_BASE_URL", &endpoint.url)
        .env("ANTHROPIC_MODEL", &endpoint.model_name)
        .env("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "100000")
        .stdin(Stdio::inherit())
        .stdout(Stdio::from(file))
        .stderr(Stdio::inherit())
        .spawn()
        .context("failed to spawn Claude Code")?;

    Ok(child)
}
