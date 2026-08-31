//! Path boundary helpers for daemon-derived worktree directories (ZER-889).
//!
//! User-supplied workspace paths stay under [`crate::directives::resolve_workspace`]
//! (must exist, must be a directory, must stay inside bot_home).
//!
//! Derived session worktrees live under a separate root (typically
//! `/var/lib/openab/worktrees` or `OPENAB_WORK_DIR`) and **must not** be passed
//! through `resolve_workspace`. This module only validates / creates path
//! boundaries — it does not run `git worktree add` (that belongs to ZER-865).

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

/// Keep only `[A-Za-z0-9._-]`; everything else becomes `_`.
/// Empty results become `_` so the path segment is never blank.
pub fn sanitize_thread_segment(thread_segment: &str) -> String {
    let sanitized: String = thread_segment
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

/// Ensure `root` is usable and return the derived path `root/<sanitized>` that
/// is guaranteed to stay under the canonical root.
///
/// Unlike [`crate::directives::resolve_workspace`], the target path **may not
/// exist yet**. This never creates the leaf directory; use
/// [`create_derived_dir`] or [`ensure_plain_folder`] for that.
pub fn ensure_derived_dir(root: &Path, thread_segment: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(root)
        .with_context(|| format!("create or open derived worktree root {}", root.display()))?;

    // Prove the root is writable (and create a marker we immediately remove).
    let probe = root.join(".openab-path-bounds-write-probe");
    std::fs::write(&probe, b"")
        .with_context(|| format!("derived worktree root is not writable: {}", root.display()))?;
    let _ = std::fs::remove_file(&probe);

    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("canonicalize derived worktree root {}", root.display()))?;

    let sanitized = sanitize_thread_segment(thread_segment);
    let target = canonical_root.join(&sanitized);

    ensure_under_root(&canonical_root, &target)?;
    Ok(target)
}

/// Like [`ensure_derived_dir`], then `create_dir_all` on the derived path.
/// Does **not** run `git worktree add`.
pub fn create_derived_dir(root: &Path, thread_segment: &str) -> Result<PathBuf> {
    let target = ensure_derived_dir(root, thread_segment)?;
    std::fs::create_dir_all(&target)
        .with_context(|| format!("create derived directory {}", target.display()))?;
    target
        .canonicalize()
        .with_context(|| format!("canonicalize derived directory {}", target.display()))
}

/// D4 helper: ensure `path` exists as a plain directory via `create_dir_all`.
///
/// Never requires a git repository and never reports "must be a git repo".
/// Intended for non-git base workspaces when ZER-865 allocates a session folder.
pub fn ensure_plain_folder(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("create plain session folder {}", path.display()))?;
    if !path.is_dir() {
        bail!(
            "plain session folder path is not a directory: {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_under_root(canonical_root: &Path, target: &Path) -> Result<()> {
    let canonical_target = if target.exists() {
        target
            .canonicalize()
            .with_context(|| format!("canonicalize derived path {}", target.display()))?
    } else {
        let parent = target
            .parent()
            .ok_or_else(|| anyhow!("derived path has no parent: {}", target.display()))?;
        let leaf = target
            .file_name()
            .ok_or_else(|| anyhow!("derived path has no file name: {}", target.display()))?;
        // Parent should already be the canonical root (single-segment join),
        // but canonicalize defensively for symlink roots.
        let canonical_parent = if parent.exists() {
            parent
                .canonicalize()
                .with_context(|| format!("canonicalize derived parent {}", parent.display()))?
        } else {
            // Should not happen after ensure_derived_dir created the root, but
            // keep the escape check sound if callers pass a nested path later.
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create derived parent {}", parent.display()))?;
            parent
                .canonicalize()
                .with_context(|| format!("canonicalize derived parent {}", parent.display()))?
        };
        canonical_parent.join(leaf)
    };

    if !canonical_target.starts_with(canonical_root) {
        bail!(
            "derived worktree path {} escapes configured root {}",
            canonical_target.display(),
            canonical_root.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn sanitize_strips_path_escape() {
        assert_eq!(sanitize_thread_segment("a/../b"), "a_.._b");
        assert_eq!(sanitize_thread_segment("../etc/passwd"), ".._etc_passwd");
        assert_eq!(sanitize_thread_segment("thread:123"), "thread_123");
        assert_eq!(sanitize_thread_segment("ok-._9"), "ok-._9");
        assert_eq!(sanitize_thread_segment(""), "_");
    }

    #[test]
    fn derived_path_allows_missing_target() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("worktrees");
        let target = ensure_derived_dir(&root, "thread-abc").unwrap();
        assert!(!target.exists());
        assert_eq!(target, root.canonicalize().unwrap().join("thread-abc"));
    }

    #[test]
    fn derived_path_sanitizes_escape_attempt() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("worktrees");
        fs::create_dir_all(&root).unwrap();
        let target = ensure_derived_dir(&root, "../etc/passwd").unwrap();
        let canonical_root = root.canonicalize().unwrap();
        assert!(target.starts_with(&canonical_root));
        assert_eq!(target.file_name().unwrap(), ".._etc_passwd");
        assert!(!target.exists());
    }

    #[test]
    fn derived_dotdot_segment_rejected_by_boundary() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("worktrees");
        fs::create_dir_all(&root).unwrap();
        // Sanitizer keeps `..` as a legal character sequence; join would escape
        // unless ensure_under_root rejects it.
        let err = ensure_derived_dir(&root, "..").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("escapes configured root"),
            "expected escape error, got: {msg}"
        );
    }

    #[test]
    fn create_derived_dir_makes_directory() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("worktrees");
        let target = create_derived_dir(&root, "sess-1").unwrap();
        assert!(target.is_dir());
        assert!(target.starts_with(root.canonicalize().unwrap()));
    }

    #[test]
    fn ensure_plain_folder_succeeds_without_git() {
        let tmp = TempDir::new().unwrap();
        let folder = tmp.path().join("plain").join("nested");
        assert!(!folder.join(".git").exists());
        ensure_plain_folder(&folder).unwrap();
        assert!(folder.is_dir());
        // Still not a git repo — and we must not error about that.
        assert!(!folder.join(".git").exists());
    }

    #[test]
    fn root_must_be_writable() {
        // Root bypasses DAC: chmod / set_readonly cannot make a directory
        // unwritable to euid 0 (Colab CPU and similar images). Skip the
        // negative assertion there; non-root (Daytona/CI) still exercises it.
        #[cfg(unix)]
        if unsafe { libc::geteuid() } == 0 {
            return;
        }

        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("locked");
        fs::create_dir_all(&root).unwrap();
        let mut perms = fs::metadata(&root).unwrap().permissions();
        let original_readonly = perms.readonly();
        perms.set_readonly(true);
        fs::set_permissions(&root, perms).unwrap();
        let result = ensure_derived_dir(&root, "x");
        // Restore so TempDir cleanup works.
        let mut perms = fs::metadata(&root).unwrap().permissions();
        perms.set_readonly(original_readonly);
        fs::set_permissions(&root, perms).unwrap();
        if !original_readonly {
            // Only assert failure when we actually made it readonly.
            assert!(result.is_err());
        }
    }
}
