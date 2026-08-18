//! Channel presentation policy — the display half of the channel abstraction.
//!
//! See `docs/adr/channel-presentation-layering.md` (ZER-569). A channel adapter
//! answers two very different kinds of question:
//!
//! 1. **Transport capability** — what the platform is physically able to do:
//!    edit a message, add a reaction, set a native assistant status line.
//! 2. **Presentation policy** — what we *choose* to show there: whether the
//!    agent's intermediate text is exposed, how tool calls are rendered,
//!    whether a streaming placeholder is posted.
//!
//! Capability belongs on the adapter, because only the adapter knows it. Policy
//! is a product decision that operators must be able to tune per channel;
//! otherwise every new display preference becomes another
//! [`crate::adapter::ChatAdapter`] method, and eventually pressure to fork the
//! agent Profile schema per channel.
//!
//! [`PresentationPolicy`] is the resolved value the router reads once per turn.
//! It is produced by [`PresentationPolicy::resolve`] from three inputs, in
//! increasing order of precedence — with one exception that always wins:
//!
//! 1. [`ChannelCapabilities`], probed from the adapter.
//! 2. The platform-agnostic `[reactions]` display settings
//!    (`narration_display`, `tool_display`) and `[markdown] tables`.
//! 3. The per-channel `[presentation.<platform>]` table
//!    ([`PresentationOverrides`]).
//!
//! **Capability is a ceiling, not a default.** `intermediate_text`, `streaming`,
//! `assistant_status` and `tool_progress_message` describe a privacy boundary or
//! a transport limit, so an override may only ever tighten them. Configuration
//! cannot make Slack publish raw agent text, and cannot make a send-once webhook
//! stream. `native_tables` is the one two-way knob: whether a platform renders
//! Markdown tables is a rendering preference rather than a safety property, so an
//! operator may opt in or out.
//!
//! Nothing here reads or writes an agent Profile. Presentation is a per-channel
//! display concern; the Profile answers "which agent, which model, which
//! reasoning effort" for a session regardless of the channel it arrived on.

use crate::adapter::ChatAdapter;
use crate::config::{ReactionsConfig, ToolDisplay};
use crate::markdown::TableMode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Per-channel presentation overrides, parsed from `[presentation.<platform>]`.
///
/// Every field is optional and unset means "inherit". A config file without a
/// `[presentation]` section therefore behaves exactly as before.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PresentationOverrides {
    /// Expose the agent's intermediate text in this channel. May only tighten:
    /// `true` cannot open a privacy-boundary channel such as Slack.
    pub intermediate_text: Option<bool>,
    /// Keep inter-tool narration in send-once replies instead of posting only
    /// the final answer. Overrides `[reactions] narration_display`.
    pub narration: Option<bool>,
    /// Use the platform's native assistant status line. May only tighten.
    pub assistant_status: Option<bool>,
    /// Keep tool activity in one editable progress message. May only tighten.
    pub tool_progress_message: Option<bool>,
    /// How tool calls are rendered: `full`, `compact` or `none`. Overrides
    /// `[reactions] tool_display`.
    pub tool_display: Option<ToolDisplay>,
    /// Send Markdown tables through untouched instead of converting them to code
    /// blocks. Two-way: an operator may opt in or out.
    pub native_tables: Option<bool>,
    /// Stream the reply by editing a message. May only tighten.
    pub streaming: Option<bool>,
    /// Post the "…" placeholder when streaming starts. May only tighten.
    pub streaming_placeholder: Option<bool>,
}

impl PresentationOverrides {
    /// The "inherit everything" value. Usable as a `&'static` fallback for a
    /// platform with no `[presentation.<platform>]` table.
    pub const INHERIT: Self = Self {
        intermediate_text: None,
        narration: None,
        assistant_status: None,
        tool_progress_message: None,
        tool_display: None,
        native_tables: None,
        streaming: None,
        streaming_placeholder: None,
    };

    /// Overlay every explicitly configured value from `other`.
    pub fn merge_from(&mut self, other: &Self) {
        macro_rules! merge {
            ($field:ident) => {
                if other.$field.is_some() {
                    self.$field = other.$field;
                }
            };
        }
        merge!(intermediate_text);
        merge!(narration);
        merge!(assistant_status);
        merge!(tool_progress_message);
        merge!(tool_display);
        merge!(native_tables);
        merge!(streaming);
        merge!(streaming_placeholder);
    }
}

