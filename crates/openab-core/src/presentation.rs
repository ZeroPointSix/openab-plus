//! Channel presentation policy — the display half of the channel abstraction.
//!
//! See `docs/adr/channel-presentation-layering.md` (ZER-569). A channel adapter
//! carries two very different kinds of knowledge:
//!
//! 1. **Transport capability** — what the platform is physically able to do
//!    (edit a message, add a reaction, set a native assistant status line).
//! 2. **Presentation policy** — what we *choose* to show there (whether the
//!    agent's intermediate text is exposed, how tool calls are rendered,
//!    whether a streaming placeholder is posted).
//!
//! Capability belongs on the adapter, because only the adapter knows it. Policy
//! is a product decision and belongs in configuration; otherwise every new
//! display preference becomes another [`crate::adapter::ChatAdapter`] method,
//! and eventually pressure to fork the agent Profile schema per channel.
//!
//! [`PresentationPolicy`] is the resolved value a router reads. It is built in
//! three steps of increasing precedence:
//!
//! 1. [`PresentationPolicy::from_display_config`] — the platform-agnostic
//!    `[reactions]` display settings that already exist today.
//! 2. [`PresentationPolicy::with_overrides`] — the channel-scoped
//!    `[<channel>.presentation]` table, i.e. [`PresentationOverrides`].
//! 3. [`PresentationPolicy::clamp_to`] — capability clamping. Policy can never
//!    ask a channel to do something it cannot do, so this step always wins.
//!
//! Nothing here reads or writes an agent Profile. Presentation is a per-channel
//! display concern; the Profile answers "which agent, which model, which
//! reasoning effort" for a session regardless of which channel it arrived on.

use crate::config::{ReactionsConfig, ToolDisplay};
use serde::Deserialize;

/// Where a channel shows the high-level "what is the agent doing right now"
/// signal for a turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusSurface {
    /// Emoji reactions on the triggering message. Today's default for Discord
    /// and for Slack apps without the assistant feature.
    #[default]
    Reactions,
    /// The platform's own status line, e.g. Slack's
    /// `assistant.threads.setStatus` for AI apps.
    AssistantStatus,
    /// No progress signal at all; only the reply itself is posted.
    None,
}

/// What a channel is physically able to do. Reported by the adapter, never by
/// configuration — an operator cannot grant Slack a capability Slack lacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelCapabilities {
    /// The channel may carry the agent's raw intermediate text. `false` marks a
    /// privacy-boundary channel (Slack today): only the finalized answer is
    /// posted, so tool lines and inter-tool narration must be suppressed.
    pub intermediate_text: bool,
    /// The channel supports emoji reactions on a message.
    pub reactions: bool,
    /// The channel has a native assistant status line.
    pub assistant_status: bool,
    /// The channel supports the message editing that live streaming needs.
    pub streaming: bool,
    /// The channel renders Markdown tables natively.
    pub native_tables: bool,
}

impl ChannelCapabilities {
    /// Every optional capability available — the permissive baseline.
    pub const fn all() -> Self {
        Self {
            intermediate_text: true,
            reactions: true,
            assistant_status: true,
            streaming: true,
            native_tables: true,
        }
    }

    /// A plain send-once channel: it still carries the full text, but has no
    /// reactions, no status line, no live editing and no native tables. The
    /// floor a brand-new webhook channel starts from.
    pub const fn send_once() -> Self {
        Self {
            intermediate_text: true,
            reactions: false,
            assistant_status: false,
            streaming: false,
            native_tables: false,
        }
    }
}

