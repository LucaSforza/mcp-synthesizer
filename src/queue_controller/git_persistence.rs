//! Git persistence for synthesis outputs.
//!
//! After successful synthesis, commits all changes in the project directory
//! to a new branch `{model_name}-{iteration}-{seed}` and pushes to origin.
//!
//! Uses git2 + auth-git2 — no shelling out to `git` CLI.

use anyhow::{Context, Result, bail};
use git2::{BranchType, Repository};
use std::path::{Path, PathBuf};

/// Authentication configuration for Git operations.
pub struct GitAuthConfig {
    pub ssh_private_key: PathBuf,
}

impl GitAuthConfig {
    pub fn new(ssh_private_key: PathBuf) -> Self {
        Self { ssh_private_key }
    }
}

/// Wraps a git2 `Repository` for synthesis output persistence.
pub struct GitPersistence {
    repo: Repository,
    authenticator: auth_git2::GitAuthenticator,
}

impl GitPersistence {
    /// Open a git repository and configure SSH authentication.
    pub fn new(repo_path: &Path, auth_config: &GitAuthConfig) -> Result<Self> {
        let repo = Repository::open(repo_path)
            .with_context(|| format!("failed to open git repository at {repo_path:?}"))?;

        // Build authenticator: only SSH key from file, no agent, no prompts.
        let authenticator = auth_git2::GitAuthenticator::new_empty()
            .try_ssh_agent(false)
            .try_cred_helper(false)
            .try_password_prompt(0)
            .prompt_ssh_key_password(false)
            .add_ssh_key_from_file(&auth_config.ssh_private_key, None as Option<String>);

        eprintln!("[DEBUG] Git: configured SSH authentication");
        Ok(Self {
            repo,
            authenticator,
        })
    }

    /// Phase 1 — create synthesis branch and checkout (run BEFORE Claude Code).
    ///
    /// Steps performed:
    ///   1. Record current HEAD shorthand.
    ///   2. Build branch name (sanitize model_name for git ref validity).
    ///   3. Check if target branch already exists (fail-fast).
    ///   4. Create new branch from HEAD commit.
    ///   5. Checkout new branch.
    ///
    /// Returns `(orig_branch, branch_name)` so the caller can pass them to
    /// `commit_and_push()` after the synthesis completes.
    pub fn checkout_synthesis_branch(
        &self,
        model_name: &str,
        iteration: u64,
        seed: u64,
    ) -> Result<(String, String)> {
        // -- Step 1: Store original branch ------------------------------------
        let head = self
            .repo
            .head()
            .context("no HEAD reference found (repository may be empty)")?;
        let orig_branch = head
            .shorthand()
            .context("HEAD has no shorthand name")?
            .to_string();
        let head_commit = head
            .peel_to_commit()
            .context("failed to resolve HEAD to a commit object")?;
        eprintln!("[DEBUG] Git: current HEAD on '{orig_branch}'");

        // -- Step 2: Build branch name (sanitize for git ref validity) --------
        let safe_model: String = model_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let branch_name = format!("{}-{}-{}", safe_model, iteration, seed);
        eprintln!("[DEBUG] Git: creating synthesis branch '{branch_name}'");

        // -- Step 3: Check for branch conflict (fail-fast) --------------------
        if self
            .repo
            .find_branch(&branch_name, BranchType::Local)
            .is_ok()
        {
            bail!("branch already exists: {branch_name}");
        }

        // -- Step 4: Create branch from HEAD commit --------------------------
        let branch = self
            .repo
            .branch(&branch_name, &head_commit, false)
            .with_context(|| format!("failed to create branch '{branch_name}'"))?;
        let branch_ref = branch.into_reference();
        let branch_ref_name = branch_ref
            .name()
            .context("newly created branch reference has no name")?
            .to_string();

        // -- Step 5: Checkout new branch -------------------------------------
        self.repo
            .set_head(&branch_ref_name)
            .context("failed to set HEAD to new branch")?;
        self.repo
            .checkout_head(Some(
                git2::build::CheckoutBuilder::new()
                    .force()
                    .remove_untracked(false),
            ))
            .with_context(|| format!("failed to checkout branch '{branch_name}'"))?;
        eprintln!("[DEBUG] Git: checked out branch '{branch_name}'");

        Ok((orig_branch, branch_name))
    }

