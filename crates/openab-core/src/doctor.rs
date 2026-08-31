//! Environment self-check for `openab doctor` (ZER-867).
//!
//! Pure checks only: no ACP spawn, no Discord/Slack connect, no image pull,
//! no CLI install. Missing tools and paths must fail explicitly.

use crate::agent_catalog::{AgentProtocol, CommandResolveSource, NamedAgent};
use crate::cli_config::cli_home_dir;
use crate::config::{AgentConfig, Config};
use std::path::{Path, PathBuf};

/// One row in the doctor checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorCheck {
    pub name: String,
    /// Hard pass when `true` and `warn` is false; hard fail when `false`.
    pub ok: bool,
    /// Soft warning (does not fail overall when `ok` is true).
    pub warn: bool,
    pub message: String,
}

impl DoctorCheck {
    pub fn ok(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: true,
            warn: false,
            message: message.into(),
        }
    }

    pub fn warn(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: true,
            warn: true,
            message: message.into(),
        }
    }

    pub fn fail(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ok: false,
            warn: false,
            message: message.into(),
        }
    }

    pub fn status_label(&self) -> &'static str {
        if !self.ok {
            "FAIL"
        } else if self.warn {
            "WARN"
        } else {
            "OK"
        }
    }
}

/// Aggregate doctor result.
#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    pub fn overall_ok(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }
}

/// Run all doctor checks against an already-parsed config.
///
/// Does not load secrets content, spawn agents, or mutate the filesystem beyond
/// optional write probes under CLI home dirs (`.claude` / `.codex`).
pub fn run_doctor(cfg: &Config) -> DoctorReport {
    let mut checks = Vec::new();

    check_agents(cfg, &mut checks);
    check_home(&mut checks);
    check_path_env(&mut checks);
    check_git(&mut checks);
    check_worktree(cfg, &mut checks);
    check_cli_homes(&mut checks);
    check_default_agent_emphasis(cfg, &mut checks);

    DoctorReport { checks }
}

fn check_agents(cfg: &Config, checks: &mut Vec<DoctorCheck>) {
    if cfg.agents.is_empty() {
        check_legacy_agent(&cfg.agent, checks);
        return;
    }

    let enabled: Vec<&NamedAgent> = cfg.agents.iter().filter(|a| a.enabled).collect();
    if enabled.is_empty() {
        checks.push(DoctorCheck::fail(
            "agents",
            "[[agents]] catalog is present but no enabled entries to check",
        ));
        return;
    }

    for agent in enabled {
        check_named_agent(agent, false, checks);
    }
}

fn check_legacy_agent(agent: &AgentConfig, checks: &mut Vec<DoctorCheck>) {
    let id = "legacy:[agent]";
    let command = agent.command.trim();
    let name = format!("agent:{id}:command");
    if command.is_empty() {
        checks.push(DoctorCheck::fail(
            name,
            format!(
                "{id}: [agent].command is empty — set an absolute path to an installed ACP CLI \
                 (OpenAB will not fetch images or install CLIs silently)"
            ),
        ));
        return;
    }

    let path = Path::new(command);
    if path.is_absolute() {
        if is_usable_executable(path) {
            checks.push(DoctorCheck::ok(
                name,
                format!("{id}: command `{command}` exists and is executable"),
            ));
        } else {
            checks.push(DoctorCheck::fail(
                name,
                missing_command_message(id, command, "explicit_absolute", command),
            ));
        }
        return;
    }

    // Relative / basename: re-resolve against current PATH for doctor (discover
    // already applied at load for catalog; legacy singular keeps raw command).
    match which_on_path(command) {
        Some(found) => {
            checks.push(DoctorCheck::warn(
                name,
                format!(
                    "{id}: command `{command}` resolved via PATH to `{}` — PATH is not the primary \
                     path; prefer an absolute command",
                    found.display()
                ),
            ));
        }
        None => {
            checks.push(DoctorCheck::fail(
                name,
                missing_command_message(id, command, "unresolved", command),
            ));
        }
    }
}