/// The `[<channel>.presentation]` config table: per-channel display overrides.
///
/// Every field is optional, and unset means "inherit". A channel section that
/// omits the table therefore keeps exactly the behaviour it has today.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PresentationOverrides {
    /// Expose the agent's intermediate text (inter-tool narration and thinking)
    /// in this channel.
    pub intermediate_text: Option<bool>,
    /// Keep inter-tool narration in send-once replies instead of posting only
    /// the final answer block.
    pub narration: Option<bool>,
    /// Which surface carries the turn's progress signal.
    pub status_surface: Option<StatusSurface>,
    /// Post a separate tool-progress message instead of inline tool lines.
    pub tool_progress_message: Option<bool>,
    /// How tool calls are rendered: `full`, `compact` or `none`.
    pub tool_display: Option<ToolDisplay>,
    /// Send Markdown tables through untouched instead of wrapping them in a
    /// code block.
    pub native_tables: Option<bool>,
    /// Stream the reply live by editing a message.
    pub streaming: Option<bool>,
    /// Post a `…` placeholder when streaming starts.
    pub streaming_placeholder: Option<bool>,
}

/// A fully resolved presentation policy for one channel.
///
/// Build it with [`PresentationPolicy::resolve`] rather than by hand, so the
/// capability clamp is never skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationPolicy {
    /// Whether the agent's raw intermediate text reaches the channel.
    pub intermediate_text: bool,
    /// Whether send-once replies keep inter-tool narration.
    pub narration: bool,
    /// Which surface carries the turn's progress signal.
    pub status_surface: StatusSurface,
    /// Whether a separate tool-progress message is posted.
    pub tool_progress_message: bool,
    /// How tool calls are rendered.
    pub tool_display: ToolDisplay,
    /// Whether Markdown tables are sent through untouched.
    pub native_tables: bool,
    /// Whether the reply is streamed live.
    pub streaming: bool,
    /// Whether a placeholder is posted at streaming start.
    pub streaming_placeholder: bool,
}

impl Default for PresentationPolicy {
    /// The shared defaults in use today: full text, tool lines shown in full,
    /// reaction-based status, send-once narration trimmed, tables wrapped.
    fn default() -> Self {
        Self {
            intermediate_text: true,
            narration: false,
            status_surface: StatusSurface::Reactions,
            tool_progress_message: false,
            tool_display: ToolDisplay::Full,
            native_tables: false,
            streaming: true,
            streaming_placeholder: true,
        }
    }
}

impl PresentationPolicy {
    /// Seed a policy from the platform-agnostic `[reactions]` display settings.
    ///
    /// This is the pre-existing global layer: `narration_display` and
    /// `tool_display` are read straight off it, and `enabled = false` means no
    /// reaction status surface.
    pub fn from_display_config(reactions: &ReactionsConfig) -> Self {
        Self {
            narration: reactions.narration_display,
            tool_display: reactions.tool_display,
            status_surface: if reactions.enabled {
                StatusSurface::Reactions
            } else {
                StatusSurface::None
            },
            ..Self::default()
        }
    }

    /// Apply a channel's `[<channel>.presentation]` overrides. Unset fields are
    /// left untouched.
    #[must_use]
    pub fn with_overrides(mut self, overrides: &PresentationOverrides) -> Self {
        if let Some(v) = overrides.intermediate_text {
            self.intermediate_text = v;
        }
        if let Some(v) = overrides.narration {
            self.narration = v;
        }
        if let Some(v) = overrides.status_surface {
            self.status_surface = v;
        }
        if let Some(v) = overrides.tool_progress_message {
            self.tool_progress_message = v;
        }
        if let Some(v) = overrides.tool_display {
            self.tool_display = v;
        }
        if let Some(v) = overrides.native_tables {
            self.native_tables = v;
        }
        if let Some(v) = overrides.streaming {
            self.streaming = v;
        }
        if let Some(v) = overrides.streaming_placeholder {
            self.streaming_placeholder = v;
        }
        self
    }

