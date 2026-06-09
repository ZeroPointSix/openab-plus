use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

// ─── Config ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize)]
pub struct LoopConfig {
    pub enabled: bool,
    pub github_token: String,
    pub discord_token: String,
    pub discord_channel_id: String,
    pub reviewer_agent_id: String,
    pub coder_agent_id: String,
    pub escalation_user_id: String,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_step_timeout_secs")]
    pub step_timeout_secs: u64,
    #[serde(default)]
    pub conditions: LoopConditions,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct LoopConditions {
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub base_branch: Vec<String>,
    #[serde(default)]
    pub repos: Vec<String>,
}

fn default_poll_interval() -> u64 {
    60
}
fn default_max_iterations() -> u32 {
    3
}
fn default_step_timeout_secs() -> u64 {
    600
}

impl LoopConfig {
    pub fn from_env() -> Option<Self> {
        let enabled = std::env::var("LOOP_ENABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        if !enabled {
            return None;
        }

        let github_token = std::env::var("LOOP_GITHUB_TOKEN")
            .or_else(|_| std::env::var("GITHUB_TOKEN"))
            .ok()?;
        let discord_token = std::env::var("LOOP_DISCORD_TOKEN")
            .or_else(|_| std::env::var("DISCORD_BOT_TOKEN"))
            .ok()?;
        let discord_channel_id = std::env::var("LOOP_DISCORD_CHANNEL_ID").ok()?;
        let reviewer_agent_id = std::env::var("LOOP_REVIEWER_AGENT_ID").ok()?;
        let coder_agent_id = std::env::var("LOOP_CODER_AGENT_ID").ok()?;
        let escalation_user_id = std::env::var("LOOP_ESCALATION_USER_ID").ok()?;

        let poll_interval_secs = std::env::var("LOOP_POLL_INTERVAL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(default_poll_interval);
        let max_iterations = std::env::var("LOOP_MAX_ITERATIONS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(default_max_iterations);
        let step_timeout_secs = std::env::var("LOOP_STEP_TIMEOUT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(default_step_timeout_secs);

        let labels = std::env::var("LOOP_LABELS")
            .ok()
            .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();
        let base_branch = std::env::var("LOOP_BASE_BRANCHES")
            .ok()
            .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();
        let repos = std::env::var("LOOP_REPOS")
            .ok()
            .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();

        Some(Self {
            enabled,
            github_token,
            discord_token,
            discord_channel_id,
            reviewer_agent_id,
            coder_agent_id,
            escalation_user_id,
            poll_interval_secs,
            max_iterations,
            step_timeout_secs,
            conditions: LoopConditions {
                labels,
                base_branch,
                repos,
            },
        })
    }
}

// ─── State Machine ────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LoopState {
    Reviewing,
    Fixing,
    Approved,
    Escalated,
    Done,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoopInstance {
    pub repo: String,
    pub pr_number: u64,
    pub branch: String,
    pub state: LoopState,
    pub iteration: u32,
    pub max_iterations: u32,
    pub last_known_sha: String,
    pub step_started_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub retries: u32,
    pub thread_id: Option<String>,
}

pub type LoopStore = Arc<Mutex<HashMap<String, LoopInstance>>>;

fn loop_key(repo: &str, pr: u64) -> String {
    format!("{repo}#{pr}")
}

// ─── GitHub Polling ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GhPullRequest {
    number: u64,
    head: GhHead,
    base: GhBase,
    labels: Vec<GhLabel>,
    state: String,
}

#[derive(Debug, Deserialize)]
struct GhHead {
    sha: String,
    #[serde(rename = "ref")]
    ref_name: String,
}

#[derive(Debug, Deserialize)]
struct GhBase {
    #[serde(rename = "ref")]
    ref_name: String,
}

#[derive(Debug, Deserialize)]
struct GhLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GhComment {
    body: String,
    created_at: String,
    user: GhUser,
}

#[derive(Debug, Deserialize)]
struct GhUser {
    login: String,
}

async fn poll_prs(client: &Client, config: &LoopConfig, repo: &str) -> Vec<GhPullRequest> {
    let url = format!("https://api.github.com/repos/{repo}/pulls?state=open&per_page=30");
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", config.github_token))
        .header("User-Agent", "openab-loop-controller")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
        Ok(r) => {
            warn!(status = %r.status(), repo, "GitHub PR poll failed");
            Vec::new()
        }
        Err(e) => {
            warn!(err = %e, repo, "GitHub PR poll error");
            Vec::new()
        }
    }
}

async fn poll_comments(
    client: &Client,
    config: &LoopConfig,
    repo: &str,
    pr: u64,
    since: &DateTime<Utc>,
) -> Vec<GhComment> {
    let since_str = since.to_rfc3339();
    let url = format!(
        "https://api.github.com/repos/{repo}/issues/{pr}/comments?since={since_str}&per_page=50"
    );
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", config.github_token))
        .header("User-Agent", "openab-loop-controller")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn pr_matches_conditions(pr: &GhPullRequest, conditions: &LoopConditions) -> bool {
    // Label filter: PR must have at least one required label
    if !conditions.labels.is_empty() {
        let pr_labels: Vec<&str> = pr.labels.iter().map(|l| l.name.as_str()).collect();
        if !conditions.labels.iter().any(|l| pr_labels.contains(&l.as_str())) {
            return false;
        }
    }
    // Base branch filter
    if !conditions.base_branch.is_empty()
        && !conditions.base_branch.contains(&pr.base.ref_name)
    {
        return false;
    }
    true
}

// ─── Discord Dispatch ─────────────────────────────────────────────────────

async fn send_discord_message(
    client: &Client,
    config: &LoopConfig,
    channel_id: &str,
    thread_id: Option<&str>,
    content: &str,
) -> Option<String> {
    let target = thread_id.unwrap_or(channel_id);
    let url = format!("https://discord.com/api/v10/channels/{target}/messages");
    let body = serde_json::json!({ "content": content });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bot {}", config.discord_token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let json: serde_json::Value = r.json().await.unwrap_or_default();
            json.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())
        }
        Ok(r) => {
            warn!(status = %r.status(), "Discord message send failed");
            None
        }
        Err(e) => {
            warn!(err = %e, "Discord message send error");
            None
        }
    }
}

