//! Named agent catalog (`[[agents]]` + `[discover]` + `default_agent`).
//!
//! When `[[agents]]` is empty/absent, OpenAB keeps the legacy singular `[agent]`
//! behaviour. When non-empty, the catalog is validated and the default named
//! agent is synthesized into `Config.agent` so `SessionPool::new(cfg.agent)`
//! stays unchanged for ZER-866a.

use crate::config::{AgentConfig, ImageHandling};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Agent transport protocol. Only `acp` is implemented in the first batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentProtocol {
    #[default]
    Acp,
    /// Reserved for a future exec-protocol spawn path (e.g. droid). Not wired.
    Exec,
}

impl AgentProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Acp => "acp",
            Self::Exec => "exec",
        }
    }
}

/// Where the effective command path came from after load-time resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandResolveSource {
    /// Absolute path written explicitly in config (or CLI override).
    ExplicitAbsolute,
    /// Relative/basename command found under `[discover].paths`.
    DiscoverPaths,
    /// Fallback via PATH (`which`-style). Not treated as the primary path.
    PathFallback,
    /// Could not resolve to an existing executable; original command retained.
    Unresolved,
}

impl CommandResolveSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitAbsolute => "explicit_absolute",
            Self::DiscoverPaths => "discover_paths",
            Self::PathFallback => "path_fallback",
            Self::Unresolved => "unresolved",
        }
    }
}

/// Optional discover roots used before falling back to PATH.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiscoverConfig {
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Raw TOML shape for one `[[agents]]` entry.
#[derive(Debug, Clone, Deserialize)]
pub struct NamedAgentRaw {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub protocol: AgentProtocol,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub working_dir: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub inherit_env: Vec<String>,
    pub images: Option<ImageHandling>,
    /// Optional Agent Profile id; not consumed in ZER-866a.
    pub profile: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Resolved named agent after validation + command resolution.
#[derive(Debug, Clone)]
pub struct NamedAgent {
    pub id: String,
    pub enabled: bool,
    pub protocol: AgentProtocol,
    /// Original command from config (before discover/PATH resolution).
    pub command: String,
    /// Absolute path when resolved; otherwise same as `command`.
    pub resolved_command: String,
    pub command_resolved: bool,
    pub resolve_source: CommandResolveSource,
    /// True when `command` was present in TOML (vs defaulted).
    pub command_explicit: bool,
    pub args: Vec<String>,
    pub working_dir: String,
    pub env: HashMap<String, String>,
    pub inherit_env: Vec<String>,
    pub images: ImageHandling,
    pub profile: Option<String>,
}

impl NamedAgent {
    /// Convert into the singular `AgentConfig` consumed by `SessionPool`.
    pub fn to_agent_config(&self) -> AgentConfig {
        AgentConfig {
            command: self.resolved_command.clone(),
            args: self.args.clone(),
            working_dir: self.working_dir.clone(),
            env: self.env.clone(),
            inherit_env: self.inherit_env.clone(),
            images: self.images,
            command_explicit: true,
        }
    }
}

/// Resolve a command once at config load.
///
/// Priority: absolute explicit path > `[discover].paths` basename lookup >
/// PATH fallback. Missing binaries do **not** fail load; they mark
/// `command_resolved = false` for later doctor (ZER-867).
pub fn resolve_command(
    command: &str,
    discover_paths: &[String],
) -> (String, bool, CommandResolveSource) {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return (command.to_string(), false, CommandResolveSource::Unresolved);
    }

    let path = Path::new(trimmed);
    if path.is_absolute() {
        if is_usable_executable(path) {
            return (
                trimmed.to_string(),
                true,
                CommandResolveSource::ExplicitAbsolute,
            );
        }
        // Absolute but missing: keep as-is, mark unresolved (no silent image pull).
        return (trimmed.to_string(), false, CommandResolveSource::Unresolved);
    }

    let basename = path.file_name().and_then(|s| s.to_str()).unwrap_or(trimmed);

    for root in discover_paths {
        let candidate = PathBuf::from(root).join(basename);
        if is_usable_executable(&candidate) {
            return (
                candidate.to_string_lossy().into_owned(),
                true,
                CommandResolveSource::DiscoverPaths,
            );
        }
    }