    /// Clamp the policy to what the channel can actually do, returning the
    /// names of the fields that had to be changed so the caller can log them.
    ///
    /// Invariants enforced here:
    ///
    /// - A privacy-boundary channel (`intermediate_text = false`) never receives
    ///   intermediate text, inter-tool narration, or per-tool lines.
    /// - Streaming, and therefore the streaming placeholder, requires a channel
    ///   that can edit messages.
    /// - An unavailable status surface degrades to reactions, then to nothing.
    #[must_use]
    pub fn clamp_to(mut self, caps: ChannelCapabilities) -> (Self, Vec<&'static str>) {
        let mut clamped: Vec<&'static str> = Vec::new();

        if !caps.intermediate_text {
            if self.intermediate_text {
                self.intermediate_text = false;
                clamped.push("intermediate_text");
            }
            if self.narration {
                self.narration = false;
                clamped.push("narration");
            }
            if self.tool_display != ToolDisplay::None {
                self.tool_display = ToolDisplay::None;
                clamped.push("tool_display");
            }
        }

        if !caps.streaming && self.streaming {
            self.streaming = false;
            clamped.push("streaming");
        }
        if !self.streaming && self.streaming_placeholder {
            self.streaming_placeholder = false;
            clamped.push("streaming_placeholder");
        }

        let wanted = self.status_surface;
        let allowed = match wanted {
            StatusSurface::AssistantStatus if !caps.assistant_status => {
                if caps.reactions {
                    StatusSurface::Reactions
                } else {
                    StatusSurface::None
                }
            }
            StatusSurface::Reactions if !caps.reactions => StatusSurface::None,
            other => other,
        };
        if allowed != wanted {
            self.status_surface = allowed;
            clamped.push("status_surface");
        }

        if !caps.native_tables && self.native_tables {
            self.native_tables = false;
            clamped.push("native_tables");
        }

        (self, clamped)
    }