/// The `[presentation]` config section: platform name -> overrides.
pub type PresentationConfig = HashMap<String, PresentationOverrides>;

/// What a channel is physically able to do, probed from its adapter.
///
/// Operators never set these. They are the ceiling that
/// [`PresentationPolicy::resolve`] clamps configuration against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelCapabilities {
    /// The channel may carry the agent's raw intermediate text. `false` marks a
    /// privacy-boundary channel (Slack today): only the finalized answer is
    /// published, so narration and per-tool detail must be suppressed.
    pub intermediate_text: bool,
    /// The channel supports the message editing that streaming needs.
    pub streaming: bool,
    /// The channel has a first-class streaming API (Slack assistant mode).
    pub native_streaming: bool,
    /// The channel has a native assistant status line.
    pub assistant_status: bool,
    /// The channel keeps tool activity in one editable progress message.
    pub tool_progress_message: bool,
    /// The channel renders Markdown tables natively.
    pub native_tables: bool,
    /// The channel wants a "…" placeholder before streaming starts.
    pub streaming_placeholder: bool,
    /// Label for a session deep link appended to progress and final messages.
    pub session_link_label: Option<&'static str>,
}

impl ChannelCapabilities {
    /// Build the legacy capability view from one adapter.
    ///
    /// Platform-specific exceptions belong in
    /// [`ChatAdapter::presentation_capabilities`], not in policy resolution.
    pub fn from_adapter<A: ChatAdapter + ?Sized>(
        adapter: &A,
        platform: &str,
        other_bot_present: bool,
    ) -> Self {
        Self {
            intermediate_text: adapter.exposes_intermediate_text(),
            streaming: adapter.use_streaming(other_bot_present),
            native_streaming: adapter.uses_native_streaming(other_bot_present),
            assistant_status: adapter.uses_assistant_status(),
            tool_progress_message: adapter.uses_tool_progress_message(),
            native_tables: adapter.renders_native_tables(platform),
            streaming_placeholder: adapter.show_streaming_placeholder(),
            session_link_label: adapter.session_link_label(),
        }
    }
}

/// A fully resolved presentation policy for one turn on one channel.
///
/// Field names match the locals the router used before this type existed, so the
/// send path reads the policy instead of interrogating the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PresentationPolicy {
    /// Whether raw intermediate agent text may be published.
    pub intermediate_text: bool,
    /// Whether the reply is streamed by editing a message.
    pub streaming: bool,
    /// Whether streaming uses the platform's native streaming API.
    pub native_streaming: bool,
    /// Whether the full running text is retained rather than trimmed to the
    /// final answer block.
    pub keep_full_text: bool,
    /// Whether progress is reported on a native status line.
    pub assistant_status: bool,
    /// Table pre-pass mode for this channel.
    pub table_mode: TableMode,
    /// How tool calls are rendered in progress output.
    pub tool_display: ToolDisplay,
    /// How tool calls are rendered in the final reply.
    pub reply_tool_display: ToolDisplay,
    /// Whether tool activity lives in a separate progress message.
    pub tool_progress_message: bool,
    /// Whether a placeholder message is posted at streaming start.
    pub streaming_placeholder: bool,
    /// Session deep-link label, if the channel has a session console.
    pub session_link_label: Option<&'static str>,
}

/// One requested value that a physical channel capability changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyClamp {
    pub requested: String,
    pub effective: String,
    pub clamped_by: String,
}

/// Complete, explainable result returned to routers and management APIs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresentationResolution {
    pub requested: BTreeMap<String, String>,
    pub effective: PresentationPolicy,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub clamped_by: BTreeMap<String, PolicyClamp>,
}

impl PresentationPolicy {
    /// Resolve capabilities, global display config and per-channel overrides
    /// into the policy the router should use for this turn.
    pub fn resolve(
        caps: ChannelCapabilities,
        reactions: &ReactionsConfig,
        table_mode: TableMode,
        overrides: &PresentationOverrides,
    ) -> Self {
        Self::resolve_with_report(caps, reactions, table_mode, overrides).effective
    }

