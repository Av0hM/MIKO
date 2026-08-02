use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct Cpu {
    pub usage: f32,
}

#[derive(Serialize, Clone)]
pub struct Memory {
    pub used_percent: f32,
    pub used_mb: u64,
    pub total_mb: u64,
}

#[derive(Serialize, Clone)]
pub struct SystemInfo {
    pub hostname: String,
    pub time: String,
    pub uptime: u64,
}

#[derive(Serialize, Clone)]
pub struct State {
    pub system: SystemInfo,
    pub cpu: Cpu,
    pub memory: Memory,
}
