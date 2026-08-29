//! Multi-agent configuration: one block per agent in a single config file.
//!
//! Background (ZER-707 / ZER-866). Historically openab was single-agent per
//! process: `crate::config::Config` carried exactly one `[agent]` section and
//! every downstream consumer assumed it. Running several agents therefore meant
//! running several processes (one Deployment per agent under Helm). The local
//! daemon direction inverts that: one daemon on a machine drives every agent
//! CLI already installed there, so the agent set has to become data.
//!
//! This module owns that data model and nothing else. It deliberately does not
//! spawn processes or touch session pools; it produces a resolved, frozen
//! registry that those layers consume.
//!
//! Two rules from the convergence draft are load bearing here:
//!
//! 1. Explicit command wins. Discovery is a fallback, never the main path. The
//!    executable is resolved once at startup and frozen for the process
//!    lifetime, so a PATH change mid-run cannot silently repoint an agent.
//! 2. Routing stays simple. A default agent plus optional channel binding.
//!    There is intentionally no per-message agent directive.

use crate::config::{AgentConfig, ImageHandling};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Wire protocol used to drive an agent backend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AgentProtocol {
    /// Agent Client Protocol over stdio. The only protocol implemented today.
    #[default]
    Acp,
    /// One-shot process execution for agents that expose no ACP subcommand
    /// (Factory droid, for example). Reserved in the schema so such agents can
    /// be described now; NOT implemented -- building a registry that contains an
    /// enabled exec agent is rejected.
    Exec,
}

impl<'de> Deserialize<'de> for AgentProtocol {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "acp" => Ok(Self::Acp),
            "exec" => Ok(Self::Exec),
            other => Err(serde::de::Error::unknown_variant(other, &["acp", "exec"])),
        }
    }
}

impl std::fmt::Display for AgentProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Acp => write!(f, "acp"),
            Self::Exec => write!(f, "exec"),
        }
    }
}

/// When an agent re-reads the native config file we write for it.
///
/// Only on_start exists. The convergence draft also sketched a per_turn value,
/// but ZER-707 has ruled that native-config changes are guaranteed for NEW
/// SESSIONS ONLY and are never pushed into a live session. Accepting per_turn
/// would advertise a semantic the daemon does not implement, so it is rejected
/// with an explanatory error instead of being silently downgraded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NativeConfigReload {
    /// The agent picks up the written config when a new session starts.
    #[default]
    OnStart,
}

impl<'de> Deserialize<'de> for NativeConfigReload {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "on_start" | "on-start" => Ok(Self::OnStart),
            "per_turn" | "per-turn" => Err(serde::de::Error::custom(
                "native_config_reload = \"per_turn\" is not supported: native config changes are \
                 guaranteed for new sessions only and are never pushed into a live session. \
                 Use \"on_start\".",
            )),
            other => Err(serde::de::Error::unknown_variant(other, &["on_start"])),
        }
    }
}

/// Fallback search locations for an agent executable.
///
/// Only consulted when the agent block has no explicit command. Entries may
/// start with ~ (expanded against the daemon home) and may contain a single *
/// component, which is expanded one directory level deep so that
/// version-manager layouts such as ~/.nvm/versions/node/*/bin work without
/// hardcoding a version.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentDiscoverConfig {
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Cross-agent defaults, applied wherever an agent block leaves a value unset.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AgentDefaults {
    /// Give each session its own git worktree. Reserved for ZER-865; parsing it
    /// here keeps the config schema stable while the execution side lands.
    pub worktree: Option<bool>,
    /// Root directory under which per-session worktrees are created.
    pub worktree_dir: Option<String>,
    /// Environment variables every agent inherits from the daemon process.
    pub inherit_env: Vec<String>,
    /// Global discovery fallback, searched after an agent's own discover paths.
    ///
    /// This lives on [defaults] rather than a top-level [discover] because
    /// [[agents]] is a TOML array of tables: an [agents.discover] header after
    /// an [[agents]] block attaches to that ONE entry, so it cannot express a
    /// shared list. Both forms are supported and searched in order.
    pub discover_paths: Vec<String>,
}

