//! Loop Controller — closed-loop agent automation.
//!
//! Manages stateful loops that orchestrate agents through cycles
//! (e.g., Issue → Code → Review → Fix → Review → Approve).
//!
//! Each loop definition is a `.toml` file in `~/.openab/loop/`.
//! Runtime state is persisted in `~/.openab/loop/state/`.

use crate::adapter::ChatAdapter;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

// ─── Loop Definition (from .toml files) ─────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct LoopDefinition {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub variant: String, // "issue" | "pr-review"
    pub channel_id: String, // Discord channel to create loop threads in
    pub trigger: TriggerConfig,
    pub roles: RolesConfig,
    pub limits: LimitsConfig,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub instructions: HashMap<String, InstructionConfig>,
    #[serde(default)]
    pub safety: SafetyConfig,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct TriggerConfig {
    pub labels: Vec<String>,
    pub repos: Vec<String>,
    #[serde(default)]
    pub exclude_labels: Vec<String>,
    #[serde(default)]
    pub base_branches: Vec<String>,
    #[serde(default)]
    pub exclude_authors: Vec<String>,
    #[serde(default)]
    pub exclude_paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RolesConfig {
    pub coder: String,
    pub reviewer: String,
    /// Pre-existing Discord thread ID. If set, the loop uses this thread
    /// instead of creating a new one. Empty or absent = auto-create.
    #[serde(default)]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LimitsConfig {
    #[serde(default = "default_max_iter")]
    pub max_iterations: u32,
    #[serde(default = "default_budget")]
    pub token_budget: u64,
    #[serde(default = "default_coding_timeout")]
    pub timeout_coding: String,
    #[serde(default = "default_review_timeout")]
    pub timeout_review: String,
    #[serde(default = "default_fix_timeout")]
    pub timeout_fix: String,
    #[serde(default = "default_total_timeout")]
    pub timeout_total: String,
    #[serde(default = "default_retry")]
    pub retry_per_step: u32,
    /// How long without any message before pinging the agent (e.g. "5m").
    #[serde(default = "default_liveness_timeout")]
    pub liveness_timeout: String,
    /// Max consecutive pings before declaring dead and retrying/escalating.
    #[serde(default = "default_max_pings")]
    pub max_pings: u32,
}

fn default_max_iter() -> u32 {
    3
}
fn default_budget() -> u64 {
    80000
}
fn default_coding_timeout() -> String {
    "30m".into()
}
fn default_review_timeout() -> String {
    "10m".into()
}
fn default_fix_timeout() -> String {
    "15m".into()
}
fn default_total_timeout() -> String {
    "90m".into()
}
fn default_retry() -> u32 {
    1
}
fn default_liveness_timeout() -> String {
    "5m".into()
}
fn default_max_pings() -> u32 {
    2
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DiscoveryConfig {
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_poll_interval")]
    pub poll_interval: String,
}

fn default_mode() -> String {
    "polling".into()
}
fn default_poll_interval() -> String {
    "60s".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstructionConfig {
    pub template: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SafetyConfig {
    #[serde(default)]
    pub path_denylist: Vec<String>,
    #[serde(default)]
    pub path_keywords: Vec<String>,
    #[serde(default)]
    pub escalation: Vec<EscalationRule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EscalationRule {
    pub category: String,
    pub risk_level: String,
    pub action: String,
}

// ─── Loop State (runtime, persisted to disk) ────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LoopState {
    Idle,
    Coding,
    Reviewing,
    Fixing,
    Approved,
    Escalated,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopInstance {
    pub loop_name: String,
    pub variant: String,
    pub work_item: WorkItem,
    pub state: LoopState,
    pub iteration: u32,
    pub max_iterations: u32,
    pub head_sha: Option<String>,
    pub pr_number: Option<u64>,
    pub token_used: u64,
    pub token_budget: u64,
    pub started_at: DateTime<Utc>,
    pub current_step_started: DateTime<Utc>,
    pub last_dispatch_id: Option<String>,
    pub thread_id: Option<String>,
    pub retries_this_step: u32,
    pub history: Vec<StepRecord>,
    /// Last time any message was observed in this loop's thread (passive liveness).
    #[serde(default = "Utc::now")]
    pub last_seen: DateTime<Utc>,
    /// How many consecutive pings sent without response.
    #[serde(default)]
    pub pings_sent: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkItem {
    pub kind: String, // "issue" | "pr"
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub step: String,
    pub dispatch_id: String,
    pub head_sha: Option<String>,
    pub result: String,
    pub tokens_consumed: u64,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub verdict: Option<String>,
    #[serde(default)]
    pub findings: Option<String>,
}

// ─── Loop Events ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum LoopEvent {
    /// New issue/PR discovered that matches a loop template
    WorkItemDiscovered {
        definition_name: String,
        item: WorkItem,
    },
    /// Agent posted a CompletionReport in the loop thread
    CompletionReport {
        thread_id: String,
        dispatch_id: String,
        agent_id: String,
        status: String, // "completed" | "failed" | "partial"
        pr_created: Option<u64>,
        head_sha: Option<String>,
        tokens_consumed: u64,
        verdict: Option<String>,
        findings: Option<String>,
    },
    /// New commits pushed (detected via polling)
    Synchronize {
        pr_number: u64,
        new_sha: String,
    },
    /// Step timed out
    Timeout {
        work_item_key: String,
    },
    /// Human override
    HumanOverride {
        thread_id: String,
        command: String,
    },
}

// ─── Loop Controller ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CompletionData {
    pub thread_id: String,
    pub dispatch_id: String,
    pub status: String,
    pub pr_created: Option<u64>,
    pub head_sha: Option<String>,
    pub tokens_consumed: u64,
    pub verdict: Option<String>,
    pub findings: Option<String>,
}

pub struct LoopController {
    definitions: Vec<LoopDefinition>,
    active_loops: HashMap<String, LoopInstance>, // key: "{loop_name}--{number}"
    active_threads: HashSet<String>,             // thread_ids with active loops
    state_dir: PathBuf,
    adapter: Arc<dyn ChatAdapter>,
    github_token: Option<String>,
}

impl LoopController {
    pub fn new(
        definitions: Vec<LoopDefinition>,
        state_dir: PathBuf,
        adapter: Arc<dyn ChatAdapter>,
        github_token: Option<String>,
    ) -> Self {
        std::fs::create_dir_all(&state_dir).ok();
        let mut ctrl = Self {
            definitions,
            active_loops: HashMap::new(),
            active_threads: HashSet::new(),
            state_dir,
            adapter,
            github_token,
        };
        ctrl.load_persisted_state();
        ctrl
    }

    /// Check if a thread_id belongs to an active loop (Layer 1 check).
    pub fn is_loop_thread(&self, thread_id: &str) -> bool {
        self.active_threads.contains(thread_id)
    }

    /// Update last_seen timestamp when any message is observed in a loop thread.
    /// This is the passive liveness signal — no special protocol needed.
    pub fn touch_thread(&mut self, thread_id: &str) {
        if let Some(inst) = self.active_loops.values_mut().find(|i| {
            i.thread_id.as_deref() == Some(thread_id)
        }) {
            inst.last_seen = Utc::now();
            inst.pings_sent = 0; // reset ping counter on any activity
        }
    }

    /// Try to parse a message as a CompletionReport (Layer 2 check).
    pub fn try_parse_report(content: &str) -> Option<LoopEvent> {
        const MARKER: &str = "<!-- oab-loop-report:";
        if !content.starts_with(MARKER) {
            return None;
        }
        let end = content.find("-->")?;
        let json_str = content[MARKER.len()..end].trim();
        let v: serde_json::Value = serde_json::from_str(json_str).ok()?;

        Some(LoopEvent::CompletionReport {
            thread_id: String::new(), // filled by caller
            dispatch_id: v["dispatch_id"].as_str().unwrap_or_default().to_string(),
            agent_id: v["agent_id"].as_str().unwrap_or_default().to_string(),
            status: v["status"].as_str().unwrap_or("completed").to_string(),
            pr_created: v["pr_created"].as_u64(),
            head_sha: v["head_sha"].as_str().map(String::from),
            tokens_consumed: v["tokens_consumed"].as_u64().unwrap_or(0),
            verdict: v["verdict"].as_str().map(String::from),
            findings: v["findings"].as_str().map(String::from),
        })
    }

    /// Main event processor.
    pub async fn consume(&mut self, event: LoopEvent) {
        match event {
            LoopEvent::WorkItemDiscovered {
                definition_name,
                item,
            } => {
                self.handle_new_work_item(&definition_name, item).await;
            }
            LoopEvent::CompletionReport {
                thread_id,
                dispatch_id,
                status,
                pr_created,
                head_sha,
                tokens_consumed,
                verdict,
                findings,
                ..
            } => {
                self.handle_completion(CompletionData {
                    thread_id,
                    dispatch_id,
                    status,
                    pr_created,
                    head_sha,
                    tokens_consumed,
                    verdict,
                    findings,
                })
                .await;
            }
            LoopEvent::Synchronize { pr_number, new_sha } => {
                self.handle_synchronize(pr_number, &new_sha).await;
            }
            LoopEvent::Timeout { work_item_key } => {
                self.handle_timeout(&work_item_key).await;
            }
            LoopEvent::HumanOverride {
                thread_id,
                command,
            } => {
                self.handle_human_override(&thread_id, &command).await;
            }
        }
    }

    /// Periodic tick — check timeouts, poll for new work items.
    pub async fn tick(&mut self) {
        // Check timeouts for all active loops
        let now = Utc::now();
        let timeout_keys: Vec<String> = self
            .active_loops
            .iter()
            .filter(|(_, inst)| {
                matches!(
                    inst.state,
                    LoopState::Coding | LoopState::Reviewing | LoopState::Fixing
                )
            })
            .filter(|(_, inst)| {
                let elapsed = now - inst.current_step_started;
                let timeout_secs = self.step_timeout_secs(inst);
                elapsed.num_seconds() > timeout_secs
            })
            .map(|(k, _)| k.clone())
            .collect();

        for key in timeout_keys {
            self.handle_timeout(&key).await;
        }

        // Passive liveness check — ping agents that have gone silent
        let liveness_actions: Vec<(String, bool)> = self
            .active_loops
            .iter()
            .filter(|(_, inst)| {
                matches!(
                    inst.state,
                    LoopState::Coding | LoopState::Reviewing | LoopState::Fixing
                )
            })
            .filter_map(|(k, inst)| {
                let def = self.definitions.iter().find(|d| d.name == inst.loop_name)?;
                let liveness_secs = parse_duration_secs(&def.limits.liveness_timeout);
                let elapsed = (now - inst.last_seen).num_seconds();
                if elapsed > liveness_secs {
                    Some((k.clone(), inst.pings_sent >= def.limits.max_pings))
                } else {
                    None
                }
            })
            .collect();

        for (key, exhausted) in liveness_actions {
            if exhausted {
                self.escalate(&key, "Agent unresponsive — no activity after repeated pings")
                    .await;
            } else {
                // Ping the agent
                let (role_id, thread_id) = {
                    let inst = &self.active_loops[&key];
                    let def = self.definitions.iter().find(|d| d.name == inst.loop_name);
                    let role = match inst.state {
                        LoopState::Coding | LoopState::Fixing => {
                            def.map(|d| d.roles.coder.clone()).unwrap_or_default()
                        }
                        _ => def.map(|d| d.roles.reviewer.clone()).unwrap_or_default(),
                    };
                    (role, inst.thread_id.clone())
                };
                if let Some(tid) = thread_id {
                    let ping_msg = format!("<@{}> 🏓 still working?", role_id);
                    let channel = crate::adapter::ChannelRef {
                        platform: "discord".into(),
                        channel_id: tid,
                        thread_id: None,
                        parent_id: None,
                        origin_event_id: None,
                    };
                    let _ = self.adapter.send_message(&channel, &ping_msg).await;
                }
                if let Some(inst) = self.active_loops.get_mut(&key) {
                    inst.pings_sent += 1;
                }
                self.persist_state(&key);
            }
        }

        // Discover new work items
        self.discover_new_items().await;
    }

    // ─── Internal: Work Item Discovery ──────────────────────────────────────

    async fn discover_new_items(&mut self) {
        for def in self.definitions.clone() {
            if !def.enabled {
                continue;
            }
            match def.variant.as_str() {
                "issue" => self.discover_issues(&def).await,
                "pr-review" => self.discover_prs(&def).await,
                _ => {}
            }
        }
    }

    async fn discover_issues(&mut self, def: &LoopDefinition) {
        for repo in &def.trigger.repos {
            let labels = def.trigger.labels.join(",");
            let url = format!(
                "https://api.github.com/repos/{}/issues?state=open&labels={}&sort=created&direction=desc&per_page=10",
                repo, labels
            );
            let issues = match self.github_get(&url).await {
                Some(v) => v,
                None => continue,
            };
            let arr = match issues.as_array() {
                Some(a) => a,
                None => continue,
            };
            for issue in arr {
                // Skip pull requests (GitHub lists PRs as issues too)
                if issue.get("pull_request").is_some() {
                    continue;
                }
                let number = issue["number"].as_u64().unwrap_or(0);
                if number == 0 {
                    continue;
                }
                // Skip if excluded label
                let issue_labels: Vec<&str> = issue["labels"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|l| l["name"].as_str()).collect())
                    .unwrap_or_default();
                if def
                    .trigger
                    .exclude_labels
                    .iter()
                    .any(|el| issue_labels.contains(&el.as_str()))
                {
                    continue;
                }
                let key = format!("{}--{}", def.name, number);
                if self.active_loops.contains_key(&key) {
                    continue;
                }
                // Check if already completed (state file exists with DONE)
                if self.is_done(&key) {
                    continue;
                }

                let item = WorkItem {
                    kind: "issue".into(),
                    repo: repo.clone(),
                    number,
                    title: issue["title"].as_str().unwrap_or_default().to_string(),
                    body: issue["body"].as_str().unwrap_or_default().to_string(),
                };
                info!(loop_name = %def.name, issue = number, "discovered new issue for loop");
                self.handle_new_work_item(&def.name, item).await;
            }
        }
    }

    async fn discover_prs(&mut self, def: &LoopDefinition) {
        for repo in &def.trigger.repos {
            let url = format!(
                "https://api.github.com/repos/{}/pulls?state=open&sort=created&direction=desc&per_page=10",
                repo
            );
            let prs = match self.github_get(&url).await {
                Some(v) => v,
                None => continue,
            };
            let arr = match prs.as_array() {
                Some(a) => a,
                None => continue,
            };
            for pr in arr {
                let number = pr["number"].as_u64().unwrap_or(0);
                if number == 0 {
                    continue;
                }
                // Check label match
                let pr_labels: Vec<&str> = pr["labels"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|l| l["name"].as_str()).collect())
                    .unwrap_or_default();
                if !def
                    .trigger
                    .labels
                    .iter()
                    .any(|l| pr_labels.contains(&l.as_str()))
                {
                    continue;
                }
                let key = format!("{}--{}", def.name, number);
                if self.active_loops.contains_key(&key) || self.is_done(&key) {
                    continue;
                }

                let item = WorkItem {
                    kind: "pr".into(),
                    repo: repo.clone(),
                    number,
                    title: pr["title"].as_str().unwrap_or_default().to_string(),
                    body: pr["body"].as_str().unwrap_or_default().to_string(),
                };
                info!(loop_name = %def.name, pr = number, "discovered new PR for loop");
                self.handle_new_work_item(&def.name, item).await;
            }
        }
    }

    // ─── Internal: State Transitions ────────────────────────────────────────

    async fn handle_new_work_item(&mut self, def_name: &str, item: WorkItem) {
        let def = match self.definitions.iter().find(|d| d.name == def_name) {
            Some(d) => d.clone(),
            None => return,
        };
        let key = format!("{}--{}", def.name, item.number);
        let now = Utc::now();

        let initial_state = match def.variant.as_str() {
            "issue" => LoopState::Coding,
            "pr-review" => LoopState::Reviewing,
            _ => LoopState::Coding,
        };

        let instance = LoopInstance {
            loop_name: def.name.clone(),
            variant: def.variant.clone(),
            work_item: item.clone(),
            state: initial_state.clone(),
            iteration: 0,
            max_iterations: def.limits.max_iterations,
            head_sha: None,
            pr_number: if item.kind == "pr" {
                Some(item.number)
            } else {
                None
            },
            token_used: 0,
            token_budget: def.limits.token_budget,
            started_at: now,
            current_step_started: now,
            last_dispatch_id: None,
            thread_id: None,
            retries_this_step: 0,
            history: vec![],
            last_seen: now,
            pings_sent: 0,
        };

        self.active_loops.insert(key.clone(), instance);
        self.persist_state(&key);

        // Resolve thread: use pre-configured thread_id or create a new one
        if let Some(ref tid) = def.roles.thread_id {
            if !tid.is_empty() {
                // Use existing thread
                if let Some(inst) = self.active_loops.get_mut(&key) {
                    inst.thread_id = Some(tid.clone());
                }
                self.active_threads.insert(tid.clone());
                self.persist_state(&key);
                info!(key, thread_id = %tid, "using pre-configured loop thread");
            }
        }
        // If no thread_id yet (not pre-configured or empty), create one
        if self.active_loops.get(&key).and_then(|i| i.thread_id.as_ref()).is_none() {
            let thread_title = format!("🔄 {} #{}", def.name, item.number);
            let channel = crate::adapter::ChannelRef {
                platform: "discord".into(),
                channel_id: def.channel_id.clone(),
                thread_id: None,
                parent_id: None,
                origin_event_id: None,
            };
            match self.adapter.send_message(&channel, &thread_title).await {
                Ok(anchor_msg) => {
                    match self
                        .adapter
                        .create_thread(&channel, &anchor_msg, &thread_title)
                        .await
                    {
                        Ok(thread_channel) => {
                            let tid = thread_channel.channel_id.clone();
                            if let Some(inst) = self.active_loops.get_mut(&key) {
                                inst.thread_id = Some(tid.clone());
                            }
                            self.active_threads.insert(tid);
                            self.persist_state(&key);
                            info!(key, thread_id = %thread_channel.channel_id, "created loop thread");
                        }
                        Err(e) => {
                            error!(key, error = %e, "failed to create loop thread");
                            self.escalate(&key, "Failed to create Discord thread for loop")
                                .await;
                            return;
                        }
                    }
                }
                Err(e) => {
                    error!(key, error = %e, "failed to send anchor message for loop thread");
                    self.escalate(&key, "Failed to send anchor message for loop thread")
                        .await;
                    return;
                }
            }
        }

        // Dispatch first step
        match initial_state {
            LoopState::Coding => {
                self.dispatch_coder_implement(&key, &def).await;
            }
            LoopState::Reviewing => {
                self.dispatch_reviewer(&key, &def).await;
            }
            _ => {}
        }
    }

    async fn handle_completion(
        &mut self,
        report: CompletionData,
    ) {
        let CompletionData {
            thread_id,
            dispatch_id,
            status,
            pr_created,
            head_sha,
            tokens_consumed,
            verdict,
            findings,
        } = report;
        let thread_id = &thread_id;
        let dispatch_id = &dispatch_id;
        let status = &status;
        // Find the loop instance by thread_id
        let key = match self.find_loop_by_thread(thread_id) {
            Some(k) => k,
            None => {
                debug!(thread_id, "completion report for unknown thread");
                return;
            }
        };

        let def = self
            .definitions
            .iter()
            .find(|d| d.name == self.active_loops[&key].loop_name)
            .cloned();

        // Verify dispatch_id matches
        {
            let inst = match self.active_loops.get(&key) {
                Some(i) => i,
                None => return,
            };
            if inst.last_dispatch_id.as_deref() != Some(dispatch_id) {
                debug!(
                    expected = ?inst.last_dispatch_id,
                    got = dispatch_id,
                    "dispatch_id mismatch, ignoring"
                );
                return;
            }
        }

        // Record step + determine next action
        enum NextAction {
            RetryStep(LoopState),
            EscalateNow(String),
            TransitionToReviewing,
            TransitionToFixing,
            TransitionToApproved,
            TransitionCodingDone(u64),  // pr_number
            Noop,
        }

        let next_action = {
            let inst = self.active_loops.get_mut(&key).unwrap();
            inst.token_used += tokens_consumed;
            inst.history.push(StepRecord {
                step: format!("{:?}", inst.state),
                dispatch_id: dispatch_id.to_string(),
                head_sha: head_sha.clone(),
                result: status.to_string(),
                tokens_consumed,
                timestamp: Utc::now(),
                verdict: verdict.clone(),
                findings: findings.clone(),
            });

            if status == "failed" {
                let max_retries = def.as_ref().map(|d| d.limits.retry_per_step).unwrap_or(1);
                if inst.retries_this_step < max_retries {
                    inst.retries_this_step += 1;
                    inst.current_step_started = Utc::now();
                    NextAction::RetryStep(inst.state.clone())
                } else {
                    NextAction::EscalateNow("Step failed after max retries".into())
                }
            } else {
                match inst.state.clone() {
                    LoopState::Coding => {
                        if let Some(pr) = pr_created {
                            inst.pr_number = Some(pr);
                            inst.head_sha = head_sha;
                            inst.state = LoopState::Reviewing;
                            inst.current_step_started = Utc::now();
                            inst.retries_this_step = 0;
                            NextAction::TransitionCodingDone(pr)
                        } else {
                            NextAction::EscalateNow("Coder completed but no PR was created".into())
                        }
                    }
                    LoopState::Reviewing => {
                        let v = verdict.as_deref().unwrap_or("");
                        if v.contains("LGTM") {
                            inst.state = LoopState::Approved;
                            NextAction::TransitionToApproved
                        } else if v.contains("CHANGES REQUESTED") || v.contains("CHANGES_REQUESTED") {
                            if inst.iteration >= inst.max_iterations {
                                NextAction::EscalateNow("Max iterations reached".into())
                            } else if inst.token_used >= inst.token_budget {
                                NextAction::EscalateNow("Token budget exhausted".into())
                            } else {
                                inst.state = LoopState::Fixing;
                                inst.current_step_started = Utc::now();
                                inst.retries_this_step = 0;
                                NextAction::TransitionToFixing
                            }
                        } else {
                            NextAction::EscalateNow(format!("Unparseable verdict: {}", v))
                        }
                    }
                    LoopState::Fixing => {
                        if let Some(sha) = head_sha {
                            inst.head_sha = Some(sha);
                        }
                        inst.iteration += 1;
                        inst.state = LoopState::Reviewing;
                        inst.current_step_started = Utc::now();
                        inst.retries_this_step = 0;
                        NextAction::TransitionToReviewing
                    }
                    _ => NextAction::Noop,
                }
            }
        };
        // Borrow of active_loops is now dropped

        self.persist_state(&key);

        // Execute the action
        match next_action {
            NextAction::RetryStep(state) => {
                if let Some(d) = &def {
                    match state {
                        LoopState::Coding => self.dispatch_coder_implement(&key, d).await,
                        LoopState::Reviewing => self.dispatch_reviewer(&key, d).await,
                        LoopState::Fixing => self.dispatch_coder_fix(&key, d).await,
                        _ => {}
                    }
                }
            }
            NextAction::EscalateNow(reason) => {
                self.escalate(&key, &reason).await;
            }
            NextAction::TransitionCodingDone(_) | NextAction::TransitionToReviewing => {
                if let Some(d) = &def {
                    let inst = &self.active_loops[&key];
                    let sha = inst.head_sha.clone().unwrap_or_default();
                    if !sha.is_empty()
                        && !self.check_ci_status(&inst.work_item.repo, &sha).await
                    {
                        self.escalate(&key, "CI checks failed — blocking review dispatch")
                            .await;
                    } else {
                        self.dispatch_reviewer(&key, d).await;
                    }
                }
            }
            NextAction::TransitionToFixing => {
                if let Some(d) = &def {
                    if self.validate_safety_policy(&key, d).await {
                        self.dispatch_coder_fix(&key, d).await;
                    } else {
                        self.escalate(&key, "Safety policy violation — requires human review").await;
                    }
                }
            }
            NextAction::TransitionToApproved => {
                self.notify_approved(&key).await;
            }
            NextAction::Noop => {}
        }
    }

    async fn handle_synchronize(&mut self, pr_number: u64, new_sha: &str) {
        // Find loop with this PR
        let key = self
            .active_loops
            .iter()
            .find(|(_, inst)| inst.pr_number == Some(pr_number))
            .map(|(k, _)| k.clone());

        let Some(key) = key else { return };

        let should_dispatch = {
            let inst = self.active_loops.get_mut(&key).unwrap();
            if inst.state == LoopState::Fixing {
                inst.head_sha = Some(new_sha.to_string());
                inst.iteration += 1;
                inst.state = LoopState::Reviewing;
                inst.current_step_started = Utc::now();
                inst.retries_this_step = 0;
                true
            } else {
                false
            }
        };

        if should_dispatch {
            self.persist_state(&key);
            let inst = &self.active_loops[&key];
            let repo = inst.work_item.repo.clone();
            let sha = inst.head_sha.clone().unwrap_or_default();
            let def = self
                .definitions
                .iter()
                .find(|d| d.name == inst.loop_name)
                .cloned();
            if let Some(d) = def {
                if !sha.is_empty() && !self.check_ci_status(&repo, &sha).await {
                    self.escalate(&key, "CI checks failed — blocking review dispatch")
                        .await;
                } else {
                    self.dispatch_reviewer(&key, &d).await;
                }
            }
        }
    }

    /// Check if required CI checks have passed for a given SHA.
    async fn check_ci_status(&self, repo: &str, sha: &str) -> bool {
        let url = format!(
            "https://api.github.com/repos/{}/commits/{}/check-runs",
            repo, sha
        );
        let response = match self.github_get(&url).await {
            Some(v) => v,
            None => return true, // Can't reach API — don't block
        };
        let check_runs = match response["check_runs"].as_array() {
            Some(runs) => runs,
            None => return true,
        };
        if check_runs.is_empty() {
            return true;
        }
        check_runs.iter().all(|run| {
            let status = run["status"].as_str().unwrap_or("");
            if status != "completed" {
                return false; // Still running — not ready for review yet
            }
            let conclusion = run["conclusion"].as_str().unwrap_or("");
            conclusion == "success" || conclusion == "skipped" || conclusion == "neutral"
        })
    }

    /// Validate safety policy before dispatching to coder.
    /// Checks BOTH reviewer findings AND the actual PR changed files against denylist.
    async fn validate_safety_policy(&self, key: &str, def: &LoopDefinition) -> bool {
        let inst = match self.active_loops.get(key) {
            Some(i) => i,
            None => return false,
        };

        // Layer 1: Check PR's actual changed files against denylist/keywords
        if let Some(pr_number) = inst.pr_number {
            let url = format!(
                "https://api.github.com/repos/{}/pulls/{}/files",
                inst.work_item.repo, pr_number
            );
            if let Some(files) = self.github_get(&url).await {
                if let Some(arr) = files.as_array() {
                    for file_entry in arr {
                        let filename = file_entry["filename"].as_str().unwrap_or("");
                        for pattern in &def.safety.path_denylist {
                            if glob_match::glob_match(pattern, filename) {
                                warn!(key, filename, pattern = pattern.as_str(), "safety policy: PR file matches path denylist");
                                return false;
                            }
                        }
                        for keyword in &def.safety.path_keywords {
                            if filename.contains(keyword.as_str()) {
                                warn!(key, filename, keyword = keyword.as_str(), "safety policy: PR file matches path keyword");
                                return false;
                            }
                        }
                    }
                }
            }
        }

        // Layer 2: Check reviewer findings for escalation rules
        let findings_str = match inst.history.iter().rev().find_map(|s| s.findings.clone()) {
            Some(f) => f,
            None => return true,
        };
        let findings: Vec<serde_json::Value> = match serde_json::from_str(&findings_str) {
            Ok(v) => v,
            Err(_) => return true,
        };
        for finding in &findings {
            let category = finding["category"].as_str().unwrap_or("");
            let risk_level = finding["risk_level"].as_str().unwrap_or("");
            for rule in &def.safety.escalation {
                let cat_match = rule.category == "*" || rule.category == category;
                let risk_match = rule.risk_level == "*" || rule.risk_level == risk_level;
                if cat_match && risk_match && rule.action == "escalate" {
                    warn!(key, category, risk_level, "safety policy: escalation triggered");
                    return false;
                }
            }
        }
        true
    }

    async fn handle_timeout(&mut self, key: &str) {
        let (should_retry, loop_name, state) = {
            let inst = match self.active_loops.get(key) {
                Some(i) => i,
                None => return,
            };
            let max_retries = self
                .definitions
                .iter()
                .find(|d| d.name == inst.loop_name)
                .map(|d| d.limits.retry_per_step)
                .unwrap_or(1);
            (
                inst.retries_this_step < max_retries,
                inst.loop_name.clone(),
                inst.state.clone(),
            )
        };

        if should_retry {
            if let Some(inst) = self.active_loops.get_mut(key) {
                inst.retries_this_step += 1;
                inst.current_step_started = Utc::now();
            }
            self.persist_state(key);
            info!(key, "step timeout — retrying");
            let def = self
                .definitions
                .iter()
                .find(|d| d.name == loop_name)
                .cloned();
            if let Some(d) = def {
                let key = key.to_string();
                match state {
                    LoopState::Coding => self.dispatch_coder_implement(&key, &d).await,
                    LoopState::Reviewing => self.dispatch_reviewer(&key, &d).await,
                    LoopState::Fixing => self.dispatch_coder_fix(&key, &d).await,
                    _ => {}
                }
            }
        } else {
            self.escalate(key, "Step timeout after max retries").await;
        }
    }

    async fn handle_human_override(&mut self, thread_id: &str, command: &str) {
        let key = match self.find_loop_by_thread(thread_id) {
            Some(k) => k,
            None => return,
        };
        match command.trim().to_lowercase().as_str() {
            "stop" => {
                let inst = self.active_loops.get_mut(&key).unwrap();
                inst.state = LoopState::Done;
                self.persist_state(&key);
                self.active_loops.remove(&key);
                info!(key, "loop stopped by human");
            }
            "resume" => {
                let inst = self.active_loops.get_mut(&key).unwrap();
                if inst.state == LoopState::Escalated {
                    // Re-enter the last active state (retry last step)
                    inst.state = LoopState::Reviewing; // safe default
                    inst.retries_this_step = 0;
                    inst.current_step_started = Utc::now();
                    self.persist_state(&key);
                    info!(key, "loop resumed by human");
                }
            }
            _ => {
                debug!(command, "unknown human override command");
            }
        }
    }

    // ─── Internal: Dispatch ─────────────────────────────────────────────────

    async fn dispatch_coder_implement(&mut self, key: &str, def: &LoopDefinition) {
        let dispatch_id = Uuid::new_v4().to_string();
        {
            let inst = self.active_loops.get_mut(key).unwrap();
            inst.last_dispatch_id = Some(dispatch_id.clone());
            inst.current_step_started = Utc::now();
        }
        self.persist_state(key);

        let inst = &self.active_loops[key];
        let template = def
            .instructions
            .get("coder_implement")
            .map(|i| i.template.as_str())
            .unwrap_or("Implement issue #{issue_number}: {issue_title}\n\n{issue_body}");

        let instruction = template
            .replace("{issue_number}", &inst.work_item.number.to_string())
            .replace("{issue_title}", &inst.work_item.title)
            .replace("{issue_body}", &inst.work_item.body)
            .replace("{dispatch_id}", &dispatch_id)
            .replace("{agent_id}", &def.roles.coder)
            .replace(
                "{thread_id}",
                inst.thread_id.as_deref().unwrap_or("unknown"),
            );

        let message = format!(
            "<@{}> 🔄 **Loop: {} #{}**\n\n{}",
            def.roles.coder, inst.work_item.kind, inst.work_item.number, instruction
        );
        self.send_to_loop_thread(key, &message).await;

        info!(
            key,
            agent = %def.roles.coder,
            dispatch_id,
            "dispatching coder (implement)"
        );
    }

    async fn dispatch_reviewer(&mut self, key: &str, def: &LoopDefinition) {
        let dispatch_id = Uuid::new_v4().to_string();
        {
            let inst = self.active_loops.get_mut(key).unwrap();
            inst.last_dispatch_id = Some(dispatch_id.clone());
            inst.current_step_started = Utc::now();
        }
        self.persist_state(key);

        let inst = &self.active_loops[key];
        let pr_url = format!(
            "https://github.com/{}/pull/{}",
            inst.work_item.repo,
            inst.pr_number.unwrap_or(0)
        );

        let template = def
            .instructions
            .get("reviewer")
            .map(|i| i.template.as_str())
            .unwrap_or("Review PR {pr_url} at `{head_sha}`.");

        let instruction = template
            .replace("{pr_url}", &pr_url)
            .replace(
                "{head_sha}",
                inst.head_sha.as_deref().unwrap_or("unknown"),
            )
            .replace("{iteration}", &inst.iteration.to_string())
            .replace("{max_iterations}", &inst.max_iterations.to_string())
            .replace("{dispatch_id}", &dispatch_id)
            .replace("{agent_id}", &def.roles.reviewer)
            .replace(
                "{thread_id}",
                inst.thread_id.as_deref().unwrap_or("unknown"),
            );

        let message = format!(
            "<@{}> 🔄 **Loop Review: PR #{}** (iteration {}/{})\n\n{}",
            def.roles.reviewer,
            inst.pr_number.unwrap_or(0),
            inst.iteration + 1,
            inst.max_iterations,
            instruction
        );
        self.send_to_loop_thread(key, &message).await;

        info!(
            key,
            agent = %def.roles.reviewer,
            dispatch_id,
            "dispatching reviewer"
        );
    }

    async fn dispatch_coder_fix(&mut self, key: &str, def: &LoopDefinition) {
        let dispatch_id = Uuid::new_v4().to_string();
        {
            let inst = self.active_loops.get_mut(key).unwrap();
            inst.last_dispatch_id = Some(dispatch_id.clone());
            inst.current_step_started = Utc::now();
        }
        self.persist_state(key);

        let inst = &self.active_loops[key];
        let pr_url = format!(
            "https://github.com/{}/pull/{}",
            inst.work_item.repo,
            inst.pr_number.unwrap_or(0)
        );

        let last_findings = inst
            .history
            .iter()
            .rev()
            .find_map(|s| s.findings.clone())
            .unwrap_or_default();

        let template = def
            .instructions
            .get("coder_fix")
            .map(|i| i.template.as_str())
            .unwrap_or("Fix findings on branch at `{head_sha}`.\n{findings}");

        let instruction = template
            .replace("{pr_url}", &pr_url)
            .replace(
                "{head_sha}",
                inst.head_sha.as_deref().unwrap_or("unknown"),
            )
            .replace("{branch}", "")
            .replace("{iteration}", &inst.iteration.to_string())
            .replace("{max_iterations}", &inst.max_iterations.to_string())
            .replace("{findings}", &last_findings)
            .replace("{dispatch_id}", &dispatch_id)
            .replace("{agent_id}", &def.roles.coder)
            .replace(
                "{thread_id}",
                inst.thread_id.as_deref().unwrap_or("unknown"),
            );

        let message = format!(
            "<@{}> 🔧 **Loop Fix: PR #{}** (iteration {}/{})\n\n{}",
            def.roles.coder,
            inst.pr_number.unwrap_or(0),
            inst.iteration + 1,
            inst.max_iterations,
            instruction
        );
        self.send_to_loop_thread(key, &message).await;

        info!(
            key,
            agent = %def.roles.coder,
            dispatch_id,
            "dispatching coder (fix)"
        );
    }

    async fn escalate(&mut self, key: &str, reason: &str) {
        if let Some(inst) = self.active_loops.get_mut(key) {
            inst.state = LoopState::Escalated;
        }
        self.persist_state(key);
        warn!(key, reason, "loop escalated to human");
        // TODO: Post escalation message to thread
    }

    async fn notify_approved(&self, key: &str) {
        info!(key, "loop completed — PR approved, awaiting merge");
        // TODO: Post approval notification to thread
    }

    // ─── Internal: Helpers ──────────────────────────────────────────────────

    async fn send_to_loop_thread(&self, key: &str, content: &str) {
        let thread_id = match self.active_loops.get(key).and_then(|i| i.thread_id.clone()) {
            Some(tid) => tid,
            None => {
                error!(key, "cannot send: no thread_id for loop");
                return;
            }
        };
        let channel = crate::adapter::ChannelRef {
            platform: "discord".into(),
            channel_id: thread_id,
            thread_id: None,
            parent_id: None,
            origin_event_id: None,
        };
        if let Err(e) = self.adapter.send_message(&channel, content).await {
            error!(key, error = %e, "failed to send message to loop thread");
        }
    }

    fn find_loop_by_thread(&self, thread_id: &str) -> Option<String> {
        self.active_loops
            .iter()
            .find(|(_, inst)| inst.thread_id.as_deref() == Some(thread_id))
            .map(|(k, _)| k.clone())
    }

    fn step_timeout_secs(&self, inst: &LoopInstance) -> i64 {
        let def = self
            .definitions
            .iter()
            .find(|d| d.name == inst.loop_name);
        let timeout_str = match (&inst.state, def) {
            (LoopState::Coding, Some(d)) => &d.limits.timeout_coding,
            (LoopState::Reviewing, Some(d)) => &d.limits.timeout_review,
            (LoopState::Fixing, Some(d)) => &d.limits.timeout_fix,
            _ => return 600, // 10m default
        };
        parse_duration_secs(timeout_str)
    }

    fn persist_state(&self, key: &str) {
        if let Some(inst) = self.active_loops.get(key) {
            let path = self.state_dir.join(format!("{}.json", key));
            if let Ok(json) = serde_json::to_string_pretty(inst) {
                if let Err(e) = std::fs::write(&path, json) {
                    error!(path = %path.display(), error = %e, "failed to persist loop state");
                }
            }
        }
    }

    fn load_persisted_state(&mut self) {
        let entries = match std::fs::read_dir(&self.state_dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(inst) = serde_json::from_str::<LoopInstance>(&content) {
                        if matches!(inst.state, LoopState::Done) {
                            continue; // don't reload completed loops
                        }
                        let key = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        if let Some(ref tid) = inst.thread_id {
                            self.active_threads.insert(tid.clone());
                        }
                        self.active_loops.insert(key, inst);
                    }
                }
            }
        }
        if !self.active_loops.is_empty() {
            info!(count = self.active_loops.len(), "restored active loops from disk");
        }
    }

    fn is_done(&self, key: &str) -> bool {
        let path = self.state_dir.join(format!("{}.json", key));
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(inst) = serde_json::from_str::<LoopInstance>(&content) {
                return inst.state == LoopState::Done;
            }
        }
        false
    }

    async fn github_get(&self, url: &str) -> Option<serde_json::Value> {
        let client = reqwest::Client::new();
        let mut req = client.get(url).header("User-Agent", "openab-loop-controller");
        if let Some(ref token) = self.github_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        match req.send().await {
            Ok(resp) if resp.status().is_success() => resp.json().await.ok(),
            Ok(resp) => {
                debug!(url, status = %resp.status(), "github API non-success");
                None
            }
            Err(e) => {
                debug!(url, error = %e, "github API request failed");
                None
            }
        }
    }
}