fn check_named_agent(agent: &NamedAgent, as_default: bool, checks: &mut Vec<DoctorCheck>) {
    let id = &agent.id;
    let label = if as_default {
        format!("default-agent:{id}")
    } else {
        format!("agent:{id}")
    };

    // 1) command non-empty
    let cmd_name = format!("{label}:command");
    if agent.command.trim().is_empty() {
        checks.push(DoctorCheck::fail(
            cmd_name,
            format!("{id}: configured command is empty"),
        ));
        return;
    }

    // 3) protocol=exec → FAIL (do not spawn)
    let proto_name = format!("{label}:protocol");
    match agent.protocol {
        AgentProtocol::Acp => {
            checks.push(DoctorCheck::ok(proto_name, format!("{id}: protocol=acp")));
        }
        AgentProtocol::Exec => {
            checks.push(DoctorCheck::fail(
                proto_name,
                format!(
                    "{id}: protocol=exec is not implemented yet — OpenAB will not spawn this agent; \
                     switch to protocol=acp or wait for a later release"
                ),
            ));
            // Still report command status below so catalog gaps are visible.
        }
    }

    // 2) resolved path exists + executable
    let resolved = agent.resolved_command.trim();
    let path = Path::new(resolved);
    let source = agent.resolve_source.as_str();
    if agent.command_resolved && is_usable_executable(path) {
        checks.push(DoctorCheck::ok(
            cmd_name.clone(),
            format!(
                "{id}: command `{}` → `{}` ({source}) exists and is executable",
                agent.command, agent.resolved_command
            ),
        ));
    } else {
        checks.push(DoctorCheck::fail(
            cmd_name,
            missing_command_message(id, &agent.command, source, &agent.resolved_command),
        ));
    }

    // 4) PATH fallback → WARN
    if agent.resolve_source == CommandResolveSource::PathFallback {
        checks.push(DoctorCheck::warn(
            format!("{label}:resolve_source"),
            format!(
                "{id}: command `{}` came from PATH fallback (`{}`) — PATH is not the primary path; \
                 prefer an absolute command or add the directory to [discover].paths",
                agent.command, agent.resolved_command
            ),
        ));
    }
}

fn missing_command_message(id: &str, configured: &str, source: &str, resolved: &str) -> String {
    format!(
        "{id}: agent command is missing or not executable — configured=`{configured}`, \
         resolved=`{resolved}`, resolve_source={source}. Install the CLI locally or set \
         `command` to an absolute path to an existing binary. OpenAB will not fetch container \
         images or install CLIs for you."
    )
}

fn check_home(checks: &mut Vec<DoctorCheck>) {
    let home = platform_home_env();
    match home {
        None => checks.push(DoctorCheck::fail(
            "env:HOME",
            "HOME (or USERPROFILE on Windows) is unset — agent CLI homes and worktrees need a home directory",
        )),
        Some(path) if path.as_os_str().is_empty() => checks.push(DoctorCheck::fail(
            "env:HOME",
            "HOME (or USERPROFILE on Windows) is empty",
        )),
        Some(path) if path.is_dir() => checks.push(DoctorCheck::ok(
            "env:HOME",
            format!("HOME is a directory: {}", path.display()),
        )),
        Some(path) => checks.push(DoctorCheck::fail(
            "env:HOME",
            format!(
                "HOME path does not exist or is not a directory: {}",
                path.display()
            ),
        )),
    }
}

fn platform_home_env() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn check_path_env(checks: &mut Vec<DoctorCheck>) {
    match std::env::var_os("PATH") {
        None => checks.push(DoctorCheck::fail(
            "env:PATH",
            "PATH is unset — cannot locate git or agent CLIs",
        )),
        Some(p) if p.is_empty() => checks.push(DoctorCheck::fail(
            "env:PATH",
            "PATH is empty — cannot locate git or agent CLIs",
        )),
        Some(_) => checks.push(DoctorCheck::ok("env:PATH", "PATH is set and non-empty")),
    }
}

fn check_git(checks: &mut Vec<DoctorCheck>) {
    match which_on_path("git") {
        Some(path) => checks.push(DoctorCheck::ok(
            "tool:git",
            format!(
                "git found at {} (needed for session worktrees / ZER-865; non-git workspaces can still use plain folders)",
                path.display()
            ),
        )),
        None => checks.push(DoctorCheck::fail(
            "tool:git",
            "git is not executable on PATH — ZER-865 session worktrees need git; \
             non-git workspaces can still create plain folders, but install git for full worktree support",
        )),
    }
}