    /// Resolve global display config, channel overrides and channel capabilities
    /// into the policy a router should use, plus the list of clamped fields.
    #[must_use]
    pub fn resolve(
        reactions: &ReactionsConfig,
        overrides: &PresentationOverrides,
        caps: ChannelCapabilities,
    ) -> (Self, Vec<&'static str>) {
        Self::from_display_config(reactions)
            .with_overrides(overrides)
            .clamp_to(caps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Capabilities of a Slack AI app: privacy boundary, native status line, no
    /// live streaming of raw agent text.
    fn slack_like() -> ChannelCapabilities {
        ChannelCapabilities {
            intermediate_text: false,
            reactions: true,
            assistant_status: true,
            streaming: false,
            native_tables: false,
        }
    }

    #[test]
    fn default_policy_matches_todays_shared_defaults() {
        let p = PresentationPolicy::default();
        assert!(p.intermediate_text);
        assert!(!p.narration);
        assert_eq!(p.status_surface, StatusSurface::Reactions);
        assert!(!p.tool_progress_message);
        assert_eq!(p.tool_display, ToolDisplay::Full);
        assert!(!p.native_tables);
        assert!(p.streaming);
        assert!(p.streaming_placeholder);
    }

    #[test]
    fn from_display_config_follows_the_reactions_section() {
        let reactions = ReactionsConfig {
            narration_display: true,
            tool_display: ToolDisplay::Compact,
            ..Default::default()
        };
        let p = PresentationPolicy::from_display_config(&reactions);
        assert!(p.narration);
        assert_eq!(p.tool_display, ToolDisplay::Compact);
        assert_eq!(p.status_surface, StatusSurface::Reactions);

        let disabled = ReactionsConfig {
            enabled: false,
            ..Default::default()
        };
        let p = PresentationPolicy::from_display_config(&disabled);
        assert_eq!(p.status_surface, StatusSurface::None);
    }

    #[test]
    fn empty_overrides_change_nothing() {
        let reactions = ReactionsConfig::default();
        let base = PresentationPolicy::from_display_config(&reactions);
        let (resolved, clamped) = PresentationPolicy::resolve(
            &reactions,
            &PresentationOverrides::default(),
            ChannelCapabilities::all(),
        );
        assert_eq!(resolved, base);
        assert!(clamped.is_empty());
    }

    #[test]
    fn overrides_win_over_global_display_config() {
        let reactions = ReactionsConfig::default();
        let overrides = PresentationOverrides {
            narration: Some(true),
            status_surface: Some(StatusSurface::AssistantStatus),
            tool_progress_message: Some(true),
            tool_display: Some(ToolDisplay::None),
            native_tables: Some(true),
            streaming_placeholder: Some(false),
            ..Default::default()
        };
        let (p, clamped) =
            PresentationPolicy::resolve(&reactions, &overrides, ChannelCapabilities::all());
        assert!(p.narration);
        assert_eq!(p.status_surface, StatusSurface::AssistantStatus);
        assert!(p.tool_progress_message);
        assert_eq!(p.tool_display, ToolDisplay::None);
        assert!(p.native_tables);
        assert!(p.streaming);
        assert!(!p.streaming_placeholder);
        assert!(clamped.is_empty());
    }

    #[test]
    fn privacy_boundary_channel_suppresses_intermediate_content() {
        let reactions = ReactionsConfig {
            narration_display: true,
            ..Default::default()
        };
        let overrides = PresentationOverrides {
            intermediate_text: Some(true),
            status_surface: Some(StatusSurface::AssistantStatus),
            ..Default::default()
        };
        let (p, clamped) = PresentationPolicy::resolve(&reactions, &overrides, slack_like());
        assert!(!p.intermediate_text, "config must not open a privacy boundary");
        assert!(!p.narration);
        assert_eq!(p.tool_display, ToolDisplay::None);
        assert_eq!(p.status_surface, StatusSurface::AssistantStatus);
        assert!(!p.streaming);
        assert!(!p.streaming_placeholder);
        assert!(clamped.contains(&"intermediate_text"));
        assert!(clamped.contains(&"narration"));
        assert!(clamped.contains(&"tool_display"));
        assert!(clamped.contains(&"streaming"));
    }

    #[test]
    fn assistant_status_degrades_when_unsupported() {
        let reactions = ReactionsConfig::default();
        let overrides = PresentationOverrides {
            status_surface: Some(StatusSurface::AssistantStatus),
            ..Default::default()
        };

        let (p, clamped) =
            PresentationPolicy::resolve(&reactions, &overrides, ChannelCapabilities::send_once());
        assert_eq!(p.status_surface, StatusSurface::None);
        assert!(clamped.contains(&"status_surface"));
        assert_eq!(
            clamped.iter().filter(|f| **f == "status_surface").count(),
            1,
            "a single degradation must be reported once"
        );

        let with_reactions = ChannelCapabilities {
            reactions: true,
            ..ChannelCapabilities::send_once()
        };
        let (p, _) = PresentationPolicy::resolve(&reactions, &overrides, with_reactions);
        assert_eq!(p.status_surface, StatusSurface::Reactions);
    }

    #[test]
    fn placeholder_requires_streaming() {
        let reactions = ReactionsConfig::default();
        let overrides = PresentationOverrides {
            streaming: Some(false),
            streaming_placeholder: Some(true),
            ..Default::default()
        };
        let (p, clamped) =
            PresentationPolicy::resolve(&reactions, &overrides, ChannelCapabilities::all());
        assert!(!p.streaming_placeholder);
        assert!(clamped.contains(&"streaming_placeholder"));
    }

    #[test]
    fn overrides_parse_from_a_channel_section() {
        #[derive(Debug, Deserialize)]
        struct ChannelSection {
            #[serde(default)]
            presentation: PresentationOverrides,
        }

        let section: ChannelSection = toml::from_str(
            r#"
[presentation]
intermediate_text = false
status_surface = "assistant_status"
tool_display = "compact"
streaming = true
"#,
        )
        .expect("presentation table parses");
        assert_eq!(section.presentation.intermediate_text, Some(false));
        assert_eq!(
            section.presentation.status_surface,
            Some(StatusSurface::AssistantStatus)
        );
        assert_eq!(section.presentation.tool_display, Some(ToolDisplay::Compact));
        assert_eq!(section.presentation.streaming, Some(true));
        assert_eq!(section.presentation.narration, None);

        // A channel section without the table inherits everything.
        let section: ChannelSection = toml::from_str("").expect("absent table is fine");
        assert_eq!(section.presentation, PresentationOverrides::default());
    }

    #[test]
    fn unknown_presentation_key_is_rejected() {
        let err = toml::from_str::<PresentationOverrides>("show_everything = true\n");
        assert!(err.is_err(), "typos must fail loudly, not be ignored");
    }
}