async fn create_discord_thread(
    client: &Client,
    config: &LoopConfig,
    channel_id: &str,
    name: &str,
) -> Option<String> {
    let url = format!("https://discord.com/api/v10/channels/{channel_id}/threads");
    let body = serde_json::json!({
        "name": name,
        "type": 11, // PUBLIC_THREAD
        "auto_archive_duration": 1440
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bot {}", config.discord_token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            let json: serde_json::Value = r.json().await.unwrap_or_default();
            json.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())
        }
        _ => None,
    }
}

async fn dispatch_reviewer(
    client: &Client,
    config: &LoopConfig,
    instance: &LoopInstance,
) {
    let msg = format!(
        "<@{}> 🔄 **Review requested** (iteration {}/{})\n\
         PR: `{}#{}`\nBranch: `{}`\nSHA: `{}`",
        config.reviewer_agent_id,
        instance.iteration + 1,
        instance.max_iterations,
        instance.repo,
        instance.pr_number,
        instance.branch,
        instance.last_known_sha,
    );
    send_discord_message(
        client,
        config,
        &config.discord_channel_id,
        instance.thread_id.as_deref(),
        &msg,
    )
    .await;
}

async fn dispatch_coder(
    client: &Client,
    config: &LoopConfig,
    instance: &LoopInstance,
    findings: &str,
) {
    let msg = format!(
        "<@{}> 🔧 **Fix requested** (iteration {}/{})\n\
         PR: `{}#{}`\nBranch: `{}`\n\n**Findings:**\n{}",
        config.coder_agent_id,
        instance.iteration + 1,
        instance.max_iterations,
        instance.repo,
        instance.pr_number,
        instance.branch,
        findings,
    );
    send_discord_message(
        client,
        config,
        &config.discord_channel_id,
        instance.thread_id.as_deref(),
        &msg,
    )
    .await;
}

