use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Cpu {
    pub usage: f32,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Memory {
    pub total_mb: u64,
    pub used_mb: u64,
    pub used_percent: f32,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Battery {
    pub percent: f32,
    pub status: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Disk {
    pub total_gb: u64,
    pub used_gb: u64,
    pub used_percent: f32,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Network {
    pub rx_kb: u64,
    pub tx_kb: u64,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Temperature {
    pub celsius: f32,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Control {
    pub wifi_enabled: bool,
    pub wifi_ssid: String,
    pub bluetooth_enabled: bool,
    pub bluetooth_connected: String,
    pub night_light_enabled: bool,
    pub night_light_temp: u32,
    pub power_profile: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct WorkspaceInfo {
    pub id: i32,
    pub name: String,
    pub monitor: String,
    pub windows: i32,
    pub active: bool,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Workspace {
    pub workspaces: Vec<WorkspaceInfo>,
    pub active_id: i32,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AiState {
    pub model: String,
    pub status: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Automation {
    pub enabled: bool,
    pub rules_count: usize,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct SystemInfo {
    pub hostname: String,
    pub kernel: String,
    pub uptime: u64,
    pub time: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct State {
    pub cpu: Cpu,
    pub memory: Memory,
    pub battery: Battery,
    pub disk: Disk,
    pub network: Network,
    pub temperature: Temperature,
    pub control: Control,
    pub workspace: Workspace,
    pub ai: AiState,
    pub automation: Automation,
    pub visualizer: Visualizer,
    pub system: SystemInfo,
    pub theme: String,
    pub plugin_data: HashMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    #[default]
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    Contains,
    StartsWith,
    EndsWith,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct RuleCondition {
    pub field: String,
    pub operator: ConditionOperator,
    pub value: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleAction {
    #[default]
    Notify,
    RunCommand { command: String },
    SetTheme { theme: String },
    SetPowerProfile { profile: String },
    ToggleWifi,
    ToggleBluetooth,
    SwitchWorkspace { workspace_id: i32 },
    LaunchApp { app_id: String },
    Speak { text: String },
    Log { message: String },
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub conditions: Vec<RuleCondition>,
    pub actions: Vec<RuleAction>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Visualizer {
    pub spectrum: Vec<u8>,
    pub now_playing_title: String,
    pub now_playing_artist: String,
    pub now_playing_status: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct AutomationState {
    pub enabled: bool,
    pub rules: Vec<Rule>,
}