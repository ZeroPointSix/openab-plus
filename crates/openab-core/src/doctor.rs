//! Environment self-check for the local daemon (ZER-707 / ZER-867).
//!
//! The daemon ships a control plane, not an environment: agent CLIs are
//! whatever the machine already has installed. That makes "what is missing, and
//! what exactly did you pick" the single most important question an operator
//! asks, and answering it is this module's whole job.
//!
//! Two properties are deliberate and must not be "simplified" away.
//!
//! **Collect all, then report.** The normal run path is fail-fast: config
//! validation bails on the first problem. Doctor must not inherit that. Its
//! entire value is being useful when the config is broken, so every check runs
//! and every finding is reported in one pass. A doctor that dies on the first
//! error tells you nothing you did not already know.
//!
//! **Lenient parsing.** Doctor reads the agent sections through
//! `crate::agents::AgentsFile`, which ignores unknown keys, rather than the
//! strict whole-document `crate::config::Config` deserializer. An unrelated
//! typo elsewhere in the file must not stop doctor from telling you that your
//! codex binary is missing.

use crate::agents::{AgentDefaults, AgentRegistry, AgentsFile, CommandSource};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Severity of a single finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Checked and healthy.
    Ok,
    /// Works, but the operator should know. Does not affect the exit code.
    Warn,
    /// Broken. Drives a non-zero exit code.
    Fail,
}

impl Status {
    fn marker(self) -> &'static str {
        match self {
            Self::Ok => "ok  ",
            Self::Warn => "warn",
            Self::Fail => "FAIL",
        }
    }
}

/// One finding.
#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
    /// Actionable next step. Required for anything that is not Ok.
    pub hint: Option<String>,
}

impl Check {
    pub fn ok(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Ok,
            detail: detail.into(),
            hint: None,
        }
    }

    pub fn warn(
        name: impl Into<String>,
        detail: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            status: Status::Warn,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }

    pub fn fail(
        name: impl Into<String>,
        detail: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            status: Status::Fail,
            detail: detail.into(),
            hint: Some(hint.into()),
        }
    }
}

/// Full result of a doctor run.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn push(&mut self, check: Check) {
        self.checks.push(check);
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        let mut ok = 0;
        let mut warn = 0;
        let mut fail = 0;
        for check in &self.checks {
            match check.status {
                Status::Ok => ok += 1,
                Status::Warn => warn += 1,
                Status::Fail => fail += 1,
            }
        }
        (ok, warn, fail)
    }

    pub fn has_failures(&self) -> bool {
        self.checks.iter().any(|c| c.status == Status::Fail)
    }

    /// Process exit code: 0 when nothing failed, 1 otherwise.
    ///
    /// Warnings deliberately do NOT fail the run, so this is safe to gate an
    /// install script or CI step on without it flapping over cosmetic findings.
    pub fn exit_code(&self) -> i32 {
        if self.has_failures() {
            1
        } else {
            0
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "openab doctor");
        let _ = writeln!(out, "");
        for check in &self.checks {
            let _ = writeln!(out, "[{}] {}: {}", check.status.marker(), check.name, check.detail);
            if let Some(hint) = &check.hint {
                let _ = writeln!(out, "         -> {hint}");
            }
        }
        let (ok, warn, fail) = self.counts();
        let _ = writeln!(out, "");
        let _ = writeln!(out, "{ok} ok, {warn} warning(s), {fail} failure(s)");
        if fail > 0 {
            let _ = writeln!(
                out,
                "Missing dependencies are reported, never auto-installed: install the CLI yourself \
                 or point [[agents]] command at the right path."
            );
        }
        out
    }
}

/// Everything doctor needs, resolved by the caller.
pub struct DoctorInput {
    /// Config document, already env-expanded. None when it could not be read.
    pub config_text: Option<String>,
    /// Where the document came from, for messages.
    pub config_source: String,
    /// Read error, when the document could not be loaded at all.
    pub config_error: Option<String>,
}

