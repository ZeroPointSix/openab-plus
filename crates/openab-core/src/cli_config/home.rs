use anyhow::{anyhow, Result};
use std::path::PathBuf;

/// Real login HOME for the agent process.
///
/// D2 / ZER-888: spawn must keep `HOME` as the real home so OAuth/auth files
/// under `~/.claude`, `~/.codex`, etc. remain reachable. Do **not** point
/// `HOME` at a per-profile isolation directory.
///
/// `OPENAB_TEST_HOME` is a test-only stand-in for that real home (and, when
/// set, also anchors OpenAB-owned paths under the same temp tree).
pub fn real_home_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("OPENAB_TEST_HOME") {
        let path = path.trim();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    dirs::home_dir().ok_or_else(|| anyhow!("unable to resolve home directory for CLI config"))
}

/// Backward-compatible alias: historically used for both spawn HOME and writer
/// roots. Spawn callers should prefer [`real_home_dir`]; writers should use
/// [`cli_config_dir`] / profile-aware path helpers.
pub fn cli_home_dir() -> Result<PathBuf> {
    real_home_dir()
}

/// OpenAB-owned state root (`OPENAB_HOME`, else `~/.openab`).
///
/// When `OPENAB_TEST_HOME` is set, isolation paths live under that temp tree
/// (`$OPENAB_TEST_HOME/cli/...`) so existing tests keep a writable sandbox.
pub fn openab_home_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("OPENAB_TEST_HOME") {
        let path = path.trim();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    if let Ok(path) = std::env::var("OPENAB_HOME") {
        let path = path.trim();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    Ok(real_home_dir()?.join(".openab"))
}

/// Sanitize a profile id (or `"system"`) for use as a single path segment.
pub fn sanitize_profile_segment(profile_id: Option<&str>) -> String {
    match profile_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect(),
        None => "system".to_string(),
    }
}

/// Per-agent + profile CLI config root:
/// `{OPENAB_HOME|~/.openab}/cli/{agent_type}/{profile_or_system}/`.
///
/// This directory is what we point Claude's `CLAUDE_CONFIG_DIR` and Codex's
/// `CODEX_HOME` at (verified against public docs; not re-verified against a
/// live CLI binary in this PR).
pub fn cli_config_dir(agent_type: &str, profile_id: Option<&str>) -> Result<PathBuf> {
    Ok(openab_home_dir()?
        .join("cli")
        .join(agent_type)
        .join(sanitize_profile_segment(profile_id)))
}

/// Ensure the isolation root exists (Codex requires `CODEX_HOME` to exist).
pub fn ensure_cli_config_dir(agent_type: &str, profile_id: Option<&str>) -> Result<PathBuf> {
    let dir = cli_config_dir(agent_type, profile_id)?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Claude settings path under the isolated `CLAUDE_CONFIG_DIR`.
///
/// Verified variable name: `CLAUDE_CONFIG_DIR` relocates `~/.claude`, so
/// `settings.json` lives at `$CLAUDE_CONFIG_DIR/settings.json`
/// (https://code.claude.com/docs/en/settings).
pub fn claude_settings_path_for(profile_id: Option<&str>) -> Result<PathBuf> {
    Ok(cli_config_dir("claude", profile_id)?.join("settings.json"))
}

/// Codex config path under the isolated `CODEX_HOME`.
///
/// Verified variable name: `CODEX_HOME` relocates `~/.codex`, so
/// `config.toml` lives at `$CODEX_HOME/config.toml`
/// (https://developers.openai.com/codex/config-basic).
pub fn codex_config_path_for(profile_id: Option<&str>) -> Result<PathBuf> {
    Ok(cli_config_dir("codex", profile_id)?.join("config.toml"))
}

pub fn claude_settings_path() -> Result<PathBuf> {
    claude_settings_path_for(None)
}

pub fn codex_config_path() -> Result<PathBuf> {
    codex_config_path_for(None)
}

/// Env vars that redirect a spawned CLI at the same isolation root OpenAB wrote.
///
/// - Claude: `CLAUDE_CONFIG_DIR` (community/docs-common; settings at `<dir>/settings.json`)
/// - Codex: `CODEX_HOME` (docs; config at `<dir>/config.toml`)
pub fn cli_isolation_env(
    agent_type: &str,
    profile_id: Option<&str>,
) -> Result<Vec<(String, String)>> {
    let dir = ensure_cli_config_dir(agent_type, profile_id)?;
    let dir_s = dir.display().to_string();
    match agent_type {
        "claude" => Ok(vec![("CLAUDE_CONFIG_DIR".into(), dir_s)]),
        "codex" => Ok(vec![("CODEX_HOME".into(), dir_s)]),
        _ => Ok(Vec::new()),
    }
}

/// Pure-ish spawn baseline: real `HOME` plus CLI isolation redirects.
///
/// `HOME` is never the isolation directory. Isolation goes only through the
/// vendor-specific env vars above.
pub fn build_spawn_home_and_cli_env(
    agent_type: Option<&str>,
    profile_id: Option<&str>,
) -> Result<(String, Vec<(String, String)>)> {
    let home = real_home_dir()?.display().to_string();
    let extra = match agent_type {
        Some(agent) => cli_isolation_env(agent, profile_id)?,
        None => Vec::new(),
    };
    Ok((home, extra))
}

#[cfg(test)]
pub(crate) fn test_home_env_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_home_override() {
        let _guard = test_home_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OPENAB_TEST_HOME", dir.path());
        assert_eq!(cli_home_dir().unwrap(), dir.path());
        assert_eq!(real_home_dir().unwrap(), dir.path());
        assert_eq!(openab_home_dir().unwrap(), dir.path());
        std::env::remove_var("OPENAB_TEST_HOME");
    }

    #[test]
    fn cli_config_dir_is_under_openab_home_per_profile() {
        let _guard = test_home_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OPENAB_TEST_HOME", dir.path());
        let a = cli_config_dir("claude", Some("p1")).unwrap();
        let b = cli_config_dir("claude", Some("p2")).unwrap();
        assert_eq!(a, dir.path().join("cli/claude/p1"));
        assert_eq!(b, dir.path().join("cli/claude/p2"));
        assert_ne!(a, b);
        std::env::remove_var("OPENAB_TEST_HOME");
    }

    #[test]
    fn spawn_env_keeps_home_real_and_sets_isolation_var() {
        let _guard = test_home_env_lock();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OPENAB_TEST_HOME", dir.path());
        let (home, extra) =
            build_spawn_home_and_cli_env(Some("claude"), Some("profile-a")).unwrap();
        assert_eq!(home, dir.path().display().to_string());
        assert!(!home.contains("/cli/claude/"));
        assert_eq!(
            extra,
            vec![(
                "CLAUDE_CONFIG_DIR".into(),
                dir.path()
                    .join("cli/claude/profile-a")
                    .display()
                    .to_string()
            )]
        );
        let (home2, extra2) =
            build_spawn_home_and_cli_env(Some("codex"), Some("profile-b")).unwrap();
        assert_eq!(home2, dir.path().display().to_string());
        assert_eq!(
            extra2,
            vec![(
                "CODEX_HOME".into(),
                dir.path().join("cli/codex/profile-b").display().to_string()
            )]
        );
        std::env::remove_var("OPENAB_TEST_HOME");
    }
}
