//! Validated, shell-free rules shared by rule authoring and the evaluator.
use crate::state::{ConditionOperator as Op, Rule, RuleAction as Action, RuleCondition, State};
use anyhow::{Result, bail, ensure};
use serde_json::Value;
use std::collections::HashSet;

/// Resolve only a real scalar exported field; arrays and arbitrary plugin payloads are excluded.
pub fn value<'a>(state: &'a Value, field: &str) -> Option<&'a Value> {
    if field.starts_with("plugin_data") {
        return None;
    }
    let mut current = state;
    for part in field.split('.') {
        current = current.as_object()?.get(part)?;
    }
    if current.is_string() || current.is_boolean() || current.is_number() {
        Some(current)
    } else {
        None
    }
}
/// Validate a structured condition against the exported schema and operator types.
pub fn validate_condition(condition: &RuleCondition) -> Result<()> {
    let schema = serde_json::to_value(State::default())?;
    let field = value(&schema, &condition.field)
        .ok_or_else(|| anyhow::anyhow!("Unknown scalar field: {}", condition.field))?;
    ensure!(
        (field.is_number() && condition.value.is_number())
            || (field.is_string() && condition.value.is_string())
            || (field.is_boolean() && condition.value.is_boolean()),
        "Condition value has wrong type"
    );
    match condition.operator {
        Op::GreaterThan | Op::LessThan => {
            ensure!(field.is_number(), "Comparison requires a numeric field")
        }
        Op::Contains | Op::StartsWith | Op::EndsWith => {
            ensure!(field.is_string(), "String operator requires a string field")
        }
        _ => {}
    }
    Ok(())
}
/// Missing or malformed values never satisfy a condition, including not-equals.
pub fn evaluate(condition: &RuleCondition, state: &Value) -> bool {
    let Some(actual) = value(state, &condition.field) else {
        return false;
    };
    let expected = &condition.value;
    match condition.operator {
        Op::Equals => actual == expected,
        Op::NotEquals => {
            actual != expected
                && ((actual.is_number() && expected.is_number())
                    || (actual.is_string() && expected.is_string())
                    || (actual.is_boolean() && expected.is_boolean()))
        }
        Op::GreaterThan => actual
            .as_f64()
            .zip(expected.as_f64())
            .is_some_and(|(a, b)| a > b),
        Op::LessThan => actual
            .as_f64()
            .zip(expected.as_f64())
            .is_some_and(|(a, b)| a < b),
        Op::Contains => actual
            .as_str()
            .zip(expected.as_str())
            .is_some_and(|(a, b)| a.contains(b)),
        Op::StartsWith => actual
            .as_str()
            .zip(expected.as_str())
            .is_some_and(|(a, b)| a.starts_with(b)),
        Op::EndsWith => actual
            .as_str()
            .zip(expected.as_str())
            .is_some_and(|(a, b)| a.ends_with(b)),
    }
}
/// Parse the small legacy command subset into typed actions; no shell grammar is accepted.
pub fn parse_command(command: &str) -> Result<Action> {
    ensure!(
        command.len() <= 512
            && command
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b" ._-".contains(&c)),
        "Command contains unsupported syntax; use a structured action"
    );
    let parts: Vec<_> = command.split_whitespace().collect();
    let action = match parts.as_slice() {
        ["gtk-launch", id] => Action::LaunchApp {
            app_id: (*id).into(),
        },
        ["hyprctl", "dispatch", "workspace", id] => Action::SwitchWorkspace {
            workspace_id: id.parse()?,
        },
        ["powerprofilesctl", "set", profile] | ["samosctl", "power-profile", profile] => {
            Action::SetPowerProfile {
                profile: (*profile).into(),
            }
        }
        ["samosctl", "theme-set", theme] => Action::SetTheme {
            theme: (*theme).into(),
        },
        ["samosctl", "wifi-toggle"] => Action::ToggleWifi,
        ["samosctl", "bluetooth-toggle"] => Action::ToggleBluetooth,
        _ => bail!("Unsupported command; use a structured action"),
    };
    validate_action(&action)?;
    Ok(action)
}
/// Validate every executable argument before a ruleset can become active.
pub fn validate_action(action: &Action) -> Result<()> {
    match action {
        Action::RunCommand { command } => {
            parse_command(command)?;
        }
        Action::SetTheme { theme } => crate::config::validate_name(theme)?,
        Action::SetPowerProfile { profile } => ensure!(
            ["balanced", "power-saver", "performance"].contains(&profile.as_str()),
            "Invalid power profile"
        ),
        Action::SwitchWorkspace { workspace_id } => {
            ensure!((1..=1000).contains(workspace_id), "Invalid workspace")
        }
        Action::LaunchApp { app_id } => ensure!(
            !app_id.is_empty()
                && app_id.len() <= 200
                && !app_id.starts_with('-')
                && app_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "Invalid desktop application ID"
        ),
        Action::Speak { text } | Action::Log { message: text } => ensure!(
            !text.trim().is_empty() && text.len() <= 2000,
            "Action text must contain 1–2000 bytes"
        ),
        _ => {}
    }
    Ok(())
}
/// Validate a complete bounded ruleset without partially applying an invalid edit.
pub fn validate(rules: &[Rule]) -> Result<()> {
    ensure!(rules.len() <= 100, "At most 100 rules are supported");
    let mut ids = HashSet::new();
    for rule in rules {
        ensure!(
            !rule.id.is_empty() && rule.id.len() <= 128 && ids.insert(&rule.id),
            "Missing or duplicate rule ID"
        );
        ensure!(
            !rule.name.trim().is_empty() && rule.name.len() <= 200,
            "Invalid rule name"
        );
        ensure!(
            !rule.conditions.is_empty()
                && rule.conditions.len() <= 8
                && !rule.actions.is_empty()
                && rule.actions.len() <= 8,
            "Rules need 1–8 conditions and actions"
        );
        for c in &rule.conditions {
            validate_condition(c)?;
        }
        for a in &rule.actions {
            validate_action(a)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_shell_and_wrong_types() {
        for text in [
            "gtk-launch firefox; touch /tmp/x",
            "gtk-launch $(id)",
            "gtk-launch firefox\nreboot",
            "gtk-launch --help",
            "samosctl theme-set ../x",
        ] {
            assert!(parse_command(text).is_err());
        }
        assert!(parse_command("hyprctl dispatch workspace 2").is_ok());
        assert!(
            validate_condition(&RuleCondition {
                field: "cpu.typo".into(),
                operator: Op::NotEquals,
                value: 0.into()
            })
            .is_err()
        );
        assert!(
            validate_condition(&RuleCondition {
                field: "cpu.usage".into(),
                operator: Op::Contains,
                value: "90".into()
            })
            .is_err()
        );
    }
}