/// One agent backend, as written in the config file.
#[derive(Debug, Clone, Deserialize)]
pub struct AgentEntry {
    /// Stable identifier. Used in pool keys, logs, doctor output and channel
    /// bindings, so it must be unique and non-empty.
    pub id: String,
    #[serde(default)]
    pub protocol: AgentProtocol,
    /// Explicit path or bare executable name. Highest-precedence source after a
    /// command-line override; when set, discovery is skipped entirely.
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Marks this entry as the fallback agent for traffic with no channel
    /// binding. At most one entry may set it.
    #[serde(default, rename = "default")]
    pub is_default: bool,
    /// Working directory for spawned sessions. Falls back to the legacy
    /// working_dir resolution when unset.
    pub workdir: Option<String>,
    /// Path to the agent's own native config file, which the daemon rewrites so
    /// model/provider changes take effect without rebuilding anything.
    pub native_config: Option<String>,
    #[serde(default)]
    pub native_config_reload: NativeConfigReload,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub inherit_env: Vec<String>,
    /// Inbound image handling. Falls back to the global default when unset.
    pub images: Option<ImageHandling>,
    /// Channels routed to this agent. Entries are opaque channel identifiers as
    /// the adapters report them. A channel may appear on at most one agent.
    #[serde(default)]
    pub channels: Vec<String>,
    /// Per-agent discovery fallback, searched before the global one.
    #[serde(default)]
    pub discover: AgentDiscoverConfig,
}

fn default_enabled() -> bool {
    true
}

/// Which precedence level supplied the executable actually used.
///
/// Reported verbatim by openab doctor: when an agent misbehaves, the first
/// question is always which binary was actually picked, and why that one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    /// Command-line override, highest precedence.
    CliOverride,
    /// Explicit command in the agent block.
    ExplicitConfig,
    /// Found by scanning discovery paths.
    DiscoverPath,
    /// Left as a bare name for the OS to resolve against PATH at spawn time.
    PathLookup,
}

impl std::fmt::Display for CommandSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CliOverride => write!(f, "cli-override"),
            Self::ExplicitConfig => write!(f, "explicit-config"),
            Self::DiscoverPath => write!(f, "discover-path"),
            Self::PathLookup => write!(f, "path-lookup"),
        }
    }
}

/// An agent whose executable has been resolved and frozen.
#[derive(Debug, Clone)]
pub struct ResolvedAgent {
    pub id: String,
    pub protocol: AgentProtocol,
    pub command: String,
    pub command_source: CommandSource,
    pub args: Vec<String>,
    pub workdir: Option<String>,
    pub native_config: Option<String>,
    pub native_config_reload: NativeConfigReload,
    pub env: HashMap<String, String>,
    pub inherit_env: Vec<String>,
    pub images: ImageHandling,
    pub channels: Vec<String>,
    pub is_default: bool,
}

impl ResolvedAgent {
    /// Project onto the legacy single-agent shape consumed by the session pool.
    ///
    /// command_source is intentionally collapsed: everything except a bare PATH
    /// lookup counts as explicit, matching what command_explicit means
    /// downstream (an operator named this binary, so do not second-guess it).
    pub fn to_agent_config(&self, fallback_working_dir: &str) -> AgentConfig {
        AgentConfig {
            command: self.command.clone(),
            args: self.args.clone(),
            working_dir: self
                .workdir
                .clone()
                .unwrap_or_else(|| fallback_working_dir.to_string()),
            env: self.env.clone(),
            inherit_env: self.inherit_env.clone(),
            images: self.images,
            command_explicit: self.command_source != CommandSource::PathLookup,
        }
    }