fn check_worktree(cfg: &Config, checks: &mut Vec<DoctorCheck>) {
    // ZER-865 worktree config is not on this branch yet — surface explicitly.
    let _ = cfg;
    checks.push(DoctorCheck::warn(
        "worktree",
        "worktree 配置未接入 — [worktree] checks will land with ZER-865; skipped for now",
    ));
}

fn check_cli_homes(checks: &mut Vec<DoctorCheck>) {
    let home = match cli_home_dir() {
        Ok(h) => h,
        Err(e) => {
            checks.push(DoctorCheck::fail(
                "cli-home",
                format!("unable to resolve CLI home (OPENAB_TEST_HOME / dirs::home_dir): {e}"),
            ));
            return;
        }
    };

    if !home.is_dir() {
        checks.push(DoctorCheck::fail(
            "cli-home",
            format!(
                "CLI home is not a directory: {} — cannot create ~/.claude or ~/.codex",
                home.display()
            ),
        ));
        return;
    }

    for name in [".claude", ".codex"] {
        let dir = home.join(name);
        match ensure_dir_writable(&dir, &home) {
            Ok(()) => checks.push(DoctorCheck::ok(
                format!("cli-home:{name}"),
                format!(
                    "{} is writable (or creatable) under {}",
                    dir.display(),
                    home.display()
                ),
            )),
            Err(msg) => checks.push(DoctorCheck::fail(format!("cli-home:{name}"), msg)),
        }
    }
}

/// Ensure `dir` exists and is writable, or that it can be created under `home`.
/// Does not read credential files.
fn ensure_dir_writable(dir: &Path, home: &Path) -> Result<(), String> {
    if dir.exists() {
        if !dir.is_dir() {
            return Err(format!("{} exists but is not a directory", dir.display()));
        }
        return probe_writable(dir);
    }

    // Directory missing is OK if we can create it.
    match std::fs::create_dir_all(dir) {
        Ok(()) => {
            // Leave the empty agent home dir in place (benign); still probe write.
            probe_writable(dir)?;
            Ok(())
        }
        Err(e) => Err(format!(
            "cannot create {} under {}: {e}",
            dir.display(),
            home.display()
        )),
    }
}

fn probe_writable(dir: &Path) -> Result<(), String> {
    let probe = dir.join(".openab-doctor-write-probe");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(format!("directory {} is not writable: {e}", dir.display())),
    }
}

fn check_default_agent_emphasis(cfg: &Config, checks: &mut Vec<DoctorCheck>) {
    if let Some(default_id) = cfg.resolved_default_agent.as_deref() {
        if let Some(agent) = cfg.agents.iter().find(|a| a.id == default_id) {
            check_named_agent(agent, true, checks);
            return;
        }
        checks.push(DoctorCheck::fail(
            "default-agent",
            format!("resolved_default_agent `{default_id}` not found in catalog"),
        ));
        return;
    }

    // Legacy singular [agent] — re-emphasize as default.
    let agent = &cfg.agent;
    let command = agent.command.trim();
    let name = "default-agent:legacy:[agent]:command";
    if command.is_empty() {
        checks.push(DoctorCheck::fail(name, "default [agent].command is empty"));
        return;
    }
    let path = Path::new(command);
    if path.is_absolute() && is_usable_executable(path) {
        checks.push(DoctorCheck::ok(
            name,
            format!("default [agent] command `{command}` exists and is executable"),
        ));
    } else if let Some(found) = which_on_path(command) {
        checks.push(DoctorCheck::warn(
            name,
            format!(
                "default [agent] command `{command}` resolved via PATH to `{}` — prefer absolute path",
                found.display()
            ),
        ));
    } else if path.is_absolute() {
        checks.push(DoctorCheck::fail(
            name,
            missing_command_message("legacy:[agent]", command, "explicit_absolute", command),
        ));
    } else {
        checks.push(DoctorCheck::fail(
            name,
            missing_command_message("legacy:[agent]", command, "unresolved", command),
        ));
    }
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
        true
    }
}