    // PATH is a fallback only — never the "guaranteed primary" path.
    if let Some(found) = which_on_path(basename) {
        return (
            found.to_string_lossy().into_owned(),
            true,
            CommandResolveSource::PathFallback,
        );
    }

    (trimmed.to_string(), false, CommandResolveSource::Unresolved)
}

fn is_usable_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(meta) => meta.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        // On Windows, treat existing files as runnable (extension checks are
        // environment-specific; doctor can refine later).
        true
    }
}

fn which_on_path(basename: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(basename);
        if is_usable_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat", "com"] {
                let with_ext = dir.join(format!("{basename}.{ext}"));
                if is_usable_executable(&with_ext) {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

/// Validate `[[agents]]`, resolve commands, and pick the default agent id.
///
/// Returns `Ok(None)` when `raw_agents` is empty (legacy `[agent]` only).
pub fn validate_and_resolve_agents(
    raw_agents: Vec<NamedAgentRaw>,
    default_agent: Option<&str>,
    discover: &DiscoverConfig,
    cli_agent_command: Option<&str>,
) -> anyhow::Result<Option<(Vec<NamedAgent>, String)>> {
    if raw_agents.is_empty() {
        return Ok(None);
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut agents: Vec<NamedAgent> = Vec::with_capacity(raw_agents.len());

    for raw in raw_agents {
        let id = raw.id.trim().to_string();
        anyhow::ensure!(!id.is_empty(), "[[agents]] entry has empty id");
        anyhow::ensure!(seen.insert(id.clone()), "duplicate [[agents]] id \"{id}\"");

        let command_explicit = raw.command.is_some();
        let configured_command = raw
            .command
            .clone()
            .unwrap_or_else(crate::config::default_agent_command);
        anyhow::ensure!(
            !configured_command.trim().is_empty(),
            "[[agents]] id \"{id}\" has empty command"
        );

        // Partial override rule mirrors `[agent]`: explicit command without args → [].
        let args = match (command_explicit, raw.args) {
            (_, Some(args)) => args,
            (true, None) => Vec::new(),
            (false, None) => crate::config::default_agent_args(),
        };

        let working_dir = raw
            .working_dir
            .unwrap_or_else(crate::config::default_working_dir);

        let (resolved_command, command_resolved, resolve_source) =
            resolve_command(&configured_command, &discover.paths);

        agents.push(NamedAgent {
            id,
            enabled: raw.enabled,
            protocol: raw.protocol,
            command: configured_command,
            resolved_command,
            command_resolved,
            resolve_source,
            command_explicit,
            args,
            working_dir,
            env: raw.env,
            inherit_env: raw.inherit_env,
            images: raw.images.unwrap_or_default(),
            profile: raw.profile,
        });
    }

    let enabled: Vec<&NamedAgent> = agents.iter().filter(|a| a.enabled).collect();
    anyhow::ensure!(
        !enabled.is_empty(),
        "[[agents]] is non-empty but no entry has enabled = true"
    );

    let default_id = if let Some(want) = default_agent.map(str::trim).filter(|s| !s.is_empty()) {
        let found = enabled.iter().find(|a| a.id == want);
        anyhow::ensure!(
            found.is_some(),
            "default_agent \"{want}\" does not match any enabled [[agents]] id"
        );
        want.to_string()
    } else {
        enabled[0].id.clone()
    };

    // Apply CLI override to the default agent's command only.
    if let Some(cli_cmd) = cli_agent_command.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(agent) = agents.iter_mut().find(|a| a.id == default_id) {
            let (resolved, ok, source) = resolve_command(cli_cmd, &discover.paths);
            agent.command = cli_cmd.to_string();
            agent.resolved_command = resolved;
            agent.command_resolved = ok;
            agent.resolve_source = source;
            agent.command_explicit = true;
        }
    }

    Ok(Some((agents, default_id)))
}

/// Error when the selected default agent uses an unimplemented protocol.
pub fn ensure_default_protocol_supported(agent: &NamedAgent) -> anyhow::Result<()> {
    match agent.protocol {
        AgentProtocol::Acp => Ok(()),
        AgentProtocol::Exec => anyhow::bail!(
            "default agent \"{}\" uses protocol = \"exec\", which is not implemented yet \
             (ZER-866a first batch does not spawn droid/exec); pick an acp agent or wait for a later PR",
            agent.id
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(unix)]
    fn write_exec(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, b"#!/bin/sh\necho ok\n").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[cfg(not(unix))]
    fn write_exec(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{name}.exe"));
        fs::write(&path, b"MZ").unwrap();
        path
    }

    #[test]
    fn agents_catalog_resolve_and_validate() {
        // No PATH mutation here — PATH fallback lives in config::agents_catalog_parse_all_scenarios
        // so cargo's parallel tests cannot race on process-global env.
        let tmp = tempfile::tempdir().unwrap();
        let discover_dir = tmp.path().join("discover");
        fs::create_dir_all(&discover_dir).unwrap();
        let discovered = write_exec(&discover_dir, "fake-claude");
        let abs = write_exec(tmp.path(), "explicit-bin");

        let (resolved, ok, src) = resolve_command(
            abs.to_str().unwrap(),
            &[discover_dir.to_string_lossy().into()],
        );
        assert!(ok);
        assert_eq!(src, CommandResolveSource::ExplicitAbsolute);
        assert_eq!(resolved, abs.to_string_lossy());

        let (resolved, ok, src) = resolve_command(
            "fake-claude",
            &[discover_dir.to_string_lossy().into_owned()],
        );
        assert!(ok);
        assert_eq!(src, CommandResolveSource::DiscoverPaths);
        assert_eq!(PathBuf::from(&resolved), discovered);

        let (resolved, ok, src) = resolve_command("definitely-missing-openab-xyz", &[]);
        assert!(!ok);
        assert_eq!(src, CommandResolveSource::Unresolved);
        assert_eq!(resolved, "definitely-missing-openab-xyz");

        assert!(
            validate_and_resolve_agents(vec![], None, &DiscoverConfig::default(), None)
                .unwrap()
                .is_none()
        );

        let raw = vec![
            NamedAgentRaw {
                id: "claude".into(),
                enabled: true,
                protocol: AgentProtocol::Acp,
                command: Some(abs.to_string_lossy().into_owned()),
                args: Some(vec!["acp".into()]),
                working_dir: Some("/tmp".into()),
                env: HashMap::new(),
                inherit_env: vec![],
                images: None,
                profile: Some("claude-default".into()),
            },
            NamedAgentRaw {
                id: "codex".into(),
                enabled: true,
                protocol: AgentProtocol::Acp,
                command: Some("fake-claude".into()),
                args: None,
                working_dir: None,
                env: HashMap::new(),
                inherit_env: vec![],
                images: None,
                profile: None,
            },
        ];
        let discover = DiscoverConfig {
            paths: vec![discover_dir.to_string_lossy().into_owned()],
        };
        let (agents, default_id) = validate_and_resolve_agents(raw, Some("codex"), &discover, None)
            .unwrap()
            .unwrap();
        assert_eq!(default_id, "codex");
        assert_eq!(agents.len(), 2);
        assert_eq!(
            agents[1].resolve_source,
            CommandResolveSource::DiscoverPaths
        );

        let err = validate_and_resolve_agents(
            vec![
                NamedAgentRaw {
                    id: "x".into(),
                    enabled: true,
                    protocol: AgentProtocol::Acp,
                    command: Some("/bin/sh".into()),
                    args: None,
                    working_dir: None,
                    env: HashMap::new(),
                    inherit_env: vec![],
                    images: None,
                    profile: None,
                },
                NamedAgentRaw {
                    id: "x".into(),
                    enabled: true,
                    protocol: AgentProtocol::Acp,
                    command: Some("/bin/sh".into()),
                    args: None,
                    working_dir: None,
                    env: HashMap::new(),
                    inherit_env: vec![],
                    images: None,
                    profile: None,
                },
            ],
            None,
            &DiscoverConfig::default(),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate"));

        let agent = NamedAgent {
            id: "droid".into(),
            enabled: true,
            protocol: AgentProtocol::Exec,
            command: "/bin/sh".into(),
            resolved_command: "/bin/sh".into(),
            command_resolved: true,
            resolve_source: CommandResolveSource::ExplicitAbsolute,
            command_explicit: true,
            args: vec![],
            working_dir: "/tmp".into(),
            env: HashMap::new(),
            inherit_env: vec![],
            images: ImageHandling::default(),
            profile: None,
        };
        let err = ensure_default_protocol_supported(&agent).unwrap_err();
        assert!(err.to_string().contains("not implemented"));
    }
}
