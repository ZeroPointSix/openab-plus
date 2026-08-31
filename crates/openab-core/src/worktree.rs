//! Per-session working directories (ZER-865).
//!
//! When enabled, each thread gets an isolated directory under
//! `[worktree].dir` (or `OPENAB_WORK_DIR`): a `git worktree` when the base
//! workspace is a git repo, otherwise a plain folder (D4).
//!
//! Isolation is for the code workspace only — not HOME or credentials (ZER-888).
//! Dirty worktree reclaim/delete is intentionally out of scope (D3).

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Default root for per-session work directories when `[worktree].dir` is omitted.
pub const DEFAULT_WORKTREE_DIR: &str = "/var/lib/openab/worktrees";

/// Environment override for the worktree root (Factory-style `FACTORY_WORK_DIR`).
pub const WORK_DIR_ENV: &str = "OPENAB_WORK_DIR";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct WorktreeConfig {
    /// Master switch. Defaults to `false` so existing single-workdir deployments
    /// keep previous behaviour until operators opt in.
    #[serde(default)]
    pub enabled: bool,
    /// Root directory that holds one subdirectory per thread.
    /// Overridden by `OPENAB_WORK_DIR` when that env var is set and non-empty.
    #[serde(default = "default_worktree_dir")]
    pub dir: String,
}

impl Default for WorktreeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: default_worktree_dir(),
        }
    }
}

fn default_worktree_dir() -> String {
    DEFAULT_WORKTREE_DIR.to_string()
}

/// Keep only `[A-Za-z0-9._-]`; everything else becomes `_`.
/// Empty results become `_` so the path segment is never blank.
pub fn sanitize_thread_id(thread_id: &str) -> String {
    let sanitized: String = thread_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized
    }
}

fn resolve_root(cfg: &WorktreeConfig) -> PathBuf {
    if let Ok(env_dir) = std::env::var(WORK_DIR_ENV) {
        let trimmed = env_dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    PathBuf::from(&cfg.dir)
}

/// True when `path` is inside a git work tree (file or directory `.git`, or
/// `git rev-parse --is-inside-work-tree` reports true).
pub fn is_git_repo(path: &Path) -> bool {
    let dot_git = path.join(".git");
    if dot_git.is_file() || dot_git.is_dir() {
        return true;
    }
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path)
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

fn ensure_under_root(root: &Path, target: &Path) -> Result<()> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("canonicalize worktree root {}", root.display()))?;
    // Target may not exist yet; canonicalize parent + join leaf.
    let canonical_target = if target.exists() {
        target
            .canonicalize()
            .with_context(|| format!("canonicalize worktree path {}", target.display()))?
    } else {
        let parent = target
            .parent()
            .ok_or_else(|| anyhow!("worktree path has no parent: {}", target.display()))?;
        let leaf = target
            .file_name()
            .ok_or_else(|| anyhow!("worktree path has no file name: {}", target.display()))?;
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create worktree parent {}", parent.display()))?;
        }
        parent
            .canonicalize()
            .with_context(|| format!("canonicalize worktree parent {}", parent.display()))?
            .join(leaf)
    };

    if !canonical_target.starts_with(&canonical_root) {
        bail!(
            "derived worktree path {} escapes configured root {}",
            canonical_target.display(),
            canonical_root.display()
        );
    }
    Ok(())
}

fn git_worktree_add(base: &Path, target: &Path) -> Result<()> {
    let output = Command::new("git")
        .args([
            "worktree",
            "add",
            "--detach",
            target
                .to_str()
                .ok_or_else(|| anyhow!("worktree path is not valid UTF-8"))?,
            "HEAD",
        ])
        .current_dir(base)
        .output()
        .with_context(|| {
            format!(
                "failed to run `git worktree add` for {} (is git installed?)",
                target.display()
            )
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "git worktree add failed for {}: {}{}",
            target.display(),
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!(" ({})", stdout.trim())
            }
        );
    }
    Ok(())
}