fn which_on_path(basename: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let base = Path::new(basename);
    let name = base
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(basename);
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if is_usable_executable(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat", "com"] {
                let with_ext = dir.join(format!("{name}.{ext}"));
                if is_usable_executable(&with_ext) {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_catalog::NamedAgent;
    use crate::config::parse_config_str;
    use std::fs;
    use std::sync::Mutex;

    // Serialize env mutations across this module's scenarios.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

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

    fn messages_joined(report: &DoctorReport) -> String {
        report
            .checks
            .iter()
            .map(|c| format!("{}|{}|{}", c.status_label(), c.name, c.message))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn doctor_all_env_scenarios() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let saved_home = std::env::var_os("HOME");
        let saved_userprofile = std::env::var_os("USERPROFILE");
        let saved_path = std::env::var_os("PATH");
        let saved_test_home = std::env::var_os("OPENAB_TEST_HOME");
        let saved_discord = std::env::var_os("DISCORD_BOT_TOKEN");

        // Fake secret must never appear in doctor output/messages.
        std::env::set_var("DISCORD_BOT_TOKEN", "secret-token-should-never-leak");

        let tmp = tempfile::tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let good = write_exec(&bin_dir, "good-agent");
        let _ = write_exec(&bin_dir, "git"); // satisfy git check in isolated PATH

        let test_home = tmp.path().join("home");
        fs::create_dir_all(&test_home).unwrap();
        std::env::set_var("OPENAB_TEST_HOME", &test_home);
        std::env::set_var("HOME", &test_home);
        #[cfg(windows)]
        std::env::set_var("USERPROFILE", &test_home);

        let mut path_dirs = vec![bin_dir.clone()];
        if let Some(ref p) = saved_path {
            for d in std::env::split_paths(p) {
                path_dirs.push(d);
            }
        }
        std::env::set_var("PATH", std::env::join_paths(&path_dirs).unwrap());

        // --- Scenario: missing command file → overall fail, no docker/pull ---
        {
            let toml = r#"
[discord]
bot_token = "${DISCORD_BOT_TOKEN}"
[[agents]]
id = "missing"
command = "/definitely/missing/openab-doctor-bin"
"#;
            // Expand ${} like load path: parse_config_str expects already-expanded.
            let expanded = toml.replace("${DISCORD_BOT_TOKEN}", "secret-token-should-never-leak");
            let cfg = parse_config_str(&expanded, "test").unwrap();
            let report = run_doctor(&cfg);
            assert!(!report.overall_ok(), "missing binary must fail");
            let text = messages_joined(&report);
            let lower = text.to_lowercase();
            assert!(
                !lower.contains("docker")
                    && !lower.contains("docker pull")
                    && !lower.contains("image pull"),
                "doctor must not suggest docker/image pull: {text}"
            );
            // Still reject a suggestion that tells the user to pull something.
            assert!(
                !lower.contains("please pull")
                    && !lower.contains("run pull")
                    && !lower.contains("pull the"),
                "doctor must not advise pulling: {text}"
            );
            assert!(
                !text.contains("secret-token-should-never-leak"),
                "token leaked into doctor messages: {text}"
            );
            assert!(text.contains("missing"), "agent id should appear: {text}");
        }

        // --- Scenario: explicit absolute path exists → pass (agent checks) ---
        {
            let toml = format!(
                r#"
[discord]
bot_token = "t"
[[agents]]
id = "ok"
command = "{abs}"
"#,
                abs = good.display()
            );
            let cfg = parse_config_str(&toml, "test").unwrap();
            let report = run_doctor(&cfg);
            let text = messages_joined(&report);
            assert!(
                report
                    .checks
                    .iter()
                    .any(|c| { c.name.contains("agent:ok:command") && c.ok && !c.warn }),
                "absolute good agent should OK: {text}"
            );
            assert!(
                !text.contains("secret-token-should-never-leak"),
                "token leaked: {text}"
            );
        }

        // --- Scenario: catalog with one good + one bad → fail; both appear ---
        {
            let toml = format!(
                r#"
[discord]
bot_token = "t"
default_agent = "good"
[[agents]]
id = "good"
command = "{abs}"
[[agents]]
id = "bad"
command = "/no/such/openab-doctor-bad"
"#,
                abs = good.display()
            );
            let cfg = parse_config_str(&toml, "test").unwrap();
            let report = run_doctor(&cfg);
            assert!(!report.overall_ok());
            let text = messages_joined(&report);
            assert!(
                text.contains("good"),
                "good agent missing from report: {text}"
            );
            assert!(
                text.contains("bad"),
                "bad agent missing from report: {text}"
            );
            let lower = text.to_lowercase();
            assert!(
                !lower.contains("docker") && !lower.contains("docker pull"),
                "no docker/pull suggestion: {text}"
            );
        }

        // --- Scenario: PATH fallback → WARN ---
        {
            let path_bin = write_exec(&bin_dir, "path-fallback-agent");
            let _ = path_bin;
            let toml = r#"
[discord]
bot_token = "t"
[[agents]]
id = "pf"
command = "path-fallback-agent"
"#;
            let cfg = parse_config_str(toml, "test").unwrap();
            assert_eq!(
                cfg.agents[0].resolve_source,
                CommandResolveSource::PathFallback
            );
            let report = run_doctor(&cfg);
            assert!(
                report
                    .checks
                    .iter()
                    .any(|c| c.warn && c.name.contains("pf")),
                "PATH fallback should WARN: {}",
                messages_joined(&report)
            );
        }

        // --- Scenario: protocol=exec (non-default) → FAIL without spawn ---
        // Load rejects exec as *default*; inject a NamedAgent for doctor coverage.
        {
            let mut cfg = parse_config_str(
                &format!(
                    r#"
[discord]
bot_token = "t"
[[agents]]
id = "acp-ok"
command = "{abs}"
"#,
                    abs = good.display()
                ),
                "test",
            )
            .unwrap();
            cfg.agents.push(NamedAgent {
                id: "droid".into(),
                enabled: true,
                protocol: AgentProtocol::Exec,
                command: good.to_string_lossy().into_owned(),
                resolved_command: good.to_string_lossy().into_owned(),
                command_resolved: true,
                resolve_source: CommandResolveSource::ExplicitAbsolute,
                command_explicit: true,
                args: vec![],
                working_dir: "/tmp".into(),
                env: Default::default(),
                inherit_env: vec![],
                images: Default::default(),
                profile: None,
            });
            let report = run_doctor(&cfg);
            assert!(!report.overall_ok());
            assert!(
                report
                    .checks
                    .iter()
                    .any(|c| { !c.ok && c.name.contains("droid") && c.message.contains("exec") }),
                "exec protocol must FAIL: {}",
                messages_joined(&report)
            );
        }

        // --- Scenario: HOME points at deleted path → fail ---
        {
            let gone = tmp.path().join("deleted-home");
            fs::create_dir_all(&gone).unwrap();
            std::env::set_var("HOME", &gone);
            #[cfg(windows)]
            std::env::set_var("USERPROFILE", &gone);
            fs::remove_dir_all(&gone).unwrap();

            let toml = format!(
                r#"
[discord]
bot_token = "t"
[[agents]]
id = "ok"
command = "{abs}"
"#,
                abs = good.display()
            );
            let cfg = parse_config_str(&toml, "test").unwrap();
            // Keep OPENAB_TEST_HOME valid so cli-home is independent of HOME fail.
            let report = run_doctor(&cfg);
            assert!(
                report.checks.iter().any(|c| c.name == "env:HOME" && !c.ok),
                "missing HOME must FAIL: {}",
                messages_joined(&report)
            );
            // restore HOME for remaining checks / cleanup
            std::env::set_var("HOME", &test_home);
            #[cfg(windows)]
            std::env::set_var("USERPROFILE", &test_home);
        }

        // worktree not wired → WARN
        {
            let toml = format!(
                r#"
[discord]
bot_token = "t"
[[agents]]
id = "ok"
command = "{abs}"
"#,
                abs = good.display()
            );
            let cfg = parse_config_str(&toml, "test").unwrap();
            let report = run_doctor(&cfg);
            assert!(
                report
                    .checks
                    .iter()
                    .any(|c| c.name == "worktree" && c.warn && c.message.contains("未接入")),
                "expected worktree WARN: {}",
                messages_joined(&report)
            );
        }

        // Restore env
        match saved_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match saved_userprofile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
        match saved_path {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        match saved_test_home {
            Some(v) => std::env::set_var("OPENAB_TEST_HOME", v),
            None => std::env::remove_var("OPENAB_TEST_HOME"),
        }
        match saved_discord {
            Some(v) => std::env::set_var("DISCORD_BOT_TOKEN", v),
            None => std::env::remove_var("DISCORD_BOT_TOKEN"),
        }
    }
}
