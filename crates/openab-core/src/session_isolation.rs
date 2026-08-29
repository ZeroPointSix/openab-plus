//! Per-session CLI config isolation (ZER-707 / ZER-888).
//!
//! # The problem
//!
//! openab writes each agent's native config file so model / provider changes
//! take effect without rebuilding anything. Those writes target **one
//! process-global path per CLI** (`~/.codex/config.toml`, `~/.claude/settings.json`).
//! Correctness currently rests on holding a per-agent-type lock across the write
//! and the spawn, so the CLI reads the right values at startup.
//!
//! That holds only while sessions are effectively serialized. Once one daemon
//! drives several agents with several profiles, "same CLI, different model or
//! provider, concurrently" stops being an edge case:
//!
//! 1. session A writes the global file (model X), spawns
//! 2. session B writes the same file (model Y), spawns
//! 3. A is fine until it is rebuilt, recovered or restarted from the pool --
//!    then it reads **B's** values
//!
//! So "changing config takes effect for new sessions" is technically true while
//! being practically wrong: a new session gets whatever the *last writer* left
//! behind, not its own profile.
//!
//! # The approach
//!
//! Decision D2 on ZER-707: follow what cc-switch and Factory do, and in
//! particular **do not touch `HOME`**.
//!
//! That constraint is not cosmetic. `AcpConnection::spawn` deliberately passes the
//! real host `HOME` to the child so agent CLIs can find their own OAuth and login
//! files. Rewriting `HOME` to isolate config would break agent authentication --
//! which is exactly why Factory's own unit keeps `HOME=/root` real and moves only
//! its own state aside via `FACTORY_HOME`.
//!
//! Both first-batch CLIs expose a config-directory variable that redirects
//! **everything**, credentials included:
//!
//! | CLI | variable | redirects |
//! |-----|----------|-----------|
//! | codex | `CODEX_HOME` | `config.toml`, `auth.json`, … |
//! | claude | `CLAUDE_CONFIG_DIR` | `settings.json`, credentials, … |
//!
//! Pointing those at a bare per-session directory would isolate the settings but
//! also throw away the login state, so every session would come up
//! unauthenticated. The split this module implements:
//!
//! * files openab itself renders are written **per session**
//! * every other entry in the global config directory is **symlinked** in, so
//!   credentials and history stay shared
//!
//! Settings become per-session; identity stays machine-wide. Deliberately not in
//! scope: multi-user or multi-tenant credential isolation and custody.

use crate::cli_config::{self, ApplyRequest};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Whether a session actually got its own config directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationStatus {
    /// The agent runs against a per-session config directory.
    PerSession,
    /// No isolation available; the agent shares the process-global path and
    /// correctness still depends on the per-agent-type lock. Reported rather
    /// than hidden, because silently degrading is the failure mode doctor
    /// exists to prevent.
    Degraded,
}

impl std::fmt::Display for IsolationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PerSession => write!(f, "per-session"),
            Self::Degraded => write!(f, "degraded (process-global)"),
        }
    }
}

/// How one CLI stores its configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliLayout {
    /// Agent type as used by `cli_config`.
    pub agent_type: &'static str,
    /// Environment variable that relocates the whole config directory.
    pub config_dir_env: &'static str,
    /// Directory name under the daemon home.
    pub dir_name: &'static str,
    /// Files openab renders. Isolated per session; everything else is shared.
    pub managed_files: &'static [&'static str],
}

const LAYOUTS: &[CliLayout] = &[
    CliLayout {
        agent_type: "codex",
        config_dir_env: "CODEX_HOME",
        dir_name: ".codex",
        managed_files: &["config.toml"],
    },
    CliLayout {
        agent_type: "claude",
        config_dir_env: "CLAUDE_CONFIG_DIR",
        dir_name: ".claude",
        managed_files: &["settings.json"],
    },
];

/// Look up the layout for an agent type.
pub fn layout_for(agent_type: &str) -> Option<CliLayout> {
    LAYOUTS.iter().copied().find(|l| l.agent_type == agent_type)
}

