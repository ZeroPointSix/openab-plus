use anyhow::{anyhow, Result};
use serde_json::{Map, Value as JsonValue};
use std::collections::BTreeMap;
use toml_edit::{DocumentMut, Item, Value as TomlEditValue};

const REDACTED_ENV_VALUE: &str = "<redacted>";

pub fn redact_sensitive_field_changes(changes: &mut BTreeMap<String, FieldChange>) {
    for (key, change) in changes.iter_mut() {
        if key == "env" {
            if let Some(from) = change.from.as_ref() {
                change.from = Some(redact_json_env_display(from));
            }
            if let Some(to) = change.to.as_ref() {
                change.to = Some(redact_json_env_display(to));
            }
        }
    }
}

fn redact_json_env_display(value: &str) -> String {
    match serde_json::from_str::<JsonValue>(value) {
        Ok(JsonValue::Object(map)) => {
            let redacted = map
                .keys()
                .map(|key| (key.clone(), JsonValue::String(REDACTED_ENV_VALUE.into())))
                .collect::<Map<_, _>>();
            serde_json::to_string(&JsonValue::Object(redacted))
                .unwrap_or_else(|_| REDACTED_ENV_VALUE.into())
        }
        _ => REDACTED_ENV_VALUE.into(),
    }
}

pub fn merge_toml_owned_keys(
    existing: &str,
    owned: &BTreeMap<String, toml::Value>,
) -> Result<(String, BTreeMap<String, FieldChange>)> {
    let mut doc = if existing.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing
            .parse::<DocumentMut>()
            .map_err(|error| anyhow!("invalid toml: {error}"))?
    };
    let mut changes = BTreeMap::new();
    for (key, value) in owned {
        let from = doc
            .get(key)
            .and_then(Item::as_value)
            .map(|v| v.to_string().trim_matches('"').to_string());
        let to = toml_value_to_string(value);
        if from.as_deref() != Some(to.as_str()) {
            changes.insert(
                key.clone(),
                FieldChange {
                    from,
                    to: Some(to.clone()),
                },
            );
        }
        doc[key] = Item::Value(toml_to_edit(value)?);
    }
    Ok((doc.to_string(), changes))
}

pub fn merge_json_owned_keys(
    existing: &str,
    owned: &BTreeMap<String, JsonValue>,
) -> Result<(String, BTreeMap<String, FieldChange>)> {
    let mut root = if existing.trim().is_empty() {
        JsonValue::Object(Map::new())
    } else {
        serde_json::from_str(existing)?
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("claude settings root must be a JSON object"))?;
    let mut changes = BTreeMap::new();
    for (key, value) in owned {
        let from = object.get(key).cloned();
        let merged_value = match (from.as_ref(), value) {
            (Some(JsonValue::Object(existing)), JsonValue::Object(incoming)) => {
                let mut nested = existing.clone();
                for (nested_key, nested_value) in incoming {
                    nested.insert(nested_key.clone(), nested_value.clone());
                }
                JsonValue::Object(nested)
            }
            _ => value.clone(),
        };
        if from.as_ref() != Some(&merged_value) {
            changes.insert(
                key.clone(),
                FieldChange {
                    from: from.map(|v| v.to_string()),
                    to: Some(merged_value.to_string()),
                },
            );
        }
        object.insert(key.clone(), merged_value);
    }
    Ok((serde_json::to_string_pretty(&root)?, changes))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldChange {
    pub from: Option<String>,
    pub to: Option<String>,
}

fn toml_value_to_string(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn toml_to_edit(value: &toml::Value) -> Result<TomlEditValue> {
    match value {
        toml::Value::String(s) => Ok(TomlEditValue::from(s.as_str())),
        toml::Value::Integer(v) => Ok(TomlEditValue::from(*v)),
        toml::Value::Float(v) => Ok(TomlEditValue::from(*v)),
        toml::Value::Boolean(v) => Ok(TomlEditValue::from(*v)),
        other => Err(anyhow!("unsupported toml owned value type: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_sensitive_field_changes_masks_env_values() {
        let mut changes = BTreeMap::from([(
            "env".into(),
            FieldChange {
                from: Some(r#"{"KEEP":"secret","ANTHROPIC_API_KEY":"old"}"#.into()),
                to: Some(r#"{"ANTHROPIC_BASE_URL":"https://api.example.com"}"#.into()),
            },
        )]);
        redact_sensitive_field_changes(&mut changes);
        let env = changes.get("env").expect("env change");
        assert!(env.from.as_deref().unwrap().contains("<redacted>"));
        assert!(!env.from.as_deref().unwrap().contains("secret"));
        assert!(env.to.as_deref().unwrap().contains("ANTHROPIC_BASE_URL"));
    }

    #[test]
    fn toml_merge_preserves_unknown_keys() {
        let existing = "model = \"old\"\napproval_policy = \"never\"\n";
        let mut owned = BTreeMap::new();
        owned.insert("model".into(), toml::Value::String("gpt-5".into()));
        owned.insert(
            "model_reasoning_effort".into(),
            toml::Value::String("high".into()),
        );
        let (merged, changes) = merge_toml_owned_keys(existing, &owned).unwrap();
        assert!(merged.contains("approval_policy"));
        assert!(merged.contains("gpt-5"));
        assert!(changes.contains_key("model"));
    }

    #[test]
    fn json_merge_preserves_unknown_keys() {
        let existing = r#"{"permissions":{"allow":["Bash"]},"model":"old","env":{"KEEP":"1"}}"#;
        let mut owned = BTreeMap::new();
        owned.insert("model".into(), JsonValue::String("claude-sonnet".into()));
        owned.insert("effortLevel".into(), JsonValue::String("high".into()));
        let mut env = serde_json::Map::new();
        env.insert(
            "ANTHROPIC_BASE_URL".into(),
            JsonValue::String("https://api.example.com".into()),
        );
        owned.insert("env".into(), JsonValue::Object(env));
        let (merged, _) = merge_json_owned_keys(existing, &owned).unwrap();
        assert!(merged.contains("permissions"));
        assert!(merged.contains("claude-sonnet"));
        assert!(merged.contains("KEEP"));
        assert!(merged.contains("ANTHROPIC_BASE_URL"));
    }
}
