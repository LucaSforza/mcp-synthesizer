# Extension: Automated Git Authentication Using SSH Keys

## Goal

The Git persistence system must support fully automated Git authentication using SSH keys only.

Authentication is required for:

* pushing synthesis branches;
* configuring upstream tracking;
* all remote Git operations performed by the synthesis pipeline.

The implementation must use:

```toml
auth-git2
```

together with:

```toml
git2
```

No interactive authentication is allowed.

---

# Motivation

The synthesis pipeline runs automatically through `queue_controller`.

After a successful synthesis:

1. A branch is created.
2. A commit is created.
3. The branch is pushed to the remote repository.

This workflow must run unattended.

Therefore authentication must be:

* deterministic;
* scriptable;
* non-interactive.

---

# Supported Authentication Method

Only SSH authentication is supported.

Supported remote format:

```text
git@github.com:user/repository.git
```

Examples:

```text
git@github.com:LucaSforza/git_diff_checker.git
git@github.com:LucaSforza/mcp-synthesizer.git
```

HTTPS remotes are explicitly unsupported.

If the configured remote uses HTTPS:

```text
return an error
```

---

# CLI Parameters

The executable must accept the path to an SSH private key.

Example:

```bash
--git-ssh-key ~/.ssh/id_ed25519
```

This argument is required whenever Git persistence is enabled.

---

# Authentication Flow

## Step 1

Read the SSH private key path from CLI.

Example:

```bash
--git-ssh-key ~/.ssh/id_ed25519
```

---

## Step 2

Verify that the file exists.

If not:

```text
return error
```

Example:

```text
SSH key not found:
~/.ssh/id_ed25519
```

---

## Step 3

Configure auth-git2 to authenticate using the provided key.

The implementation must not rely on:

* interactive prompts;
* ssh-agent;
* terminal input;
* credential helpers.

The provided key must be sufficient for authentication.

---

## Step 4

Use auth-git2 credentials during:

```text
git push
```

and any other remote operation.

---

# Security Requirements

The SSH private key must never appear in:

* logs;
* panic messages;
* debug output;
* error messages.

Forbidden:

```text
[DEBUG] Using key:
/home/user/.ssh/id_ed25519
```

Allowed:

```text
[DEBUG] Using configured SSH authentication
```

---

# Error Handling

Authentication failures must be explicit.

Examples:

```text
Failed to authenticate with remote origin
```

```text
SSH authentication failed
```

```text
Repository remote is not an SSH remote
```

---

# Remote Validation

Before attempting authentication:

1. Resolve remote `origin`.
2. Read its URL.
3. Verify it is an SSH remote.

Valid:

```text
git@github.com:user/repository.git
```

Invalid:

```text
https://github.com/user/repository.git
```

If the remote is not SSH:

```text
return error
```

---

# Integration With Git Persistence

Authentication must be used by:

```rust
persist_synthesis(...)
```

during:

* push;
* upstream configuration.

No Git operation should invoke external Git commands.

All operations must go through:

```rust
git2
auth-git2
```

---

# Suggested Configuration

```rust
pub struct GitAuthConfig {
    pub ssh_private_key: PathBuf,
}
```

The configuration should be created once during startup and passed to the Git persistence component.

---

# Logging

Use the project's existing logging style.

Examples:

```text
[DEBUG] Validating SSH remote
[DEBUG] Loading SSH authentication configuration
[DEBUG] Pushing synthesis branch
```

Do not log:

* private key contents;
* passphrases;
* credential details.

---

# Success Criteria

The task is complete when:

1. A synthesis branch can be pushed without user interaction.
2. Authentication is performed using auth-git2.
3. Only SSH remotes are accepted.
4. The SSH private key is provided via CLI.
5. No credentials appear in logs.
6. Authentication failures return meaningful errors.
7. queue_controller can push synthesis branches completely unattended.

```
```