/// Agent types that can be isolated.
pub fn supported_agent_types() -> Vec<&'static str> {
    LAYOUTS.iter().map(|l| l.agent_type).collect()
}

/// What was set up for one session, and what to hand the child process.
#[derive(Debug, Clone)]
pub struct IsolationPlan {
    pub status: IsolationStatus,
    pub agent_type: String,
    pub session_key: String,
    /// Per-session config directory, when isolated.
    pub dir: Option<PathBuf>,
    /// Environment to merge into the spawn. Never contains secret values.
    pub env: Vec<(String, String)>,
    /// Why isolation was skipped.
    pub reason: Option<String>,
    /// Entries symlinked from the global directory (credentials, history, …).
    pub shared: Vec<String>,
    /// Files written per session.
    pub isolated: Vec<String>,
}

impl IsolationPlan {
    fn degraded(agent_type: &str, session_key: &str, reason: impl Into<String>) -> Self {
        Self {
            status: IsolationStatus::Degraded,
            agent_type: agent_type.to_string(),
            session_key: session_key.to_string(),
            dir: None,
            env: Vec::new(),
            reason: Some(reason.into()),
            shared: Vec::new(),
            isolated: Vec::new(),
        }
    }

    pub fn is_isolated(&self) -> bool {
        self.status == IsolationStatus::PerSession
    }
}

/// Per-session config directories rooted under a daemon-owned location.
#[derive(Debug, Clone)]
pub struct Isolation {
    root: PathBuf,
}

impl Isolation {
    /// Root the session directories at an explicit path.
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolve the root from the environment.
    ///
    /// Prefers `OPENAB_HOME` (set by the systemd unit template so daemon state
    /// lives outside HOME), and otherwise falls back to a directory under the
    /// daemon home. Never derived from `HOME` rewriting.
    pub fn from_env() -> Self {
        if let Ok(base) = std::env::var("OPENAB_HOME") {
            let base = base.trim();
            if !base.is_empty() {
                return Self::with_root(Path::new(base).join("sessions"));
            }
        }
        let base = cli_config::cli_home_dir().unwrap_or_else(|_| PathBuf::from("/tmp"));
        Self::with_root(base.join(".openab").join("sessions"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Directory this session would use, without creating anything.
    pub fn session_dir(&self, agent_type: &str, session_key: &str) -> Option<PathBuf> {
        let layout = layout_for(agent_type)?;
        Some(
            self.root
                .join(sanitize_session_key(session_key))
                .join(layout.dir_name),
        )
    }

    /// Report what isolation would look like, without touching the filesystem.
    pub fn plan(&self, agent_type: &str, session_key: &str) -> IsolationPlan {
        let Some(layout) = layout_for(agent_type) else {
            return IsolationPlan::degraded(
                agent_type,
                session_key,
                format!(
                    "no config-directory variable known for agent type '{agent_type}'; \
                     supported: {}",
                    supported_agent_types().join(", ")
                ),
            );
        };
        let dir = self
            .root
            .join(sanitize_session_key(session_key))
            .join(layout.dir_name);
        IsolationPlan {
            status: IsolationStatus::PerSession,
            agent_type: agent_type.to_string(),
            session_key: session_key.to_string(),
            env: vec![(
                layout.config_dir_env.to_string(),
                dir.display().to_string(),
            )],
            dir: Some(dir),
            reason: None,
            shared: Vec::new(),
            isolated: layout
                .managed_files
                .iter()
                .map(|f| (*f).to_string())
                .collect(),
        }
    }

    /// Create the session directory and link shared state into it.
    ///
    /// Idempotent: re-preparing an existing session refreshes links without
    /// disturbing the per-session managed files.
    pub fn prepare(&self, agent_type: &str, session_key: &str) -> Result<IsolationPlan> {
        let Some(layout) = layout_for(agent_type) else {
            return Ok(self.plan(agent_type, session_key));
        };
        let mut plan = self.plan(agent_type, session_key);
        let dir = plan.dir.clone().expect("layout implies a directory");

        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create session config dir {}", dir.display()))?;
        harden_dir(&dir);

        let global = cli_config::cli_home_dir()
            .context("cannot resolve daemon home for shared CLI state")?
            .join(layout.dir_name);

        if let Ok(entries) = std::fs::read_dir(&global) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name_str) = name.to_str() else {
                    continue;
                };
                // Files openab renders are per-session; never link them, or the
                // session would write straight back into the shared file and we
                // would be back to the clobbering we are fixing.
                if layout.managed_files.contains(&name_str) {
                    continue;
                }
                let link = dir.join(name_str);
                if link.exists() || link.symlink_metadata().is_ok() {
                    plan.shared.push(name_str.to_string());
                    continue;
                }
                if link_shared(&entry.path(), &link).is_ok() {
                    plan.shared.push(name_str.to_string());
                }
            }
        }
        plan.shared.sort();
        Ok(plan)
    }