async fn escalate(
    client: &Client,
    config: &LoopConfig,
    instance: &LoopInstance,
    reason: &str,
) {
    let msg = format!(
        "<@{}> ⚠️ **Loop escalation** — `{}#{}`\nReason: {}\nState: {:?} | Iteration: {}",
        config.escalation_user_id,
        instance.repo,
        instance.pr_number,
        reason,
        instance.state,
        instance.iteration,
    );
    send_discord_message(
        client,
        config,
        &config.discord_channel_id,
        instance.thread_id.as_deref(),
        &msg,
    )
    .await;
}

// ─── Core Loop Logic ──────────────────────────────────────────────────────

fn parse_verdict(comment_body: &str) -> Option<Verdict> {
    let body = comment_body.to_uppercase();
    if body.contains("LGTM") && body.contains('✅') {
        Some(Verdict::Lgtm)
    } else if body.contains("CHANGES REQUESTED") && body.contains('⚠') {
        Some(Verdict::ChangesRequested(comment_body.to_string()))
    } else if body.contains("INCOMPLETE") && body.contains('⏸') {
        Some(Verdict::Incomplete)
    } else {
        None
    }
}

#[derive(Debug)]
enum Verdict {
    Lgtm,
    ChangesRequested(String),
    Incomplete,
}

