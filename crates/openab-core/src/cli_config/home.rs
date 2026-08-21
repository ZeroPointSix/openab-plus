use anyhow::{anyhow, Result};
use std::path::PathBuf;

/// Resolve the CLI home directory for file renderers and agent spawn.
/// Prefer `OPENAB_TEST_HOME` for tests, then `dirs::home_dir()`, never trust a
/// bare relative fallback that could overwrite the wrong tree.
pub fn cli_home_dir() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("OPENAB_TEST_HOME") {
        let path = path.trim();
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }
    dirs::home_dir().ok_or_else(|| anyhow!("unable to resolve home directory for CLI config"))
}

pub fn codex_config_path() -> Result<PathBuf> {
    Ok(cli_home_dir()?.join(".codex").join("config.toml"))
}

pub fn claude_settings_path() -> Result<PathBuf> {
    Ok(cli_home_dir()?.join(".claude").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_home_override() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OPENAB_TEST_HOME", dir.path());
        assert_eq!(cli_home_dir().unwrap(), dir.path());
        std::env::remove_var("OPENAB_TEST_HOME");
    }
}