    /// Render this session's native config into its own directory.
    ///
    /// Reuses the existing renderer rather than duplicating it: the render runs
    /// under the same per-agent-type lock it always did, and the produced file
    /// is snapshotted into the session directory inside that critical section.
    /// The global file therefore degrades to a scratch render target, and each
    /// session reads an immutable copy of its own settings.
    pub async fn materialize(
        &self,
        session_key: &str,
        request: &ApplyRequest,
    ) -> Result<IsolationPlan> {
        let agent_type = request.agent_type.clone();
        let Some(layout) = layout_for(&agent_type) else {
            return Ok(self.plan(&agent_type, session_key));
        };
        let plan = self.prepare(&agent_type, session_key)?;
        let dir = plan.dir.clone().expect("layout implies a directory");

        let lock = cli_config::lock_for(&agent_type).await;
        let _guard = lock.lock().await;
        cli_config::apply_unlocked(request).await?;

        let global = cli_config::cli_home_dir()?.join(layout.dir_name);
        for managed in layout.managed_files {
            let from = global.join(managed);
            if !from.is_file() {
                continue;
            }
            let to = dir.join(managed);
            std::fs::copy(&from, &to).with_context(|| {
                format!(
                    "failed to snapshot {} into {}",
                    from.display(),
                    to.display()
                )
            })?;
            harden_file(&to);
        }
        Ok(plan)
    }

    /// Remove a session's directory. Returns whether anything was removed.
    ///
    /// Only ever touches the daemon-owned session tree. Shared state is reached
    /// through symlinks, and removing a symlink never touches its target, so
    /// credentials in the global directory cannot be destroyed here.
    pub fn cleanup(&self, session_key: &str) -> Result<bool> {
        let dir = self.root.join(sanitize_session_key(session_key));
        if !dir.exists() {
            return Ok(false);
        }
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("failed to remove session dir {}", dir.display()))?;
        Ok(true)
    }
}

/// Make a session key safe as a single path component.
///
/// Thread identifiers look like `slack:C0123:1699999999.123`, so they cannot be
/// used directly. Disallowed characters collapse to `-`, and an overlong key is
/// truncated with a short digest appended so two long keys sharing a prefix do
/// not collide.
pub fn sanitize_session_key(key: &str) -> String {
    const MAX: usize = 96;
    let mapped: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // Dots are allowed because real thread ids embed timestamps like 1699.5,
    // but a surviving ".." would still be a path-traversal component, so any
    // run of them collapses before trimming.
    let mut mapped = mapped;
    while mapped.contains("..") {
        mapped = mapped.replace("..", "-");
    }
    let trimmed = mapped.trim_matches(|c| c == '-' || c == '.');
    let safe = if trimmed.is_empty() { "session" } else { trimmed };
    if safe.len() <= MAX {
        return safe.to_string();
    }
    let digest = short_digest(key);
    let head: String = safe.chars().take(MAX - digest.len() - 1).collect();
    format!("{head}-{digest}")
}

