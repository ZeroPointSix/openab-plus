pub const THINKING_LEVELS: &[&str] = &[
    "off", "minimal", "low", "medium", "high", "xhigh", "max",
];

const CLAUDE_SUPPORTED: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const CODEX_SUPPORTED: &[&str] = THINKING_LEVELS;

pub fn supported_levels(agent_type: &str, _model: Option<&str>) -> Vec<String> {
    let levels = match agent_type {
        "claude" => CLAUDE_SUPPORTED,
        "codex" => CODEX_SUPPORTED,
        _ => THINKING_LEVELS,
    };
    levels.iter().map(|s| (*s).to_string()).collect()
}

pub fn is_supported(agent_type: &str, model: Option<&str>, level: &str) -> bool {
    supported_levels(agent_type, model)
        .iter()
        .any(|item| item == level)
}

pub fn disabled_levels(agent_type: &str, model: Option<&str>) -> Vec<String> {
    THINKING_LEVELS
        .iter()
        .filter(|level| !is_supported(agent_type, model, level))
        .map(|level| (*level).to_string())
        .collect()
}

/// Map unified level to Codex `model_reasoning_effort` value.
pub fn codex_effort_value(level: &str) -> Option<&'static str> {
    match level {
        "off" => Some("none"),
        "minimal" | "low" | "medium" | "high" | "xhigh" | "max" => {
            THINKING_LEVELS.iter().find(|v| **v == level).copied()
        }
        _ => None,
    }
}

/// Map unified level to Claude `effortLevel` / CLAUDE_CODE_EFFORT_LEVEL.
pub fn claude_effort_value(level: &str) -> Option<&'static str> {
    CLAUDE_SUPPORTED.iter().find(|value| **value == level).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_disables_off_and_minimal() {
        let disabled = disabled_levels("claude", None);
        assert!(disabled.contains(&"off".into()));
        assert!(disabled.contains(&"minimal".into()));
        assert!(!disabled.contains(&"high".into()));
    }
}
