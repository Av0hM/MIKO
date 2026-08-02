mod module_manager;
mod modules;
use chrono::Local;
use serde::Serialize;
use std::{fs, path::PathBuf};
use sysinfo::System;
use tokio::time::{Duration, sleep};

#[derive(Serialize)]
struct Cpu {
    usage: f32,
}

#[derive(Serialize)]
struct Memory {
    used_percent: f32,
    used_mb: u64,
    total_mb: u64,
}

#[derive(Serialize)]
struct SystemInfo {
    hostname: String,
    time: String,
    uptime: u64,
}

#[derive(Serialize)]
struct State {
    system: SystemInfo,
    cpu: Cpu,
    memory: Memory,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let home = std::env::var("HOME")?;
    let dir = PathBuf::from(format!("{home}/.local/state/samos"));
    fs::create_dir_all(&dir)?;

    let mut sys = System::new_all();

    println!("SamOS daemon started");

    let mut manager = module_manager::ModuleManager::new();
    manager.register(modules::cpu::CpuModule::new());
    manager.init()?;

    loop {
        sys.refresh_all();

        let total = sys.total_memory() / 1024 / 1024;
        let used = sys.used_memory() / 1024 / 1024;

        let state = State {
            system: SystemInfo {
                hostname: System::host_name().unwrap_or_else(|| "unknown".into()),
                time: Local::now().format("%H:%M:%S").to_string(),
                uptime: System::uptime(),
            },
            cpu: Cpu {
                usage: sys.global_cpu_usage(),
            },
            memory: Memory {
                used_percent: if total > 0 {
                    (used as f32 / total as f32) * 100.0
                } else {
                    0.0
                },
                used_mb: used,
                total_mb: total,
            },
        };

        fs::write(
            dir.join("state.json"),
            serde_json::to_string_pretty(&state)?,
        )?;

        manager.update()?;
        sleep(Duration::from_secs(1)).await;
    }
}