    /// Resolve policy and retain enough information to explain every clamp.
    pub fn resolve_with_report(
        caps: ChannelCapabilities,
        reactions: &ReactionsConfig,
        table_mode: TableMode,
        overrides: &PresentationOverrides,
    ) -> PresentationResolution {
        let mut requested = BTreeMap::new();
        let mut clamped_by = BTreeMap::new();

        let requested_intermediate = overrides.intermediate_text.unwrap_or(true);
        requested.insert(
            "intermediate_text".into(),
            requested_intermediate.to_string(),
        );
        // Privacy boundary: capability is a ceiling, so an override can only
        // ever close this, never open it.
        let intermediate_text = caps.intermediate_text && requested_intermediate;
        record_clamp(
            &mut clamped_by,
            "intermediate_text",
            requested_intermediate,
            intermediate_text,
            "channel.capability.intermediate_text",
        );

        let requested_streaming = overrides.streaming.unwrap_or(true);
        requested.insert("streaming".into(), requested_streaming.to_string());
        let streaming = caps.streaming && requested_streaming && intermediate_text;
        record_clamp(
            &mut clamped_by,
            "streaming",
            requested_streaming,
            streaming,
            if !intermediate_text {
                "effective.intermediate_text"
            } else {
                "channel.capability.streaming"
            },
        );

        // Intermediate narration is never retained across a privacy boundary,
        // even when the shared narration setting is enabled.
        let narration = overrides.narration.unwrap_or(reactions.narration_display);
        requested.insert("narration".into(), narration.to_string());
        let keep_full_text = intermediate_text && (streaming || narration);

        let native_streaming = streaming && caps.native_streaming;
        let requested_assistant_status = overrides.assistant_status.unwrap_or(true);
        requested.insert(
            "assistant_status".into(),
            requested_assistant_status.to_string(),
        );
        let assistant_status = caps.assistant_status && requested_assistant_status;
        record_clamp(
            &mut clamped_by,
            "assistant_status",
            requested_assistant_status,
            assistant_status,
            "channel.capability.assistant_status",
        );

        let requested_native_tables = overrides.native_tables.unwrap_or(caps.native_tables);
        requested.insert("native_tables".into(), requested_native_tables.to_string());
        let native_tables = caps.native_tables && requested_native_tables;
        record_clamp(
            &mut clamped_by,
            "native_tables",
            requested_native_tables,
            native_tables,
            "channel.capability.native_tables",
        );
        let table_mode = if native_tables {
            TableMode::Off
        } else {
            table_mode
        };

        // Tool titles can carry command arguments or other execution detail, so
        // a privacy-boundary channel exposes only generic counts and states.
        let requested_tool_display = overrides.tool_display.unwrap_or(reactions.tool_display);
        requested.insert("tool_display".into(), requested_tool_display.to_string());
        let tool_display = if intermediate_text {
            requested_tool_display
        } else {
            ToolDisplay::None
        };
        if tool_display != requested_tool_display {
            clamped_by.insert(
                "tool_display".into(),
                PolicyClamp {
                    requested: requested_tool_display.to_string(),
                    effective: tool_display.to_string(),
                    clamped_by: "effective.intermediate_text".into(),
                },
            );
        }
        let requested_tool_progress = overrides.tool_progress_message.unwrap_or(true);
        requested.insert(
            "tool_progress_message".into(),
            requested_tool_progress.to_string(),
        );
        let tool_progress_message = caps.tool_progress_message && requested_tool_progress;
        record_clamp(
            &mut clamped_by,
            "tool_progress_message",
            requested_tool_progress,
            tool_progress_message,
            "channel.capability.tool_progress_message",
        );
        // Tool lines already live in the progress message; do not duplicate them
        // in the final reply.
        let reply_tool_display = if tool_progress_message {
            ToolDisplay::None
        } else {
            tool_display
        };

        let requested_placeholder = overrides.streaming_placeholder.unwrap_or(true);
        requested.insert(
            "streaming_placeholder".into(),
            requested_placeholder.to_string(),
        );
        let streaming_placeholder =
            streaming && caps.streaming_placeholder && requested_placeholder;
        record_clamp(
            &mut clamped_by,
            "streaming_placeholder",
            requested_placeholder,
            streaming_placeholder,
            if !streaming {
                "effective.streaming"
            } else {
                "channel.capability.streaming_placeholder"
            },
        );

        let effective = Self {
            intermediate_text,
            streaming,
            native_streaming,
            keep_full_text,
            assistant_status,
            table_mode,
            tool_display,
            reply_tool_display,
            tool_progress_message,
            streaming_placeholder,
            session_link_label: caps.session_link_label,
        };

        PresentationResolution {
            requested,
            effective,
            clamped_by,
        }
    }
}