// ─── File Loader ────────────────────────────────────────────────────────────

/// Load all .toml loop definitions from a directory.
pub fn load_loop_definitions(dir: &Path) -> Vec<LoopDefinition> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    let mut defs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|e| e == "toml").unwrap_or(false) {
            match std::fs::read_to_string(&path) {
                Ok(content) => match toml::from_str::<LoopDefinition>(&content) {
                    Ok(def) => {
                        if def.enabled {
                            info!(name = %def.name, path = %path.display(), "loaded loop definition");
                        } else {
                            debug!(name = %def.name, "loop definition disabled, skipping");
                        }
                        defs.push(def);
                    }
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "failed to parse loop definition");
                    }
                },
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "failed to read loop file");
                }
            }
        }
    }
    defs
}

/// Create a shared LoopController instance (call once, share with Handler).
pub fn create_loop_controller(
    loop_dir: &Path,
    adapter: Arc<dyn ChatAdapter>,
    github_token: Option<String>,
) -> Option<Arc<Mutex<LoopController>>> {
    let defs = load_loop_definitions(loop_dir);
    if defs.is_empty() {
        info!("no loop definitions found, loop controller idle");
        return None;
    }
    let enabled_count = defs.iter().filter(|d| d.enabled).count();
    info!(total = defs.len(), enabled = enabled_count, "loop controller created");
    let state_dir = loop_dir.join("state");
    Some(Arc::new(Mutex::new(LoopController::new(
        defs, state_dir, adapter, github_token,
    ))))
}

/// Run the loop controller background tick task.
pub async fn run_loop_controller(
    controller: Arc<Mutex<LoopController>>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let poll_interval = tokio::time::Duration::from_secs(60);

    loop {
        tokio::select! {
            _ = tokio::time::sleep(poll_interval) => {
                let mut ctrl = controller.lock().await;
                ctrl.tick().await;
            }
            _ = shutdown_rx.changed() => {
                info!("loop controller shutting down");
                break;
            }
        }
    }
}

// ─── Utility ────────────────────────────────────────────────────────────────

fn parse_duration_secs(s: &str) -> i64 {
    let s = s.trim();
    if let Some(m) = s.strip_suffix('m') {
        m.parse::<i64>().unwrap_or(10) * 60
    } else if let Some(h) = s.strip_suffix('h') {
        h.parse::<i64>().unwrap_or(1) * 3600
    } else if let Some(sec) = s.strip_suffix('s') {
        sec.parse::<i64>().unwrap_or(60)
    } else {
        s.parse::<i64>().unwrap_or(600)
    }
}