    /// Phase 2 — stage, commit, push, and restore original branch
    /// (run AFTER successful synthesis, on the branch already checked out).
    ///
    /// Steps performed:
    ///   6. Stage all files (`git add -A` equivalent).
    ///   7. Create commit with caller-provided message.
    ///   8. Push to origin and set upstream tracking.
    ///   9. Checkout original branch (restore working tree).
    pub fn commit_and_push(
        &self,
        branch_name: &str,
        orig_branch: &str,
        commit_message: &str,
    ) -> Result<()> {
        // -- Step 6: Stage all changes (git add -A equivalent) ---------------
        let mut index = self
            .repo
            .index()
            .context("failed to open repository index")?;
        index
            .add_all(
                ["*"].iter(),
                git2::IndexAddOption::CHECK_PATHSPEC | git2::IndexAddOption::DEFAULT,
                None,
            )
            .context("failed to stage changes")?;
        index
            .update_all(["*"], None)
            .context("failed to update index for deleted files")?;
        index
            .write()
            .context("failed to write index after staging")?;

        let tree_id = index
            .write_tree()
            .context("failed to write tree object from index")?;
        let tree = self
            .repo
            .find_tree(tree_id)
            .with_context(|| format!("failed to find newly created tree {tree_id}"))?;
        eprintln!("[DEBUG] Git: staged all repository changes");

        // -- Step 7: Create commit -------------------------------------------
        let sig = self
            .repo
            .signature()
            .context("failed to read git signature (check git config user.name and user.email)")?;

        // Find HEAD commit as parent.
        let head = self
            .repo
            .head()
            .context("no HEAD reference for commit parent")?;
        let head_commit = head
            .peel_to_commit()
            .context("failed to resolve HEAD to a commit object")?;

        let commit_oid = self
            .repo
            .commit(
                Some("HEAD"),
                &sig,
                &sig,
                commit_message,
                &tree,
                &[&head_commit],
            )
            .context("failed to create commit")?;
        eprintln!("[DEBUG] Git: created commit {commit_oid}: '{commit_message}'");

        // -- Step 8: Push to origin + set upstream tracking ------------------
        let push_result = self.push_and_set_upstream(branch_name);

        // Always restore original branch before propagating any error.
        self.restore_original_branch(orig_branch)?;

        push_result?;
        Ok(())
    }

    /// Push branch to origin and set upstream tracking.
    /// Returns Err if push or upstream configuration fails.
    fn push_and_set_upstream(&self, branch_name: &str) -> Result<()> {
        let mut remote = self
            .repo
            .find_remote("origin")
            .context("failed to find remote 'origin'")?;

        // Validate remote URL is SSH.
        let remote_url = remote
            .url()
            .context("remote 'origin' has no URL")?
            .to_string();
        if !remote_url.starts_with("git@") && !remote_url.starts_with("ssh://") {
            bail!(
                "remote 'origin' URL is not an SSH remote: expected git@ or ssh://, got {remote_url:?}"
            );
        }

        // Build credentials callback from auth-git2.
        let git_config =
            git2::Config::open_default().context("failed to open git config for authentication")?;
        let mut push_callbacks = git2::RemoteCallbacks::new();
        push_callbacks.credentials(self.authenticator.credentials(&git_config));

        let mut push_opts = git2::PushOptions::new();
        push_opts.remote_callbacks(push_callbacks);

        let refspec = format!("refs/heads/{}:refs/heads/{}", branch_name, branch_name);
        remote
            .push(&[&refspec], Some(&mut push_opts))
            .with_context(|| format!("failed to push branch '{branch_name}' to origin"))?;
        eprintln!("[DEBUG] Git: pushed branch '{branch_name}' to origin");

        // Equivalent to: git branch --set-upstream-to=origin/<branch>
        let mut branch = self
            .repo
            .find_branch(branch_name, BranchType::Local)
            .with_context(|| format!("failed to find local branch '{branch_name}'"))?;
        let upstream = format!("origin/{}", branch_name);
        branch
            .set_upstream(Some(upstream.as_str()))
            .with_context(|| format!("failed to set upstream tracking for '{branch_name}'"))?;
        eprintln!("[DEBUG] Git: upstream tracking set: '{branch_name}' -> 'origin/{branch_name}'");

        Ok(())
    }

    /// Restore working tree to original branch.
    fn restore_original_branch(&self, orig_branch: &str) -> Result<()> {
        self.repo
            .set_head(&format!("refs/heads/{}", orig_branch))
            .with_context(|| format!("failed to set HEAD back to '{orig_branch}'"))?;
        self.repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .with_context(|| format!("failed to checkout original branch '{orig_branch}'"))?;
        eprintln!("[DEBUG] Git: returned to original branch '{orig_branch}'");
        Ok(())
    }
}
