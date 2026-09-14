use crate::state::{ConditionOperator, Rule, RuleAction, RuleCondition, State};
use anyhow::{anyhow, Result};
use serde_json::{json, Number, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub struct AutomationModule {
    rules: Arc<Mutex<Vec<Rule>>>,
    last_states: Arc<Mutex<HashMap<String, serde_json::Value>>>,
}

impl AutomationModule {
    pub fn new() -> Result<Self> {
        Ok(Self {
            rules: Arc::new(Mutex::new(Vec::new())),
            last_states: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn load_rules(&self) -> Result<()> {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = format!("{}/.config/samos/rules.json", home);

        if std::path::Path::new(&path).exists() {
            let content = std::fs::read_to_string(&path)?;
            let rules: Vec<Rule> = serde_json::from_str(&content)?;
            let validated_rules = Self::validate_rules(rules)?;
            *self.rules.lock().unwrap() = validated_rules;
        }
        Ok(())
    }

    fn validate_rules(rules: Vec<Rule>) -> Result<Vec<Rule>> {
        let mut validated = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        for mut rule in rules {
            if rule.id.is_empty() {
                rule.id = uuid::Uuid::new_v4().to_string();
            }
            if seen_ids.contains(&rule.id) {
                return Err(anyhow!("Duplicate rule ID: {}", rule.id));
            }
            seen_ids.insert(rule.id.clone());

            if rule.name.is_empty() {
                return Err(anyhow!("Rule '{}' has empty name", rule.id));
            }

            if rule.conditions.is_empty() {
                return Err(anyhow!("Rule '{}' has no conditions", rule.id));
            }

            for condition in &rule.conditions {
                if condition.field.is_empty() {
                    return Err(anyhow!("Rule '{}' has condition with empty field", rule.id));
                }
                if !Self::is_valid_field_path(&condition.field) {
                    return Err(anyhow!("Rule '{}' has invalid field path: {}", rule.id, condition.field));
                }
            }

            if rule.actions.is_empty() {
                return Err(anyhow!("Rule '{}' has no actions", rule.id));
            }

            for action in &rule.actions {
                if let RuleAction::RunCommand { command } = action {
                    if !Self::is_command_allowed(command) {
                        return Err(anyhow!("Rule '{}' uses disallowed command: {}", rule.id, command));
                    }
                }
            }

            validated.push(rule);
        }

        Ok(validated)
    }

    fn is_valid_field_path(path: &str) -> bool {
        let valid_prefixes = [
            "system.", "cpu.", "memory.", "battery.", "disk.", "network.",
            "temperature.", "control.", "workspace.", "ai.", "automation.", "theme"
        ];
        valid_prefixes.iter().any(|p| path.starts_with(p))
    }

    fn is_command_allowed(command: &str) -> bool {
        let allowed_prefixes = [
            "samosctl ",
            "hyprctl dispatch workspace ",
            "powerprofilesctl set ",
            "gtk-launch ",
            "notify-send ",
        ];
        let dangerous_patterns = [
            "rm ", "sudo ", "dd ", "mkfs ", "shutdown", "reboot", "kill ",
            "chmod 777", "chown root", "> /dev/", "| sh", "| bash", "; rm",
            "&& rm", "`", "$(", "||", "&&",
        ];

        for pattern in &dangerous_patterns {
            if command.contains(pattern) {
                return false;
            }
        }

        allowed_prefixes.iter().any(|p| command.starts_with(p))
    }

    fn save_rules(&self) -> Result<()> {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = format!("{}/.config/samos/rules.json", home);

        std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap())?;
        let content = serde_json::to_string_pretty(&*self.rules.lock().unwrap())?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn add_rule(&self, mut rule: Rule) -> Result<()> {
        if rule.id.is_empty() {
            rule.id = uuid::Uuid::new_v4().to_string();
        }
        let validated = Self::validate_rules(vec![rule])?;
        self.rules.lock().unwrap().push(validated.into_iter().next().unwrap());
        self.save_rules()
    }

    pub fn remove_rule(&self, rule_id: &str) -> Result<()> {
        self.rules.lock().unwrap().retain(|r| r.id != rule_id);
        self.save_rules()
    }

    pub fn get_rules(&self) -> Vec<Rule> {
        self.rules.lock().unwrap().clone()
    }

    fn evaluate_condition(&self, condition: &RuleCondition, state: &State) -> bool {
        let value = self.get_state_value(&condition.field, state);
        match &condition.operator {
            ConditionOperator::Equals => value == condition.value,
            ConditionOperator::NotEquals => value != condition.value,
            ConditionOperator::GreaterThan => {
                if let (Value::Number(a), Value::Number(b)) = (value, &condition.value) {
                    a.as_f64().unwrap_or(0.0) > b.as_f64().unwrap_or(0.0)
                } else {
                    false
                }
            }
            ConditionOperator::LessThan => {
                if let (Value::Number(a), Value::Number(b)) = (value, &condition.value) {
                    a.as_f64().unwrap_or(0.0) < b.as_f64().unwrap_or(0.0)
                } else {
                    false
                }
            }
            ConditionOperator::Contains => {
                if let (Value::String(a), Value::String(b)) = (value, &condition.value) {
                    a.contains(b)
                } else {
                    false
                }
            }
            ConditionOperator::StartsWith => {
                if let (Value::String(a), Value::String(b)) = (value, &condition.value) {
                    a.starts_with(b)
                } else {
                    false
                }
            }
            ConditionOperator::EndsWith => {
                if let (Value::String(a), Value::String(b)) = (value, &condition.value) {
                    a.ends_with(b)
                } else {
                    false
                }
            }
        }
    }

    fn get_state_value(&self, path: &str, state: &State) -> serde_json::Value {
        let parts: Vec<&str> = path.split('.').collect();
        match parts.as_slice() {
            ["system", field] => match *field {
                "hostname" => Value::String(state.system.hostname.clone()),
                "kernel" => Value::String(state.system.kernel.clone()),
                "uptime" => Value::Number(state.system.uptime.into()),
                "time" => Value::String(state.system.time.clone()),
                _ => Value::Null,
            },
            ["cpu", field] => match *field {
                "usage" => {
                    let num = Number::from_f64(state.cpu.usage as f64).unwrap();
                    Value::Number(num)
                }
                _ => Value::Null,
            },
            ["memory", field] => match *field {
                "used_mb" => Value::Number(state.memory.used_mb.into()),
                "total_mb" => Value::Number(state.memory.total_mb.into()),
                "used_percent" => {
                    let num = Number::from_f64(state.memory.used_percent as f64).unwrap();
                    Value::Number(num)
                }
                _ => Value::Null,
            },
            ["battery", field] => match *field {
                "percent" => {
                    let num = Number::from_f64(state.battery.percent as f64).unwrap();
                    Value::Number(num)
                }
                "status" => Value::String(state.battery.status.clone()),
                _ => Value::Null,
            },
            ["disk", field] => match *field {
                "used_gb" => Value::Number(state.disk.used_gb.into()),
                "total_gb" => Value::Number(state.disk.total_gb.into()),
                "used_percent" => {
                    let num = Number::from_f64(state.disk.used_percent as f64).unwrap();
                    Value::Number(num)
                }
                _ => Value::Null,
            },
            ["network", field] => match *field {
                "rx_kb" => Value::Number(state.network.rx_kb.into()),
                "tx_kb" => Value::Number(state.network.tx_kb.into()),
                _ => Value::Null,
            },
            ["temperature", field] => match *field {
                "celsius" => {
                    let num = Number::from_f64(state.temperature.celsius as f64).unwrap();
                    Value::Number(num)
                }
                _ => Value::Null,
            },
            ["control", field] => match *field {
                "wifi_enabled" => Value::Bool(state.control.wifi_enabled),
                "wifi_ssid" => Value::String(state.control.wifi_ssid.clone()),
                "bluetooth_enabled" => Value::Bool(state.control.bluetooth_enabled),
                "bluetooth_connected" => Value::String(state.control.bluetooth_connected.clone()),
                "night_light_enabled" => Value::Bool(state.control.night_light_enabled),
                "night_light_temp" => Value::Number(state.control.night_light_temp.into()),
                "power_profile" => Value::String(state.control.power_profile.clone()),
                _ => Value::Null,
            },
            ["workspace", field] => match *field {
                "active_id" => Value::Number(state.workspace.active_id.into()),
                _ => Value::Null,
            },
            ["ai", field] => match *field {
                "model" => Value::String(state.ai.model.clone()),
                "status" => Value::String(state.ai.status.clone()),
                _ => Value::Null,
            },
            ["automation", field] => match *field {
                "enabled" => Value::Bool(state.automation.enabled),
                "rules_count" => Value::Number(state.automation.rules_count.into()),
                _ => Value::Null,
            },
            ["theme"] => Value::String(state.theme.clone()),
            _ => Value::Null,
        }
    }

    fn execute_action(&self, action: &RuleAction) -> Result<()> {
        match action {
            RuleAction::Notify => {
                let _ = std::process::Command::new("notify-send")
                    .args(["Automation", "Rule triggered"])
                    .output();
            }
            RuleAction::RunCommand { command } => {
                if !Self::is_command_allowed(command) {
                    return Err(anyhow!("Command not allowed by security policy: {}", command));
                }
                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .output()?;
                if !output.status.success() {
                    eprintln!("Command failed: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            RuleAction::SetTheme { theme } => {
                let output = std::process::Command::new("samosctl")
                    .args(["theme-set", theme])
                    .output()?;
                if !output.status.success() {
                    eprintln!("Failed to set theme: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            RuleAction::SetPowerProfile { profile } => {
                let output = std::process::Command::new("powerprofilesctl")
                    .args(["set", profile])
                    .output()?;
                if !output.status.success() {
                    eprintln!("Failed to set power profile: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            RuleAction::ToggleWifi => {
                let output = std::process::Command::new("samosctl")
                    .args(["wifi-toggle"])
                    .output()?;
                if !output.status.success() {
                    eprintln!("Failed to toggle wifi: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            RuleAction::ToggleBluetooth => {
                let output = std::process::Command::new("samosctl")
                    .args(["bluetooth-toggle"])
                    .output()?;
                if !output.status.success() {
                    eprintln!("Failed to toggle bluetooth: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            RuleAction::SwitchWorkspace { workspace_id } => {
                let output = std::process::Command::new("hyprctl")
                    .args(["dispatch", "workspace", &workspace_id.to_string()])
                    .output()?;
                if !output.status.success() {
                    eprintln!("Failed to switch workspace: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            RuleAction::LaunchApp { app_id } => {
                let output = std::process::Command::new("gtk-launch")
                    .arg(app_id)
                    .output()?;
                if !output.status.success() {
                    eprintln!("Failed to launch app: {}", String::from_utf8_lossy(&output.stderr));
                }
            }
            RuleAction::Speak { text } => {
                let output = std::process::Command::new("sh")
                    .args([
                        "-c",
                        &format!(
                            "ESPEAK_DATA_PATH=~/.local/share/espeak-ng-data LD_LIBRARY_PATH=~/.local/lib:$LD_LIBRARY_PATH piper --model ~/.local/share/piper/voices/en_US-lessac-medium.onnx --output_file /tmp/automation_tts.wav <<< '{}'",
                            text
                        ),
                    ])
                    .output()?;
                if !output.status.success() {
                    eprintln!("TTS failed: {}", String::from_utf8_lossy(&output.stderr));
                } else {
                    let _ = std::process::Command::new("aplay")
                        .args(["/tmp/automation_tts.wav"])
                        .output();
                }
            }
            RuleAction::Log { message } => {
                eprintln!("[automation] {}", message);
            }
        }
        Ok(())
    }

    fn state_changed(&self, state: &State) -> bool {
        let mut last_states = self.last_states.lock().unwrap();
        let current = self.state_to_value(state);
        let changed = last_states.get("state") != Some(&current);
        last_states.insert("state".to_string(), current);
        changed
    }

    fn state_to_value(&self, state: &State) -> serde_json::Value {
        json!({
            "cpu": state.cpu.usage,
            "memory": {
                "used_mb": state.memory.used_mb,
                "total_mb": state.memory.total_mb,
                "used_percent": state.memory.used_percent,
            },
            "battery": {
                "percent": state.battery.percent,
                "status": state.battery.status,
            },
            "disk": {
                "used_gb": state.disk.used_gb,
                "total_gb": state.disk.total_gb,
                "used_percent": state.disk.used_percent,
            },
            "network": {
                "rx_kb": state.network.rx_kb,
                "tx_kb": state.network.tx_kb,
            },
            "temperature": state.temperature.celsius,
            "control": {
                "wifi_enabled": state.control.wifi_enabled,
                "wifi_ssid": state.control.wifi_ssid,
                "bluetooth_enabled": state.control.bluetooth_enabled,
                "bluetooth_connected": state.control.bluetooth_connected,
                "night_light_enabled": state.control.night_light_enabled,
                "night_light_temp": state.control.night_light_temp,
                "power_profile": state.control.power_profile,
            },
            "workspace": {
                "active_id": state.workspace.active_id,
            },
            "ai": {
                "model": state.ai.model,
                "status": state.ai.status,
            },
            "automation": {
                "enabled": state.automation.enabled,
                "rules_count": state.automation.rules_count,
            },
            "theme": state.theme,
        })
    }
}

impl crate::modules::Module for AutomationModule {
    fn name(&self) -> &'static str {
        "automation"
    }

    fn init(&mut self) -> Result<()> {
        self.load_rules()?;
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        state.automation.enabled = true;
        state.automation.rules_count = self.rules.lock().unwrap().len();

        if !self.state_changed(state) {
            return Ok(());
        }

        let rules = self.rules.lock().unwrap().clone();
        for rule in rules {
            if !rule.enabled {
                continue;
            }

            let mut all_match = true;
            for condition in &rule.conditions {
                if !self.evaluate_condition(condition, state) {
                    all_match = false;
                    break;
                }
            }

            if all_match {
                for action in &rule.actions {
                    if let Err(e) = self.execute_action(action) {
                        eprintln!("Failed to execute action for rule '{}': {}", rule.name, e);
                    }
                }
            }
        }
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}