/// Run every check and return a single report. Never returns Err: a failure to
/// even load the config is itself a reported check.
pub fn run(input: &DoctorInput) -> Report {
    let mut report = Report::default();

    check_home(&mut report);
    check_path(&mut report);
    check_git(&mut report);

    let Some(text) = input.config_text.as_deref() else {
        report.push(Check::fail(
            "config",
            format!(
                "cannot read {}: {}",
                input.config_source,
                input.config_error.as_deref().unwrap_or("unknown error")
            ),
            "create the config file or pass --config with the right path",
        ));
        return report;
    };

    let agents_file = match AgentsFile::from_toml_str(text, &input.config_source) {
        Ok(file) => {
            report.push(Check::ok(
                "config",
                format!("parsed {} ({} agent block(s))", input.config_source, file.agents.len()),
            ));
            file
        }
        Err(e) => {
            report.push(Check::fail(
                "config",
                format!("{e}"),
                "fix the TOML syntax or the agents/defaults sections",
            ));
            // No agent set to inspect, but the environment checks above still
            // stand on their own, so return what we have rather than bailing.
            return report;
        }
    };

    check_worktree_root(&mut report, &agents_file.defaults);

    if agents_file.agents.is_empty() {
        report.push(Check::warn(
            "agents",
            "no [[agents]] blocks; falling back to the legacy single [agent] section",
            "migrate to [[agents]] so this machine can serve more than one agent",
        ));
        return report;
    }

    let registry = match AgentRegistry::build(
        &agents_file.agents,
        &agents_file.defaults,
        crate::config::ImageHandling::default(),
        None,
    ) {
        Ok(registry) => registry,
        Err(e) => {
            report.push(Check::fail(
                "agents",
                format!("{e}"),
                "fix the [[agents]] blocks listed in the error",
            ));
            return report;
        }
    };

    report.push(Check::ok(
        "agents",
        format!(
            "{} enabled, default is '{}'",
            registry.agents().len(),
            registry.default_agent().id
        ),
    ));

    for agent in registry.agents() {
        check_agent_command(&mut report, &agent.id, &agent.command, agent.command_source);
        check_agent_native_config(&mut report, &agent.id, agent.native_config.as_deref());
        check_agent_isolation(&mut report, &agent.id);
        check_agent_workdir(&mut report, &agent.id, agent.workdir.as_deref());
    }

    report
}

fn check_home(report: &mut Report) {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => {
            let path = Path::new(&home);
            if path.is_dir() {
                report.push(Check::ok("HOME", home));
            } else {
                report.push(Check::fail(
                    "HOME",
                    format!("{home} is not a directory"),
                    "set HOME in the systemd unit to the service user's real home directory",
                ));
            }
        }
        _ => report.push(Check::fail(
            "HOME",
            "not set",
            "agent CLIs locate their own login state under HOME; set Environment=HOME in the unit",
        )),
    }
}

fn check_path(report: &mut Report) {
    let Ok(path) = std::env::var("PATH") else {
        report.push(Check::fail(
            "PATH",
            "not set",
            "systemd services do not inherit a login shell PATH; set Environment=PATH in the unit",
        ));
        return;
    };
    let entries: Vec<&str> = path.split(':').filter(|p| !p.is_empty()).collect();
    if entries.is_empty() {
        report.push(Check::fail(
            "PATH",
            "empty",
            "set Environment=PATH in the unit, including any version-manager bin directories",
        ));
        return;
    }
    let missing: Vec<&str> = entries
        .iter()
        .copied()
        .filter(|dir| !Path::new(dir).is_dir())
        .collect();
    if missing.is_empty() {
        report.push(Check::ok("PATH", format!("{} entries", entries.len())));
    } else {
        report.push(Check::warn(
            "PATH",
            format!(
                "{} entries, {} do not exist: {}",
                entries.len(),
                missing.len(),
                missing.join(", ")
            ),
            "harmless, but usually means the unit PATH was copied from another machine",
        ));
    }
}

fn check_git(report: &mut Report) {
    match resolve_in_path("git") {
        Some(path) => report.push(Check::ok("git", path.display().to_string())),
        None => report.push(Check::fail(
            "git",
            "not found on PATH",
            "install git: per-session worktrees need it",
        )),
    }
}

fn check_worktree_root(report: &mut Report, defaults: &AgentDefaults) {
    let Some(dir) = defaults.worktree_dir.as_deref() else {
        report.push(Check::warn(
            "worktree root",
            "[defaults] worktree_dir is not set",
            "set it (or OPENAB_WORK_DIR) so per-session worktrees do not land in HOME",
        ));
        return;
    };
    let path = PathBuf::from(expand_home_for_report(dir));
    if path.is_dir() {
        report.push(Check::ok("worktree root", path.display().to_string()));
    } else if path.exists() {
        report.push(Check::fail(
            "worktree root",
            format!("{} exists but is not a directory", path.display()),
            "point worktree_dir at a directory",
        ));
    } else {
        report.push(Check::warn(
            "worktree root",
            format!("{} does not exist yet", path.display()),
            "it will be created on first use; ExecStartPre in the unit template does this",
        ));
    }
}