    /// Pool key for this agent, optionally scoped to a profile.
    ///
    /// Reuses the existing pool-key dimension rather than adding a new one, so
    /// the session pool, thread mapping, snapshots and event bus stay untouched.
    pub fn pool_key(&self, profile_id: Option<&str>) -> String {
        match profile_id {
            Some(profile) if !profile.is_empty() => {
                format!("agent:{}+profile:{}", self.id, profile)
            }
            _ => format!("agent:{}", self.id),
        }
    }
}

/// Frozen set of resolved agents plus the routing table.
#[derive(Debug, Clone)]
pub struct AgentRegistry {
    agents: Vec<ResolvedAgent>,
    default_index: usize,
    channel_index: HashMap<String, usize>,
}

impl AgentRegistry {
    /// Resolve and validate an agent set.
    ///
    /// cli_command_override corresponds to a command-line flag and applies to
    /// the default agent only; per-agent overrides belong in the config file.
    pub fn build(
        entries: &[AgentEntry],
        defaults: &AgentDefaults,
        global_images: ImageHandling,
        cli_command_override: Option<&str>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !entries.is_empty(),
            "no agents configured: add at least one [[agents]] block"
        );

        let mut seen_ids: HashSet<&str> = HashSet::new();
        for entry in entries {
            let id = entry.id.trim();
            anyhow::ensure!(!id.is_empty(), "an [[agents]] entry has an empty id");
            anyhow::ensure!(
                id == entry.id,
                "agent id '{}' has leading or trailing whitespace",
                entry.id
            );
            anyhow::ensure!(
                !id.contains(':') && !id.contains('+'),
                "agent id '{id}' must not contain ':' or '+' (reserved for pool keys)"
            );
            anyhow::ensure!(seen_ids.insert(id), "duplicate agent id '{id}'");
        }

        let enabled: Vec<&AgentEntry> = entries.iter().filter(|e| e.enabled).collect();
        anyhow::ensure!(
            !enabled.is_empty(),
            "every configured agent is disabled: enable at least one [[agents]] block"
        );

        for entry in &enabled {
            anyhow::ensure!(
                entry.protocol != AgentProtocol::Exec,
                "agent '{}' uses protocol = \"exec\", which is reserved but not implemented; \
                 set enabled = false or use protocol = \"acp\"",
                entry.id
            );
        }

        let default_ids: Vec<&str> = enabled
            .iter()
            .filter(|e| e.is_default)
            .map(|e| e.id.as_str())
            .collect();
        anyhow::ensure!(
            default_ids.len() <= 1,
            "more than one agent is marked default: {}",
            default_ids.join(", ")
        );

        let default_id = default_ids
            .first()
            .copied()
            .unwrap_or_else(|| enabled[0].id.as_str())
            .to_string();

        let mut agents = Vec::with_capacity(enabled.len());
        for entry in &enabled {
            let is_default_entry = entry.id == default_id;
            let override_for_entry = cli_command_override.filter(|_| is_default_entry);
            let (command, command_source) = resolve_command(entry, defaults, override_for_entry)?;

            let mut inherit_env = Vec::new();
            let mut seen_env = HashSet::new();
            for key in defaults
                .inherit_env
                .iter()
                .chain(entry.inherit_env.iter())
                .cloned()
            {
                if seen_env.insert(key.clone()) {
                    inherit_env.push(key);
                }
            }

            agents.push(ResolvedAgent {
                id: entry.id.clone(),
                protocol: entry.protocol,
                command,
                command_source,
                args: entry.args.clone(),
                workdir: entry.workdir.clone(),
                native_config: entry.native_config.clone(),
                native_config_reload: entry.native_config_reload,
                env: entry.env.clone(),
                inherit_env,
                images: entry.images.unwrap_or(global_images),
                channels: entry.channels.clone(),
                is_default: is_default_entry,
            });
        }

        let default_index = agents
            .iter()
            .position(|a| a.is_default)
            .expect("one enabled agent is always marked default");