fn short_digest(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(input.as_bytes());
    digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

#[cfg(unix)]
fn link_shared(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn link_shared(target: &Path, link: &Path) -> std::io::Result<()> {
    if target.is_dir() {
        return Ok(());
    }
    std::fs::copy(target, link).map(|_| ())
}

fn harden_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn harden_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex_request(model: &str) -> ApplyRequest {
        ApplyRequest {
            agent_type: "codex".into(),
            model: Some(model.into()),
            reasoning_effort: Some("high".into()),
            ..ApplyRequest::default()
        }
    }

    #[test]
    fn layout_is_known_for_first_batch_clis_only() {
        assert_eq!(layout_for("codex").map(|l| l.config_dir_env), Some("CODEX_HOME"));
        assert_eq!(
            layout_for("claude").map(|l| l.config_dir_env),
            Some("CLAUDE_CONFIG_DIR")
        );
        assert!(layout_for("opencode").is_none());
        assert!(layout_for("droid").is_none());
        let mut supported = supported_agent_types();
        supported.sort();
        assert_eq!(supported, vec!["claude", "codex"]);
    }

    #[test]
    fn managed_files_are_the_ones_openab_renders() {
        assert_eq!(layout_for("codex").unwrap().managed_files, &["config.toml"]);
        assert_eq!(layout_for("claude").unwrap().managed_files, &["settings.json"]);
    }

    #[test]
    fn session_keys_become_safe_path_components() {
        // Real thread ids carry colons and dots.
        assert_eq!(sanitize_session_key("slack:C0123:1699.5"), "slack-C0123-1699.5");
        assert_eq!(sanitize_session_key("plain"), "plain");
        // Path traversal cannot survive sanitization.
        let evil = sanitize_session_key("../../etc/passwd");
        assert!(!evil.contains('/'));
        assert!(!evil.contains(".."), "got {evil}");
        // Empty or all-invalid keys still yield a usable component.
        assert_eq!(sanitize_session_key(""), "session");
        assert_eq!(sanitize_session_key("///"), "session");
    }

    #[test]
    fn long_keys_sharing_a_prefix_do_not_collide() {
        let prefix = "a".repeat(200);
        let one = sanitize_session_key(&format!("{prefix}-one"));
        let two = sanitize_session_key(&format!("{prefix}-two"));
        assert_ne!(one, two);
        assert!(one.len() <= 96, "len {}", one.len());
        assert!(two.len() <= 96, "len {}", two.len());
    }

    #[test]
    fn unsupported_agent_type_degrades_with_a_reason() {
        let iso = Isolation::with_root("/tmp/openab-test-root");
        let plan = iso.plan("opencode", "s1");
        assert_eq!(plan.status, IsolationStatus::Degraded);
        assert!(!plan.is_isolated());
        assert!(plan.dir.is_none());
        assert!(plan.env.is_empty());
        let reason = plan.reason.unwrap_or_default();
        assert!(reason.contains("opencode"), "got {reason}");
        // The reason names what IS supported, so the operator learns the shape
        // of the gap rather than only that there is one.
        assert!(reason.contains("codex"), "got {reason}");
    }

    #[test]
    fn plan_exposes_the_config_dir_variable_and_no_secrets() {
        let iso = Isolation::with_root("/tmp/openab-test-root");
        let plan = iso.plan("codex", "slack:C1");
        assert_eq!(plan.status, IsolationStatus::PerSession);
        let (key, value) = &plan.env[0];
        assert_eq!(key, "CODEX_HOME");
        assert!(value.ends_with("/slack-C1/.codex"), "got {value}");
        assert_eq!(plan.isolated, vec!["config.toml".to_string()]);
    }

    #[test]
    fn from_env_prefers_openab_home_over_daemon_home() {
        std::env::set_var("OPENAB_HOME", "/var/lib/openab-test");
        let iso = Isolation::from_env();
        assert_eq!(iso.root(), Path::new("/var/lib/openab-test/sessions"));
        std::env::remove_var("OPENAB_HOME");
    }

    // All filesystem-and-env dependent behaviour lives in one test.
    //
    // OPENAB_TEST_HOME is process-global and cargo runs tests in parallel, so
    // these cannot be separate test functions without racing each other. Same
    // convention the rest of this crate already uses.
    #[tokio::test]
    async fn per_session_isolation_end_to_end() {
        let home = tempfile::tempdir().expect("home");
        let root = tempfile::tempdir().expect("root");
        std::env::set_var("OPENAB_TEST_HOME", home.path());

        // Pre-existing global codex state: a credential file openab does not
        // own, and a config file it does.
        let global = home.path().join(".codex");
        std::fs::create_dir_all(&global).expect("mkdir global");
        std::fs::write(global.join("auth.json"), "{\"token\":\"shared\"}").expect("auth");
        std::fs::write(global.join("config.toml"), "model = \"stale\"\n").expect("cfg");

        let iso = Isolation::with_root(root.path());

        // --- prepare links shared state but never the managed file ---
        let plan = iso.prepare("codex", "slack:A").expect("prepare");
        assert!(plan.is_isolated());
        let dir_a = plan.dir.clone().expect("dir");
        assert!(dir_a.is_dir());
        assert!(
            plan.shared.contains(&"auth.json".to_string()),
            "shared: {:?}",
            plan.shared
        );
        assert!(
            !plan.shared.contains(&"config.toml".to_string()),
            "managed file must not be shared, got {:?}",
            plan.shared
        );

        // Credentials are reachable through the session dir...
        let seen = std::fs::read_to_string(dir_a.join("auth.json")).expect("read auth");
        assert!(seen.contains("shared"));
        // ...as a link rather than a copy, so a re-login in the global dir is
        // picked up without re-preparing every session.
        #[cfg(unix)]
        assert!(std::fs::symlink_metadata(dir_a.join("auth.json"))
            .expect("symlink meta")
            .file_type()
            .is_symlink());
        // The stale global config was not linked in.
        assert!(!dir_a.join("config.toml").exists());

        // prepare is idempotent
        let again = iso.prepare("codex", "slack:A").expect("prepare again");
        assert!(again.shared.contains(&"auth.json".to_string()));

        // --- the regression this module exists to prevent ---
        // Two sessions, same CLI, different models. Before per-session dirs the
        // second write clobbered the first, so a rebuilt session A would come
        // back up reading model-beta.
        iso.materialize("slack:A", &codex_request("model-alpha"))
            .await
            .expect("materialize A");
        iso.materialize("slack:B", &codex_request("model-beta"))
            .await
            .expect("materialize B");

        let a = std::fs::read_to_string(dir_a.join("config.toml")).expect("read A");
        let dir_b = iso.session_dir("codex", "slack:B").expect("dir B");
        let b = std::fs::read_to_string(dir_b.join("config.toml")).expect("read B");

        assert!(a.contains("model-alpha"), "A got: {a}");
        assert!(!a.contains("model-beta"), "A was clobbered by B: {a}");
        assert!(b.contains("model-beta"), "B got: {b}");

        // Session B also sees the shared credentials.
        assert!(dir_b.join("auth.json").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir_a.join("config.toml"))
                .expect("meta")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "session config must not be world readable");
        }

        // --- cleanup removes only the session tree, never shared state ---
        assert!(iso.cleanup("slack:A").expect("cleanup"));
        assert!(!dir_a.exists());
        assert!(dir_b.exists(), "cleanup must not touch other sessions");
        assert!(
            global.join("auth.json").is_file(),
            "removing a symlink must never delete the credential it points at"
        );
        assert!(!iso.cleanup("slack:A").expect("cleanup again"));

        std::env::remove_var("OPENAB_TEST_HOME");
    }

    #[tokio::test]
    async fn materialize_on_unsupported_agent_type_reports_degraded_without_writing() {
        let root = tempfile::tempdir().expect("root");
        let iso = Isolation::with_root(root.path());
        let plan = iso
            .materialize(
                "s1",
                &ApplyRequest {
                    agent_type: "opencode".into(),
                    ..ApplyRequest::default()
                },
            )
            .await
            .expect("plan");
        assert_eq!(plan.status, IsolationStatus::Degraded);
        assert!(plan.dir.is_none());
        // Nothing was created for an agent we cannot isolate.
        assert!(std::fs::read_dir(root.path())
            .expect("read root")
            .next()
            .is_none());
    }
}