fn check_agent_command(
    report: &mut Report,
    id: &str,
    command: &str,
    source: CommandSource,
) {
    let name = format!("agent '{id}' command");
    // Report the precedence level unconditionally: the first question when an
    // agent misbehaves is which binary was picked and why that one.
    let located = if command.contains('/') {
        let path = Path::new(command);
        is_executable(path).then(|| path.to_path_buf())
    } else {
        resolve_in_path(command)
    };

    match located {
        Some(path) => report.push(Check::ok(
            name,
            format!("{} (via {source})", path.display()),
        )),
        None if source == CommandSource::PathLookup => report.push(Check::fail(
            name,
            format!("'{command}' not found on PATH (via {source})"),
            format!(
                "install the CLI, or set command / discover paths for agent '{id}'; \
                 openab will not install it for you"
            ),
        )),
        None => report.push(Check::fail(
            name,
            format!("'{command}' is not an executable file (via {source})"),
            format!("fix the command path for agent '{id}'"),
        )),
    }
}

fn check_agent_native_config(report: &mut Report, id: &str, native_config: Option<&str>) {
    let name = format!("agent '{id}' native config");
    let Some(raw) = native_config else {
        report.push(Check::warn(
            name,
            "not configured",
            format!("set native_config for agent '{id}' to let openab write model/provider settings"),
        ));
        return;
    };
    let path = PathBuf::from(expand_home_for_report(raw));
    if path.is_file() {
        report.push(Check::ok(name, path.display().to_string()));
        return;
    }
    match path.parent() {
        Some(parent) if parent.is_dir() => report.push(Check::warn(
            name,
            format!("{} does not exist yet", path.display()),
            "it will be created on first write",
        )),
        Some(parent) => report.push(Check::fail(
            name,
            format!("parent directory {} does not exist", parent.display()),
            format!("create it, or point native_config for '{id}' somewhere that exists"),
        )),
        None => report.push(Check::fail(
            name,
            format!("{} has no parent directory", path.display()),
            "use an absolute path",
        )),
    }
}

/// Report whether per-session CLI config isolation is active.
///
/// Today it is not: config writes target one process-global path per CLI, so
/// concurrent sessions using the same CLI with different model/provider
/// settings overwrite each other and a rebuilt session can read another
/// session's values. Saying so out loud is the point — silently degrading is
/// exactly what this command exists to prevent.
fn check_agent_isolation(report: &mut Report, id: &str) {
    let name = format!("agent '{id}' config isolation");
    report.push(Check::warn(
        name,
        "not isolated: native config is written to one process-global path per CLI",
        "concurrent sessions with different model/provider settings can overwrite each other; \
         tracked as the session-level isolation work under ZER-707",
    ));
}

fn check_agent_workdir(report: &mut Report, id: &str, workdir: Option<&str>) {
    let Some(raw) = workdir else {
        return;
    };
    let name = format!("agent '{id}' workdir");
    let path = PathBuf::from(expand_home_for_report(raw));
    if path.is_dir() {
        report.push(Check::ok(name, path.display().to_string()));
    } else {
        report.push(Check::fail(
            name,
            format!("{} is not an existing directory", path.display()),
            format!("create it or fix workdir for agent '{id}'"),
        ));
    }
}