        let mut channel_index = HashMap::new();
        for (index, agent) in agents.iter().enumerate() {
            for channel in &agent.channels {
                let channel = channel.trim();
                anyhow::ensure!(
                    !channel.is_empty(),
                    "agent '{}' has an empty channel binding",
                    agent.id
                );
                if let Some(previous) = channel_index.insert(channel.to_string(), index) {
                    anyhow::bail!(
                        "channel '{channel}' is bound to both '{}' and '{}'",
                        agents[previous].id,
                        agent.id
                    );
                }
            }
        }

        Ok(Self {
            agents,
            default_index,
            channel_index,
        })
    }

    /// Build a registry from the legacy single agent section, so a config
    /// written before the agents array keeps working unchanged.
    pub fn from_legacy_agent(config: &AgentConfig) -> anyhow::Result<Self> {
        let entry = AgentEntry {
            id: "default".to_string(),
            protocol: AgentProtocol::Acp,
            command: Some(config.command.clone()),
            args: config.args.clone(),
            enabled: true,
            is_default: true,
            workdir: Some(config.working_dir.clone()),
            native_config: None,
            native_config_reload: NativeConfigReload::OnStart,
            env: config.env.clone(),
            inherit_env: config.inherit_env.clone(),
            images: Some(config.images),
            channels: Vec::new(),
            discover: AgentDiscoverConfig::default(),
        };
        let mut registry = Self::build(&[entry], &AgentDefaults::default(), config.images, None)?;
        // A legacy command that came from an env-var default was never operator
        // asserted; preserve that distinction so doctor does not overstate it.
        if !config.command_explicit {
            registry.agents[0].command_source = CommandSource::PathLookup;
        }
        Ok(registry)
    }

    pub fn agents(&self) -> &[ResolvedAgent] {
        &self.agents
    }

    pub fn default_agent(&self) -> &ResolvedAgent {
        &self.agents[self.default_index]
    }

    pub fn get(&self, id: &str) -> Option<&ResolvedAgent> {
        self.agents.iter().find(|a| a.id == id)
    }

    /// Route a channel to an agent: explicit binding first, default otherwise.
    pub fn for_channel(&self, channel: &str) -> &ResolvedAgent {
        self.channel_index
            .get(channel)
            .map(|index| &self.agents[*index])
            .unwrap_or_else(|| self.default_agent())
    }
}

/// Apply the four-level precedence: CLI flag, explicit config, discovery, PATH.
fn resolve_command(
    entry: &AgentEntry,
    defaults: &AgentDefaults,
    cli_override: Option<&str>,
) -> anyhow::Result<(String, CommandSource)> {
    if let Some(command) = cli_override.map(str::trim).filter(|c| !c.is_empty()) {
        return Ok((command.to_string(), CommandSource::CliOverride));
    }

    if let Some(command) = entry
        .command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        return Ok((expand_home(command), CommandSource::ExplicitConfig));
    }

    for raw in entry
        .discover
        .paths
        .iter()
        .chain(defaults.discover_paths.iter())
    {
        for dir in expand_glob_dir(&expand_home(raw)) {
            let candidate = dir.join(&entry.id);
            if is_executable_file(&candidate) {
                return Ok((candidate.display().to_string(), CommandSource::DiscoverPath));
            }
        }
    }

    // Nothing matched. Fall back to the bare id and let the OS resolve it at
    // spawn time; doctor is responsible for reporting that this happened rather
    // than failing startup here, so one missing CLI cannot stop the daemon from
    // serving its other agents.
    Ok((entry.id.clone(), CommandSource::PathLookup))
}

fn expand_home(path: &str) -> String {
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_string();
    };
    let Ok(home) = crate::cli_config::cli_home_dir() else {
        return path.to_string();
    };
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        home.display().to_string()
    } else {
        home.join(rest).display().to_string()
    }
}