/// Resolve (and create if needed) the per-thread working directory.
///
/// Call only when worktree is enabled and there is no stored workdir and no
/// user `working_dir_override`. On success the returned path is ready to use
/// as the session cwd; the caller persists it via `session_workdirs`.
pub fn ensure_session_workdir(
    cfg: &WorktreeConfig,
    base_working_dir: &str,
    thread_id: &str,
) -> Result<PathBuf> {
    let root = resolve_root(cfg);
    std::fs::create_dir_all(&root)
        .with_context(|| format!("create worktree root {}", root.display()))?;

    let sanitized = sanitize_thread_id(thread_id);
    let target = root.join(&sanitized);
    ensure_under_root(&root, &target)?;

    if target.exists() {
        // D3: reuse existing directory; never delete or `git worktree remove`.
        return Ok(target.canonicalize().unwrap_or(target));
    }

    let base = Path::new(base_working_dir);
    if is_git_repo(base) {
        git_worktree_add(base, &target)?;
    } else {
        // D4: non-git base → plain folder, do not require git.
        std::fs::create_dir_all(&target)
            .with_context(|| format!("create session workdir {}", target.display()))?;
    }

    Ok(target.canonicalize().unwrap_or(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// Serialize tests that touch `OPENAB_WORK_DIR` so parallel cargo test
    /// workers do not race on process-wide env.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn init_git_repo(dir: &Path) {
        assert!(Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(dir)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .status()
            .unwrap()
            .success());
        fs::write(dir.join("README.md"), "hi").unwrap();
        assert!(Command::new("git")
            .args(["add", "README.md"])
            .current_dir(dir)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn sanitize_strips_path_escape() {
        assert_eq!(sanitize_thread_id("a/../b"), "a_.._b");
        assert_eq!(sanitize_thread_id(".."), "..");
        assert_eq!(sanitize_thread_id("thread:123"), "thread_123");
        assert_eq!(sanitize_thread_id("ok-._9"), "ok-._9");
        assert_eq!(sanitize_thread_id(""), "_");
    }

    #[test]
    fn git_repo_two_threads_get_distinct_worktrees() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(WORK_DIR_ENV);

        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("repo");
        let work_root = tmp.path().join("work");
        fs::create_dir_all(&base).unwrap();
        init_git_repo(&base);

        let cfg = WorktreeConfig {
            enabled: true,
            dir: work_root.to_string_lossy().into_owned(),
        };

        let a = ensure_session_workdir(&cfg, base.to_str().unwrap(), "thread-a").unwrap();
        let b = ensure_session_workdir(&cfg, base.to_str().unwrap(), "thread-b").unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with(&work_root.canonicalize().unwrap()));
        assert!(b.starts_with(&work_root.canonicalize().unwrap()));
        assert!(a.join(".git").exists());
        assert!(b.join(".git").exists());

        let list = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&base)
            .output()
            .unwrap();
        assert!(list.status.success());
        let out = String::from_utf8_lossy(&list.stdout);
        assert!(out.contains(a.to_str().unwrap()) || a.exists());
        assert!(out.contains(b.to_str().unwrap()) || b.exists());
    }

    #[test]
    fn non_git_base_creates_plain_folder() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(WORK_DIR_ENV);

        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("plain");
        let work_root = tmp.path().join("work");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("file.txt"), "x").unwrap();

        let cfg = WorktreeConfig {
            enabled: true,
            dir: work_root.to_string_lossy().into_owned(),
        };
        let path = ensure_session_workdir(&cfg, base.to_str().unwrap(), "t1").unwrap();
        assert!(path.is_dir());
        assert!(!path.join(".git").exists());
        assert!(path.starts_with(work_root.canonicalize().unwrap()));
    }

    #[test]
    fn existing_dir_is_reused_without_deleting_contents() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(WORK_DIR_ENV);

        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("plain");
        let work_root = tmp.path().join("work");
        fs::create_dir_all(&base).unwrap();

        let cfg = WorktreeConfig {
            enabled: true,
            dir: work_root.to_string_lossy().into_owned(),
        };
        let first = ensure_session_workdir(&cfg, base.to_str().unwrap(), "reuse-me").unwrap();
        let sentinel = first.join("sentinel.txt");
        fs::write(&sentinel, "keep-me").unwrap();

        let second = ensure_session_workdir(&cfg, base.to_str().unwrap(), "reuse-me").unwrap();
        assert_eq!(first, second);
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "keep-me");
    }

    #[test]
    fn path_escape_thread_id_stays_under_root() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(WORK_DIR_ENV);

        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("plain");
        let work_root = tmp.path().join("work");
        fs::create_dir_all(&base).unwrap();

        let cfg = WorktreeConfig {
            enabled: true,
            dir: work_root.to_string_lossy().into_owned(),
        };
        let path = ensure_session_workdir(&cfg, base.to_str().unwrap(), "../../escape").unwrap();
        let root = work_root.canonicalize().unwrap();
        assert!(path.starts_with(&root));
        assert_eq!(
            path.file_name().and_then(|s| s.to_str()),
            Some(".._.._escape")
        );
    }

    #[test]
    fn openab_work_dir_env_overrides_config_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("plain");
        let cfg_dir = tmp.path().join("from-config");
        let env_dir = tmp.path().join("from-env");
        fs::create_dir_all(&base).unwrap();

        std::env::set_var(WORK_DIR_ENV, &env_dir);
        let cfg = WorktreeConfig {
            enabled: true,
            dir: cfg_dir.to_string_lossy().into_owned(),
        };
        let path = ensure_session_workdir(&cfg, base.to_str().unwrap(), "env-thread").unwrap();
        assert!(path.starts_with(env_dir.canonicalize().unwrap()));
        assert!(!path.starts_with(&cfg_dir));
        std::env::remove_var(WORK_DIR_ENV);
    }

    #[test]
    fn default_config_is_disabled() {
        let cfg = WorktreeConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.dir, DEFAULT_WORKTREE_DIR);
    }
}
