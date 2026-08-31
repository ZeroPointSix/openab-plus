//! Per-session working directories (ZER-865).
//!
//! When enabled, each thread gets an isolated directory under
//! `[worktree].dir` (or `OPENAB_WORK_DIR`): a `git worktree` when the base
//! workspace is a git repo, otherwise a plain folder (D4).
//!
//! Path sanitize and derived-root escape checks reuse [`crate::path_bounds`]
//! (ZER-889) so daemon-derived worktrees and user `[[ws:]]` validation stay on
//! one shared boundary implementation.
//!
//! Isolation is for the code workspace only — not HOME or credentials (ZER-888).
//! Dirty worktree reclaim/delete is intentionally out of scope (D3).

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::path_bounds::{self, sanitize_thread_segment};

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

/// Sanitize a thread id into a single path segment.
///
/// Thin wrapper over [`path_bounds::sanitize_thread_segment`] so call sites and
/// tests keep the ZER-865 name while sharing one implementation with ZER-889.
pub fn sanitize_thread_id(thread_id: &str) -> String {
    sanitize_thread_segment(thread_id)
}

/// Resolve the worktree root: non-empty `OPENAB_WORK_DIR` wins over `[worktree].dir`.
pub fn resolve_worktree_root(cfg: &WorktreeConfig) -> PathBuf {
    if let Ok(env_dir) = std::env::var(WORK_DIR_ENV) {
        let trimmed = env_dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    PathBuf::from(&cfg.dir)
}

fn resolve_root(cfg: &WorktreeConfig) -> PathBuf {
    resolve_worktree_root(cfg)
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
///
/// Path sanitize + root escape checks go through [`path_bounds::ensure_derived_dir`];
/// non-git bases use [`path_bounds::ensure_plain_folder`] (D4). This never deletes
/// existing trees (D3 out of scope) and never calls `AcpConnection::spawn`.
pub fn ensure_session_workdir(
    cfg: &WorktreeConfig,
    base_working_dir: &str,
    thread_id: &str,
) -> Result<PathBuf> {
    let root = resolve_root(cfg);
    // Shared ZER-889 boundary: sanitize segment, create/writable root, refuse escape.
    let target = path_bounds::ensure_derived_dir(&root, thread_id)?;

    if target.exists() {
        // D3: reuse existing directory; never delete or `git worktree remove`.
        return Ok(target.canonicalize().unwrap_or(target));
    }

    let base = Path::new(base_working_dir);
    if is_git_repo(base) {
        git_worktree_add(base, &target)?;
    } else {
        // D4: non-git base → plain folder via shared helper, do not require git.
        path_bounds::ensure_plain_folder(&target)?;
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
        // Same implementation as path_bounds.
        assert_eq!(
            sanitize_thread_id("../etc/passwd"),
            path_bounds::sanitize_thread_segment("../etc/passwd")
        );
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
    fn dotdot_thread_id_rejected_via_shared_path_bounds() {
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
        // Sanitizer keeps literal `..`; shared ensure_derived_dir must refuse escape.
        let err = ensure_session_workdir(&cfg, base.to_str().unwrap(), "..").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("escapes configured root"),
            "expected shared path_bounds escape error, got: {msg}"
        );
        // Same failure mode as calling path_bounds directly.
        let direct = path_bounds::ensure_derived_dir(&work_root, "..").unwrap_err();
        assert!(format!("{direct:#}").contains("escapes configured root"));
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