/// Expand at most one * component, one directory level deep.
///
/// Enough for version-manager layouts without pulling in a glob dependency or
/// walking arbitrarily deep trees during startup.
fn expand_glob_dir(pattern: &str) -> Vec<PathBuf> {
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return vec![PathBuf::from(pattern)];
    };
    let prefix = Path::new(prefix.trim_end_matches('/'));
    let suffix = suffix.trim_start_matches('/');
    let Ok(read_dir) = std::fs::read_dir(prefix) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in read_dir.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        out.push(if suffix.is_empty() {
            entry.path()
        } else {
            entry.path().join(suffix)
        });
    }
    out.sort();
    out
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Standalone view of the agent-related sections of a config document.
///
/// Deliberately parsed independently of `crate::config::Config`: the agent set
/// is the one part of the config the daemon must understand before it can do
/// anything else, and keeping it separate means adding it does not perturb the
/// large existing config struct or its deserialization order. Unknown keys are
/// ignored, so this can be pointed at a full config document.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AgentsFile {
    /// One block per agent: `[[agents]]`.
    pub agents: Vec<AgentEntry>,
    /// Cross-agent defaults: `[defaults]`.
    pub defaults: AgentDefaults,
}

impl AgentsFile {
    /// Parse the agent sections out of an already env-expanded config document.
    pub fn from_toml_str(expanded: &str, source: &str) -> anyhow::Result<Self> {
        toml::from_str(expanded)
            .map_err(|e| anyhow::anyhow!("failed to parse agent config from {source}: {e}"))
    }

    /// True when the document uses the agents array rather than the legacy
    /// single-agent section.
    pub fn has_agents(&self) -> bool {
        !self.agents.is_empty()
    }

    /// Build a registry from this document, falling back to the legacy
    /// single-agent section when no agents array is present.
    pub fn build_registry(
        &self,
        legacy: &AgentConfig,
        cli_command_override: Option<&str>,
    ) -> anyhow::Result<AgentRegistry> {
        if self.has_agents() {
            AgentRegistry::build(
                &self.agents,
                &self.defaults,
                legacy.images,
                cli_command_override,
            )
        } else {
            AgentRegistry::from_legacy_agent(legacy)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> AgentEntry {
        AgentEntry {
            id: id.to_string(),
            protocol: AgentProtocol::Acp,
            command: None,
            args: Vec::new(),
            enabled: true,
            is_default: false,
            workdir: None,
            native_config: None,
            native_config_reload: NativeConfigReload::OnStart,
            env: HashMap::new(),
            inherit_env: Vec::new(),
            images: None,
            channels: Vec::new(),
            discover: AgentDiscoverConfig::default(),
        }
    }

    fn build(entries: &[AgentEntry]) -> anyhow::Result<AgentRegistry> {
        AgentRegistry::build(entries, &AgentDefaults::default(), ImageHandling::Send, None)
    }

    fn make_executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("write stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod stub");
        }
        path
    }