async fn process_loop_tick(
    client: &Client,
    config: &LoopConfig,
    store: &LoopStore,
) {
    let repos = &config.conditions.repos;
    if repos.is_empty() {
        return;
    }

    for repo in repos {
        // 1. Poll open PRs and check for new ones matching conditions
        let prs = poll_prs(client, config, repo).await;
        let mut store_lock = store.lock().await;

        for pr in &prs {
            if pr.state != "open" {
                continue;
            }
            if !pr_matches_conditions(pr, &config.conditions) {
                continue;
            }

            let key = loop_key(repo, pr.number);

            if !store_lock.contains_key(&key) {
                // New PR entering the loop
                info!(repo, pr = pr.number, "new PR entering loop");
                let instance = LoopInstance {
                    repo: repo.clone(),
                    pr_number: pr.number,
                    branch: pr.head.ref_name.clone(),
                    state: LoopState::Reviewing,
                    iteration: 0,
                    max_iterations: config.max_iterations,
                    last_known_sha: pr.head.sha.clone(),
                    step_started_at: Utc::now(),
                    created_at: Utc::now(),
                    retries: 0,
                    thread_id: None,
                };
                store_lock.insert(key.clone(), instance);

                // Create a Discord thread for this loop
                let thread_name = format!("loop: {}#{}", repo, pr.number);
                drop(store_lock);
                let thread_id =
                    create_discord_thread(client, config, &config.discord_channel_id, &thread_name)
                        .await;
                let mut store_lock = store.lock().await;
                if let Some(inst) = store_lock.get_mut(&key) {
                    inst.thread_id = thread_id;
                    dispatch_reviewer(client, config, inst).await;
                }
            } else if let Some(inst) = store_lock.get_mut(&key) {
                // Existing loop — check for new push (SHA changed)
                if inst.state == LoopState::Fixing && pr.head.sha != inst.last_known_sha {
                    info!(repo, pr = pr.number, "new push detected, triggering re-review");
                    inst.last_known_sha = pr.head.sha.clone();
                    inst.iteration += 1;
                    inst.state = LoopState::Reviewing;
                    inst.step_started_at = Utc::now();
                    inst.retries = 0;
                    dispatch_reviewer(client, config, inst).await;
                }
            }
        }

        // 2. For active REVIEWING instances, check for verdict comments
        let keys: Vec<String> = store_lock
            .iter()
            .filter(|(_, inst)| inst.repo == *repo && inst.state == LoopState::Reviewing)
            .map(|(k, _)| k.clone())
            .collect();
        drop(store_lock);

        for key in keys {
            let mut store_lock = store.lock().await;
            let inst = match store_lock.get(&key) {
                Some(i) => i.clone(),
                None => continue,
            };
            drop(store_lock);

            let comments =
                poll_comments(client, config, repo, inst.pr_number, &inst.step_started_at).await;

            for comment in comments.iter().rev() {
                if let Some(verdict) = parse_verdict(&comment.body) {
                    let mut store_lock = store.lock().await;
                    let inst = match store_lock.get_mut(&key) {
                        Some(i) => i,
                        None => break,
                    };
                    match verdict {
                        Verdict::Lgtm => {
                            info!(repo, pr = inst.pr_number, "LGTM — loop complete");
                            inst.state = LoopState::Approved;
                            escalate(
                                client,
                                config,
                                inst,
                                "PR approved ✅ — ready to merge",
                            )
                            .await;
                        }
                        Verdict::ChangesRequested(findings) => {
                            if inst.iteration >= inst.max_iterations {
                                inst.state = LoopState::Escalated;
                                escalate(
                                    client,
                                    config,
                                    inst,
                                    "Max iterations reached",
                                )
                                .await;
                            } else {
                                info!(repo, pr = inst.pr_number, "changes requested, dispatching coder");
                                inst.state = LoopState::Fixing;
                                inst.step_started_at = Utc::now();
                                inst.retries = 0;
                                dispatch_coder(client, config, inst, &findings).await;
                            }
                        }
                        Verdict::Incomplete => {
                            if inst.retries < 1 {
                                info!(repo, pr = inst.pr_number, "incomplete review, retrying");
                                inst.retries += 1;
                                inst.step_started_at = Utc::now();
                                dispatch_reviewer(client, config, inst).await;
                            } else {
                                inst.state = LoopState::Escalated;
                                escalate(
                                    client,
                                    config,
                                    inst,
                                    "Reviewer failed to complete after retry",
                                )
                                .await;
                            }
                        }
                    }
                    break;
                }
            }
        }

        // 3. Check timeouts for all active instances
        let mut store_lock = store.lock().await;
        let timeout_keys: Vec<String> = store_lock
            .iter()
            .filter(|(_, inst)| {
                inst.repo == *repo
                    && matches!(inst.state, LoopState::Reviewing | LoopState::Fixing)
                    && (Utc::now() - inst.step_started_at).num_seconds() as u64
                        > config.step_timeout_secs
            })
            .map(|(k, _)| k.clone())
            .collect();
        drop(store_lock);

        for key in timeout_keys {
            let mut store_lock = store.lock().await;
            let inst = match store_lock.get_mut(&key) {
                Some(i) => i,
                None => continue,
            };

            if inst.retries < 1 {
                warn!(repo, pr = inst.pr_number, state = ?inst.state, "step timeout, retrying");
                inst.retries += 1;
                inst.step_started_at = Utc::now();
                match inst.state {
                    LoopState::Reviewing => dispatch_reviewer(client, config, inst).await,
                    LoopState::Fixing => {
                        dispatch_coder(client, config, inst, "(retry — previous attempt timed out)")
                            .await
                    }
                    _ => {}
                }
            } else {
                warn!(repo, pr = inst.pr_number, "step timeout after retry, escalating");
                inst.state = LoopState::Escalated;
                escalate(
                    client,
                    config,
                    inst,
                    &format!("Step {:?} timed out after retry", inst.state),
                )
                .await;
            }
        }

        // 4. Clean up done/closed PRs
        let open_pr_numbers: Vec<u64> = prs.iter().map(|p| p.number).collect();
        let mut store_lock = store.lock().await;
        store_lock.retain(|_, inst| {
            if inst.repo != *repo {
                return true;
            }
            if matches!(inst.state, LoopState::Done) {
                return false;
            }
            // Remove if PR was closed externally
            open_pr_numbers.contains(&inst.pr_number)
        });
    }
}

// ─── Entry Point ──────────────────────────────────────────────────────────

pub async fn start(config: LoopConfig) {
    info!(
        poll_interval = config.poll_interval_secs,
        max_iterations = config.max_iterations,
        repos = ?config.conditions.repos,
        labels = ?config.conditions.labels,
        "loop controller started"
    );

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("HTTP client must build");

    let store: LoopStore = Arc::new(Mutex::new(HashMap::new()));
    let interval = std::time::Duration::from_secs(config.poll_interval_secs);

    loop {
        process_loop_tick(&client, &config, &store).await;
        tokio::time::sleep(interval).await;
    }
}