fn expand_home_for_report(path: &str) -> String {
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

/// Find a bare executable name on PATH.
pub fn resolve_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let candidate = Path::new(dir).join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn input(text: Option<&str>) -> DoctorInput {
        DoctorInput {
            config_text: text.map(str::to_string),
            config_source: "test.toml".to_string(),
            config_error: text.is_none().then(|| "no such file".to_string()),
        }
    }

    fn find<'a>(report: &'a Report, name_fragment: &str) -> &'a Check {
        report
            .checks
            .iter()
            .find(|c| c.name.contains(name_fragment))
            .unwrap_or_else(|| panic!("no check matching '{name_fragment}' in {:?}", report.checks))
    }

    #[test]
    fn exit_code_is_zero_without_failures() {
        let mut report = Report::default();
        report.push(Check::ok("a", "fine"));
        report.push(Check::warn("b", "meh", "do something"));
        assert_eq!(report.exit_code(), 0);
        assert!(!report.has_failures());
    }

    #[test]
    fn exit_code_is_one_with_any_failure() {
        let mut report = Report::default();
        report.push(Check::ok("a", "fine"));
        report.push(Check::fail("b", "broken", "fix it"));
        assert_eq!(report.exit_code(), 1);
        assert!(report.has_failures());
    }

    #[test]
    fn counts_split_by_status() {
        let mut report = Report::default();
        report.push(Check::ok("a", "x"));
        report.push(Check::ok("b", "x"));
        report.push(Check::warn("c", "x", "y"));
        report.push(Check::fail("d", "x", "y"));
        assert_eq!(report.counts(), (2, 1, 1));
    }

    #[test]
    fn non_ok_checks_always_carry_a_hint() {
        let report = run(&input(Some("[[agents]]\nid = \"definitely-not-installed-xyz\"\n")));
        for check in &report.checks {
            if check.status != Status::Ok {
                assert!(
                    check.hint.is_some(),
                    "check {} has no actionable hint",
                    check.name
                );
            }
        }
    }

    #[test]
    fn render_includes_markers_and_summary() {
        let mut report = Report::default();
        report.push(Check::fail("thing", "broken", "fix it"));
        let text = report.render();
        assert!(text.contains("[FAIL] thing: broken"));
        assert!(text.contains("-> fix it"));
        assert!(text.contains("1 failure(s)"));
        assert!(text.contains("never auto-installed"));
    }

    #[test]
    fn unreadable_config_is_a_reported_failure_not_a_panic() {
        let report = run(&input(None));
        let check = find(&report, "config");
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("cannot read"), "got {}", check.detail);
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn broken_toml_still_reports_environment_checks() {
        let report = run(&input(Some("this is not valid toml {{{")));
        // The config check fails...
        assert_eq!(find(&report, "config").status, Status::Fail);
        // ...but HOME / PATH / git were still inspected. This is the whole
        // reason doctor does not reuse the fail-fast run path.
        assert!(report.checks.iter().any(|c| c.name == "HOME"));
        assert!(report.checks.iter().any(|c| c.name == "PATH"));
        assert!(report.checks.iter().any(|c| c.name == "git"));
    }

    #[test]
    fn unknown_keys_elsewhere_do_not_break_the_agent_scan() {
        // A typo in an unrelated section must not stop doctor from telling you
        // about your agents.
        let text = "\
[some_unrelated_section]\n\
typo_key = 42\n\
\n\
[[agents]]\n\
id = \"codex\"\n\
command = \"/definitely/not/here/codex-acp\"\n";
        let report = run(&input(Some(text)));
        assert_eq!(find(&report, "config").status, Status::Ok);
        let cmd = find(&report, "agent 'codex' command");
        assert_eq!(cmd.status, Status::Fail);
        assert!(cmd.detail.contains("explicit-config"), "got {}", cmd.detail);
    }

    #[test]
    fn invalid_agent_set_is_reported_once_and_clearly() {
        let text = "[[agents]]\nid = \"a\"\n\n[[agents]]\nid = \"a\"\n";
        let report = run(&input(Some(text)));
        let check = find(&report, "agents");
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("duplicate agent id"), "got {}", check.detail);
    }

    #[test]
    fn legacy_config_without_agents_array_warns() {
        let report = run(&input(Some("[agent]\ncommand = \"kiro-cli\"\n")));
        let check = find(&report, "agents");
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("legacy"), "got {}", check.detail);
    }

    #[test]
    fn resolved_command_reports_precedence_level() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("codex");
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let text = format!(
            "[[agents]]\nid = \"codex\"\ncommand = \"{}\"\n",
            bin.display()
        );
        let report = run(&input(Some(&text)));
        let check = find(&report, "agent 'codex' command");
        assert_eq!(check.status, Status::Ok, "detail: {}", check.detail);
        assert!(check.detail.contains("explicit-config"), "got {}", check.detail);
    }

    #[test]
    fn missing_binary_is_a_failure_that_never_offers_to_install() {
        let report = run(&input(Some(
            "[[agents]]\nid = \"totally-absent-cli-9x8y\"\n",
        )));
        let check = find(&report, "command");
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("path-lookup"), "got {}", check.detail);
        let hint = check.hint.as_deref().unwrap_or_default();
        assert!(hint.contains("will not install it for you"), "got {hint}");
    }

    #[test]
    fn isolation_is_reported_as_not_isolated_today() {
        let report = run(&input(Some("[[agents]]\nid = \"codex\"\n")));
        let check = find(&report, "config isolation");
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("process-global"), "got {}", check.detail);
    }

    #[test]
    fn missing_workdir_is_a_failure() {
        let report = run(&input(Some(
            "[[agents]]\nid = \"codex\"\nworkdir = \"/nonexistent/path/xyz\"\n",
        )));
        let check = find(&report, "workdir");
        assert_eq!(check.status, Status::Fail);
    }

    #[test]
    fn worktree_root_missing_is_only_a_warning() {
        let report = run(&input(Some(
            "[defaults]\nworktree_dir = \"/nonexistent/worktrees\"\n\n[[agents]]\nid = \"codex\"\n",
        )));
        let check = find(&report, "worktree root");
        assert_eq!(check.status, Status::Warn);
    }

    #[test]
    fn resolve_in_path_finds_a_standard_binary() {
        // sh exists on every platform this daemon targets.
        assert!(resolve_in_path("sh").is_some());
        assert!(resolve_in_path("definitely-not-a-real-binary-zzz").is_none());
    }
}