fn record_clamp(
    clamps: &mut BTreeMap<String, PolicyClamp>,
    field: &str,
    requested: bool,
    effective: bool,
    reason: &str,
) {
    if requested != effective {
        clamps.insert(
            field.into(),
            PolicyClamp {
                requested: requested.to_string(),
                effective: effective.to_string(),
                clamped_by: reason.into(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Discord: full text, reactions for status, no native streaming or tables.
    fn discord_like() -> ChannelCapabilities {
        ChannelCapabilities {
            intermediate_text: true,
            streaming: true,
            native_streaming: false,
            assistant_status: false,
            tool_progress_message: false,
            native_tables: false,
            streaming_placeholder: true,
            session_link_label: None,
        }
    }

    /// Slack AI app: privacy boundary, native status line, native streaming,
    /// tool progress message, native tables.
    fn slack_like() -> ChannelCapabilities {
        ChannelCapabilities {
            intermediate_text: false,
            streaming: true,
            native_streaming: true,
            assistant_status: true,
            tool_progress_message: true,
            native_tables: true,
            streaming_placeholder: true,
            session_link_label: Some("Open session"),
        }
    }

    /// A plain send-once webhook channel.
    fn send_once_like() -> ChannelCapabilities {
        ChannelCapabilities {
            intermediate_text: true,
            streaming: false,
            native_streaming: false,
            assistant_status: false,
            tool_progress_message: false,
            native_tables: false,
            streaming_placeholder: false,
            session_link_label: None,
        }
    }

    fn resolve(caps: ChannelCapabilities, reactions: &ReactionsConfig) -> PresentationPolicy {
        PresentationPolicy::resolve(
            caps,
            reactions,
            TableMode::default(),
            &PresentationOverrides::INHERIT,
        )
    }

    fn is_default_table_mode(mode: TableMode) -> bool {
        format!("{mode:?}") == format!("{:?}", TableMode::default())
    }

    #[test]
    fn inherit_is_the_default() {
        assert_eq!(
            PresentationOverrides::INHERIT,
            PresentationOverrides::default()
        );
    }

    #[test]
    fn discord_keeps_todays_behaviour() {
        let reactions = ReactionsConfig::default();
        let p = resolve(discord_like(), &reactions);
        assert!(p.intermediate_text);
        assert!(p.streaming);
        assert!(!p.native_streaming);
        assert!(
            p.keep_full_text,
            "streaming implies the running text is kept"
        );
        assert!(!p.assistant_status);
        assert!(
            is_default_table_mode(p.table_mode),
            "tables still converted"
        );
        assert_eq!(p.tool_display, reactions.tool_display);
        assert_eq!(p.reply_tool_display, reactions.tool_display);
        assert!(!p.tool_progress_message);
        assert!(p.streaming_placeholder);
        assert_eq!(p.session_link_label, None);
    }

    #[test]
    fn slack_privacy_boundary_suppresses_intermediate_content() {
        let reactions = ReactionsConfig {
            narration_display: true,
            tool_display: ToolDisplay::Full,
            ..Default::default()
        };
        let p = resolve(slack_like(), &reactions);
        assert!(!p.intermediate_text);
        assert!(!p.streaming, "no raw chunks across a privacy boundary");
        assert!(!p.native_streaming);
        assert!(
            !p.keep_full_text,
            "narration_display cannot cross the boundary"
        );
        assert!(p.assistant_status, "status line still reports progress");
        assert_eq!(p.tool_display, ToolDisplay::None);
        assert_eq!(p.reply_tool_display, ToolDisplay::None);
        assert!(p.tool_progress_message);
        assert!(!p.streaming_placeholder, "no placeholder without streaming");
        assert!(
            matches!(p.table_mode, TableMode::Off),
            "Slack renders tables natively"
        );
        assert_eq!(p.session_link_label, Some("Open session"));
    }

    #[test]
    fn append_only_channel_never_streams() {
        let reactions = ReactionsConfig::default();
        let mut append_only = discord_like();
        append_only.streaming = false;
        append_only.native_streaming = false;
        append_only.streaming_placeholder = false;
        let p = resolve(append_only, &reactions);
        assert!(!p.streaming);
        assert!(!p.native_streaming);
        assert!(!p.streaming_placeholder);
        assert!(!p.keep_full_text, "send-once trims unless narration is on");

        let chatty = ReactionsConfig {
            narration_display: true,
            ..Default::default()
        };
        let p = resolve(append_only, &chatty);
        assert!(p.keep_full_text, "narration_display keeps the running text");
    }

    #[test]
    fn send_once_channel_follows_narration_setting() {
        let quiet = ReactionsConfig::default();
        assert!(!resolve(send_once_like(), &quiet).keep_full_text);

        let chatty = ReactionsConfig {
            narration_display: true,
            ..Default::default()
        };
        let p = resolve(send_once_like(), &chatty);
        assert!(p.keep_full_text);
        assert!(!p.streaming_placeholder);
    }

    #[test]
    fn overrides_cannot_open_a_privacy_boundary() {
        let reactions = ReactionsConfig::default();
        let overrides = PresentationOverrides {
            intermediate_text: Some(true),
            streaming: Some(true),
            tool_display: Some(ToolDisplay::Full),
            narration: Some(true),
            ..Default::default()
        };
        let p =
            PresentationPolicy::resolve(slack_like(), &reactions, TableMode::default(), &overrides);
        assert!(!p.intermediate_text);
        assert!(!p.streaming);
        assert!(!p.keep_full_text);
        assert_eq!(p.tool_display, ToolDisplay::None);
    }

    #[test]
    fn overrides_can_tighten_and_tune() {
        let reactions = ReactionsConfig::default();
        let overrides = PresentationOverrides {
            streaming: Some(false),
            tool_display: Some(ToolDisplay::Compact),
            native_tables: Some(true),
            ..Default::default()
        };
        let p = PresentationPolicy::resolve(
            discord_like(),
            &reactions,
            TableMode::default(),
            &overrides,
        );
        assert!(!p.streaming, "an operator may turn streaming off");
        assert!(!p.streaming_placeholder);
        assert_eq!(p.tool_display, ToolDisplay::Compact);
        assert!(
            is_default_table_mode(p.table_mode),
            "config cannot claim native table support the adapter lacks"
        );
    }

    #[test]
    fn assistant_status_can_be_turned_off_per_channel() {
        let reactions = ReactionsConfig::default();
        let overrides = PresentationOverrides {
            assistant_status: Some(false),
            ..Default::default()
        };
        let p =
            PresentationPolicy::resolve(slack_like(), &reactions, TableMode::default(), &overrides);
        assert!(!p.assistant_status);
    }

    #[test]
    fn resolution_reports_requested_effective_and_clamp_reason() {
        let reactions = ReactionsConfig::default();
        let overrides = PresentationOverrides {
            intermediate_text: Some(true),
            streaming: Some(true),
            native_tables: Some(true),
            ..Default::default()
        };

        let resolution = PresentationPolicy::resolve_with_report(
            send_once_like(),
            &reactions,
            TableMode::default(),
            &overrides,
        );

        assert_eq!(resolution.requested["streaming"], "true");
        assert!(!resolution.effective.streaming);
        assert_eq!(
            resolution.clamped_by["streaming"].clamped_by,
            "channel.capability.streaming"
        );
        assert_eq!(
            resolution.clamped_by["native_tables"].clamped_by,
            "channel.capability.native_tables"
        );
    }

    #[test]
    fn presentation_section_parses_from_toml() {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            #[serde(default)]
            presentation: PresentationConfig,
        }

        let parsed: Wrapper = toml::from_str(
            "[presentation.slack]\nassistant_status = false\ntool_display = \"compact\"\n\n[presentation.telegram]\nnarration = true\nstreaming_placeholder = false\n",
        )
        .expect("presentation section parses");
        let slack = parsed.presentation.get("slack").expect("slack overrides");
        assert_eq!(slack.assistant_status, Some(false));
        assert_eq!(slack.tool_display, Some(ToolDisplay::Compact));
        assert_eq!(slack.intermediate_text, None);
        let telegram = parsed
            .presentation
            .get("telegram")
            .expect("telegram overrides");
        assert_eq!(telegram.narration, Some(true));
        assert_eq!(telegram.streaming_placeholder, Some(false));

        let empty: Wrapper = toml::from_str("").expect("absent section is fine");
        assert!(empty.presentation.is_empty());
    }

    #[test]
    fn unknown_presentation_key_is_rejected() {
        assert!(
            toml::from_str::<PresentationOverrides>("show_everything = true\n").is_err(),
            "typos must fail loudly rather than be silently ignored"
        );
    }
}
