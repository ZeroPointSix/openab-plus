use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Starting,
    Idle,
    Running,
    Suspended,
    Error,
    Exited,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProfileStatus {
    Active,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileConfigError {
    pub config_id: String,
    pub error: String,
}

impl ProfileConfigError {
    pub fn new(config_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            config_id: config_id.into(),
            error: error.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSource {
    pub platform: String,
    pub thread_id: String,
}

impl SessionSource {
    pub fn from_session_key(session_key: &str) -> Self {
        match session_key.split_once(':') {
            Some((platform, thread_id)) if !platform.is_empty() && !thread_id.is_empty() => Self {
                platform: platform.to_string(),
                thread_id: thread_id.to_string(),
            },
            _ => Self {
                platform: "unknown".into(),
                thread_id: session_key.to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub agent: String,
    pub source: SessionSource,
    pub workdir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_status: Option<ProfileStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub status: SessionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile_config_errors: Vec<ProfileConfigError>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_url: Option<String>,
}

impl SessionSnapshot {
    pub fn new(
        session_id: String,
        agent: String,
        workdir: String,
        profile_id: Option<String>,
        profile_name: Option<String>,
        model: Option<String>,
        external_base_url: Option<&str>,
    ) -> Self {
        let now = Utc::now();
        let external_url = session_external_url(external_base_url, &session_id);
        let profile_status = profile_id.as_ref().map(|_| ProfileStatus::Active);
        Self {
            source: SessionSource::from_session_key(&session_id),
            session_id,
            agent,
            workdir,
            profile_id,
            profile_name,
            profile_status,
            model,
            status: SessionStatus::Idle,
            last_error: None,
            profile_config_errors: Vec::new(),
            created_at: now,
            updated_at: now,
            external_url,
        }
    }

    pub fn set_status(&mut self, status: SessionStatus) {
        self.status = status;
        if !matches!(self.status, SessionStatus::Error | SessionStatus::Exited) {
            self.last_error = None;
        }
        self.updated_at = Utc::now();
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.status = SessionStatus::Error;
        self.last_error = Some(error.into());
        self.updated_at = Utc::now();
    }

    pub fn set_exited(&mut self, error: Option<String>) {
        self.status = SessionStatus::Exited;
        if let Some(error) = error {
            self.last_error = Some(error);
        }
        self.updated_at = Utc::now();
    }

    pub fn set_profile_config_errors(&mut self, errors: Vec<ProfileConfigError>) {
        self.profile_config_errors = errors;
        self.updated_at = Utc::now();
    }

    pub fn set_model(&mut self, model: Option<String>) {
        self.model = model;
        self.updated_at = Utc::now();
    }

    pub fn mark_profile_deleted(&mut self, profile_id: &str) -> bool {
        if self.profile_id.as_deref() != Some(profile_id)
            || self.profile_status == Some(ProfileStatus::Deleted)
        {
            return false;
        }

        self.profile_status = Some(ProfileStatus::Deleted);
        self.updated_at = Utc::now();
        true
    }
}

pub fn session_external_url(base_url: Option<&str>, session_id: &str) -> Option<String> {
    let base_url = base_url?.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return None;
    }
    Some(format!(
        "{base_url}/#/sessions/{}",
        encode_path_segment(session_id)
    ))
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_splits_platform_and_thread() {
        let source = SessionSource::from_session_key("slack:1729.42");

        assert_eq!(source.platform, "slack");
        assert_eq!(source.thread_id, "1729.42");
    }

    #[test]
    fn external_url_targets_hash_router_session_route() {
        let url = session_external_url(Some("https://openab.example/"), "slack:1729.42");

        assert_eq!(
            url.as_deref(),
            Some("https://openab.example/#/sessions/slack%3A1729.42")
        );
    }

    #[test]
    fn suspended_status_clears_last_error() {
        let mut snapshot = SessionSnapshot::new(
            "slack:thread".into(),
            "codex".into(),
            "/workspace".into(),
            None,
            None,
            None,
            None,
        );

        snapshot.set_error("process failed");
        snapshot.set_status(SessionStatus::Suspended);

        assert_eq!(snapshot.status, SessionStatus::Suspended);
        assert_eq!(snapshot.last_error, None);
    }

    #[test]
    fn error_status_preserves_last_error() {
        let mut snapshot = SessionSnapshot::new(
            "discord:thread".into(),
            "codex".into(),
            "/workspace".into(),
            Some("default".into()),
            Some("Default".into()),
            Some("gpt-5".into()),
            None,
        );

        snapshot.set_error("process failed");
        assert_eq!(snapshot.status, SessionStatus::Error);
        assert_eq!(snapshot.last_error.as_deref(), Some("process failed"));

        snapshot.set_status(SessionStatus::Running);
        assert_eq!(snapshot.last_error, None);
    }

    #[test]
    fn empty_profile_config_errors_are_not_serialized() {
        let snapshot = SessionSnapshot::new(
            "slack:thread".into(),
            "codex".into(),
            "/workspace".into(),
            None,
            None,
            None,
            None,
        );

        let value = serde_json::to_value(snapshot).expect("snapshot should serialize");

        assert!(value.get("profile_config_errors").is_none());
    }

    #[test]
    fn profile_config_errors_are_serialized_when_present() {
        let mut snapshot = SessionSnapshot::new(
            "slack:thread".into(),
            "codex".into(),
            "/workspace".into(),
            None,
            None,
            None,
            None,
        );
        snapshot.set_profile_config_errors(vec![ProfileConfigError::new("model", "unsupported")]);

        let value = serde_json::to_value(snapshot).expect("snapshot should serialize");

        assert_eq!(value["profile_config_errors"][0]["config_id"], "model");
        assert_eq!(value["profile_config_errors"][0]["error"], "unsupported");
    }

    #[test]
    fn profile_status_starts_active_and_marks_deleted() {
        let mut snapshot = SessionSnapshot::new(
            "slack:thread".into(),
            "codex".into(),
            "/workspace".into(),
            Some("profile-1".into()),
            Some("Default Profile".into()),
            None,
            None,
        );

        assert_eq!(snapshot.profile_status, Some(ProfileStatus::Active));
        assert!(snapshot.mark_profile_deleted("profile-1"));
        assert_eq!(snapshot.profile_id.as_deref(), Some("profile-1"));
        assert_eq!(snapshot.profile_name.as_deref(), Some("Default Profile"));
        assert_eq!(snapshot.profile_status, Some(ProfileStatus::Deleted));
        assert!(!snapshot.mark_profile_deleted("profile-1"));
        assert!(!snapshot.mark_profile_deleted("profile-2"));
    }
}