    #[test]
    fn explicit_command_beats_discovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        make_executable(dir.path(), "codex");
        let mut e = entry("codex");
        e.command = Some("/opt/custom/codex-acp".into());
        e.discover.paths = vec![dir.path().display().to_string()];
        let reg = build(&[e]).expect("registry");
        let agent = reg.default_agent();
        assert_eq!(agent.command, "/opt/custom/codex-acp");
        assert_eq!(agent.command_source, CommandSource::ExplicitConfig);
    }

    #[test]
    fn cli_override_beats_explicit_command() {
        let mut e = entry("codex");
        e.command = Some("/opt/custom/codex-acp".into());
        let reg = AgentRegistry::build(
            &[e],
            &AgentDefaults::default(),
            ImageHandling::Send,
            Some("/tmp/from-flag"),
        )
        .expect("registry");
        assert_eq!(reg.default_agent().command, "/tmp/from-flag");
        assert_eq!(
            reg.default_agent().command_source,
            CommandSource::CliOverride
        );
    }

    #[test]
    fn discovery_finds_executable_when_command_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expected = make_executable(dir.path(), "claude");
        let mut e = entry("claude");
        e.discover.paths = vec![dir.path().display().to_string()];
        let reg = build(&[e]).expect("registry");
        assert_eq!(reg.default_agent().command, expected.display().to_string());
        assert_eq!(
            reg.default_agent().command_source,
            CommandSource::DiscoverPath
        );
    }

    #[test]
    fn discovery_skips_non_executable_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("claude"), "not executable").expect("write");
        let mut e = entry("claude");
        e.discover.paths = vec![dir.path().display().to_string()];
        let reg = build(&[e]).expect("registry");
        // Falls through to bare-name PATH lookup instead of picking a non-exec file.
        assert_eq!(reg.default_agent().command, "claude");
        assert_eq!(reg.default_agent().command_source, CommandSource::PathLookup);
    }

    #[test]
    fn global_discover_paths_are_searched_after_per_agent_ones() {
        let per_agent = tempfile::tempdir().expect("tempdir");
        let global = tempfile::tempdir().expect("tempdir");
        let expected = make_executable(global.path(), "opencode");
        let mut e = entry("opencode");
        e.discover.paths = vec![per_agent.path().display().to_string()];
        let defaults = AgentDefaults {
            discover_paths: vec![global.path().display().to_string()],
            ..AgentDefaults::default()
        };
        let reg = AgentRegistry::build(&[e], &defaults, ImageHandling::Send, None).expect("registry");
        assert_eq!(reg.default_agent().command, expected.display().to_string());
    }

    #[test]
    fn glob_component_expands_one_level() {
        let root = tempfile::tempdir().expect("tempdir");
        let versioned = root.path().join("v22.0.0").join("bin");
        std::fs::create_dir_all(&versioned).expect("mkdir");
        let expected = make_executable(&versioned, "gemini");
        let mut e = entry("gemini");
        e.discover.paths = vec![format!("{}/*/bin", root.path().display())];
        let reg = build(&[e]).expect("registry");
        assert_eq!(reg.default_agent().command, expected.display().to_string());
        assert_eq!(
            reg.default_agent().command_source,
            CommandSource::DiscoverPath
        );
    }

    #[test]
    fn missing_binary_does_not_fail_startup() {
        let reg = build(&[entry("nonexistent-agent")]).expect("registry still builds");
        assert_eq!(reg.default_agent().command_source, CommandSource::PathLookup);
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let err = build(&[entry("codex"), entry("codex")]).unwrap_err().to_string();
        assert!(err.contains("duplicate agent id"), "got: {err}");
    }

    #[test]
    fn empty_id_is_rejected() {
        let err = build(&[entry("")]).unwrap_err().to_string();
        assert!(err.contains("empty id"), "got: {err}");
    }

    #[test]
    fn reserved_characters_in_id_are_rejected() {
        for bad in ["a:b", "a+b"] {
            let err = build(&[entry(bad)]).unwrap_err().to_string();
            assert!(err.contains("reserved for pool keys"), "got: {err}");
        }
    }

    #[test]
    fn all_disabled_is_rejected() {
        let mut e = entry("codex");
        e.enabled = false;
        let err = build(&[e]).unwrap_err().to_string();
        assert!(err.contains("every configured agent is disabled"), "got: {err}");
    }

    #[test]
    fn empty_agent_set_is_rejected() {
        let err = build(&[]).unwrap_err().to_string();
        assert!(err.contains("no agents configured"), "got: {err}");
    }

    #[test]
    fn two_defaults_are_rejected() {
        let mut a = entry("codex");
        a.is_default = true;
        let mut b = entry("claude");
        b.is_default = true;
        let err = build(&[a, b]).unwrap_err().to_string();
        assert!(err.contains("more than one agent is marked default"), "got: {err}");
    }

    #[test]
    fn first_enabled_agent_is_default_when_unmarked() {
        let mut disabled = entry("disabled-one");
        disabled.enabled = false;
        let reg = build(&[disabled, entry("codex"), entry("claude")]).expect("registry");
        assert_eq!(reg.default_agent().id, "codex");
        assert_eq!(reg.agents().len(), 2);
    }

    #[test]
    fn explicit_default_marker_wins_over_ordering() {
        let a = entry("codex");
        let mut b = entry("claude");
        b.is_default = true;
        let reg = build(&[a, b]).expect("registry");
        assert_eq!(reg.default_agent().id, "claude");
    }

    #[test]
    fn channel_binding_routes_and_falls_back_to_default() {
        let mut a = entry("codex");
        a.channels = vec!["C_CODEX".into()];
        let mut b = entry("claude");
        b.is_default = true;
        b.channels = vec!["C_CLAUDE".into()];
        let reg = build(&[a, b]).expect("registry");
        assert_eq!(reg.for_channel("C_CODEX").id, "codex");
        assert_eq!(reg.for_channel("C_CLAUDE").id, "claude");
        assert_eq!(reg.for_channel("C_UNBOUND").id, "claude");
    }

    #[test]
    fn channel_bound_twice_is_rejected() {
        let mut a = entry("codex");
        a.channels = vec!["C_SHARED".into()];
        let mut b = entry("claude");
        b.channels = vec!["C_SHARED".into()];
        let err = build(&[a, b]).unwrap_err().to_string();
        assert!(err.contains("is bound to both"), "got: {err}");
    }

    #[test]
    fn enabled_exec_protocol_is_rejected_but_disabled_is_tolerated() {
        let mut droid = entry("droid");
        droid.protocol = AgentProtocol::Exec;
        let err = build(&[droid.clone()]).unwrap_err().to_string();
        assert!(err.contains("reserved but not implemented"), "got: {err}");

        droid.enabled = false;
        let reg = build(&[droid, entry("codex")]).expect("disabled exec agent is ignored");
        assert_eq!(reg.agents().len(), 1);
        assert_eq!(reg.default_agent().id, "codex");
    }

    #[test]
    fn per_turn_reload_is_rejected_with_explanation() {
        let err = toml::from_str::<AgentEntry>(
            "id = \"codex\"\nnative_config_reload = \"per_turn\"\n",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("new sessions only"), "got: {err}");
    }

    #[test]
    fn on_start_reload_parses() {
        let parsed: AgentEntry = toml::from_str(
            "id = \"codex\"\nnative_config_reload = \"on_start\"\n",
        )
        .expect("parse");
        assert_eq!(parsed.native_config_reload, NativeConfigReload::OnStart);
    }

    #[test]
    fn agents_array_parses_from_toml() {
        let parsed = AgentsFile::from_toml_str(
            r#"
[[agents]]
id = "codex"
command = "codex-acp"
default = true
channels = ["C_ONE"]

[[agents]]
id = "claude"
command = "claude-agent-acp"
enabled = false

[defaults]
discover_paths = ["~/.local/bin"]
inherit_env = ["HTTPS_PROXY"]
"#,
            "test",
        )
        .expect("parse agents file");
        assert_eq!(parsed.agents.len(), 2);
        assert_eq!(parsed.agents[0].id, "codex");
        assert!(parsed.agents[0].is_default);
        assert_eq!(parsed.agents[0].channels, vec!["C_ONE".to_string()]);
        assert!(!parsed.agents[1].enabled);
        assert_eq!(parsed.defaults.discover_paths.len(), 1);
        assert_eq!(parsed.defaults.inherit_env, vec!["HTTPS_PROXY".to_string()]);
    }

    #[test]
    fn per_agent_discover_table_attaches_to_preceding_block() {
        // Documents the TOML shape: [agents.discover] binds to the last
        // [[agents]] entry, which is why the shared list lives on [defaults].
        let parsed = AgentsFile::from_toml_str(
            r#"
[[agents]]
id = "codex"

[agents.discover]
paths = ["/opt/a"]

[[agents]]
id = "claude"
"#,
            "test",
        )
        .expect("parse agents file");
        assert_eq!(parsed.agents[0].discover.paths, vec!["/opt/a".to_string()]);
        assert!(parsed.agents[1].discover.paths.is_empty());
    }

    #[test]
    fn pool_key_scopes_by_agent_and_profile() {
        let reg = build(&[entry("codex")]).expect("registry");
        let agent = reg.default_agent();
        assert_eq!(agent.pool_key(None), "agent:codex");
        assert_eq!(agent.pool_key(Some("")), "agent:codex");
        assert_eq!(agent.pool_key(Some("deep")), "agent:codex+profile:deep");
    }

    #[test]
    fn to_agent_config_projects_onto_legacy_shape() {
        let mut e = entry("codex");
        e.command = Some("codex-acp".into());
        e.args = vec!["--flag".into()];
        e.env.insert("KEY".into(), "value".into());
        e.inherit_env = vec!["HTTPS_PROXY".into()];
        let defaults = AgentDefaults {
            inherit_env: vec!["HTTP_PROXY".into(), "HTTPS_PROXY".into()],
            ..AgentDefaults::default()
        };
        let reg = AgentRegistry::build(&[e], &defaults, ImageHandling::Skip, None).expect("registry");
        let cfg = reg.default_agent().to_agent_config("/fallback");
        assert_eq!(cfg.command, "codex-acp");
        assert_eq!(cfg.args, vec!["--flag".to_string()]);
        assert_eq!(cfg.working_dir, "/fallback");
        assert_eq!(cfg.env.get("KEY").map(String::as_str), Some("value"));
        assert!(cfg.command_explicit);
        assert_eq!(cfg.images, ImageHandling::Skip);
        // defaults first, per-agent appended, duplicates collapsed
        assert_eq!(
            cfg.inherit_env,
            vec!["HTTP_PROXY".to_string(), "HTTPS_PROXY".to_string()]
        );
    }

    #[test]
    fn workdir_overrides_fallback_working_dir() {
        let mut e = entry("codex");
        e.workdir = Some("/srv/project".into());
        let reg = build(&[e]).expect("registry");
        let cfg = reg.default_agent().to_agent_config("/fallback");
        assert_eq!(cfg.working_dir, "/srv/project");
    }

    #[test]
    fn path_lookup_is_not_reported_as_explicit() {
        let reg = build(&[entry("codex")]).expect("registry");
        let cfg = reg.default_agent().to_agent_config("/fallback");
        assert!(!cfg.command_explicit);
    }

    #[test]
    fn legacy_agent_section_still_builds_a_registry() {
        let legacy = AgentConfig {
            command: "kiro-cli".into(),
            args: vec!["acp".into()],
            working_dir: "/home/bot".into(),
            env: HashMap::new(),
            inherit_env: Vec::new(),
            images: ImageHandling::Send,
            command_explicit: true,
        };
        let reg = AgentRegistry::from_legacy_agent(&legacy).expect("registry");
        assert_eq!(reg.agents().len(), 1);
        let agent = reg.default_agent();
        assert_eq!(agent.id, "default");
        assert_eq!(agent.command, "kiro-cli");
        assert_eq!(agent.command_source, CommandSource::ExplicitConfig);
        assert_eq!(agent.pool_key(None), "agent:default");
        // Unbound channels route to it because it is the only agent.
        assert_eq!(reg.for_channel("anything").id, "default");
    }

    #[test]
    fn legacy_non_explicit_command_is_reported_as_path_lookup() {
        let legacy = AgentConfig {
            command: "openab-agent".into(),
            args: Vec::new(),
            working_dir: "/home/bot".into(),
            env: HashMap::new(),
            inherit_env: Vec::new(),
            images: ImageHandling::Send,
            command_explicit: false,
        };
        let reg = AgentRegistry::from_legacy_agent(&legacy).expect("registry");
        assert_eq!(reg.default_agent().command_source, CommandSource::PathLookup);
    }

    #[test]
    fn get_returns_agent_by_id() {
        let reg = build(&[entry("codex"), entry("claude")]).expect("registry");
        assert_eq!(reg.get("claude").map(|a| a.id.as_str()), Some("claude"));
        assert!(reg.get("missing").is_none());
    }
}