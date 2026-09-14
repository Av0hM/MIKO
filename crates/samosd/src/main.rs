mod export;
mod module_manager;
mod modules;

use anyhow::Result;
use chrono::Local;
use samos_core::{
    config::Config,
    modules::{init_global_plugin_manager, shutdown_global_plugins, update_global_plugins},
    state::State,
};
use std::path::PathBuf;
use sysinfo::System;
use tokio::time::{Duration, sleep};

fn load_config() -> Config {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = PathBuf::from(home).join(".config/samos/config.toml");

    if path.exists() {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&content).unwrap_or_default()
    } else {
        Config::default()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("SamOS daemon started");

    let config = load_config();

    // Initialize plugin manager
    let plugin_dir = std::env::var("HOME").unwrap_or_default() + "/.local/lib/samos/plugins";
    init_global_plugin_manager(&plugin_dir)?;

    let mut manager = module_manager::ModuleManager::new();
    manager.register(modules::cpu::CpuModule::new());
    manager.register(modules::memory::MemoryModule::new());
    manager.register(modules::battery::BatteryModule::new());
    manager.register(modules::disk::DiskModule::new());
    manager.register(modules::network::NetworkModule::new());
    manager.register(modules::temperature::TemperatureModule::new());
    manager.register(modules::control::ControlModule::new()?);
    manager.register(modules::workspace::WorkspaceModule::new()?);
    manager.register(modules::miko::MikoModule::new()?);
    manager.register(modules::automation::AutomationModule::new()?);
    manager.register(modules::visualizer::VisualizerModule::new()?);
    manager.register(modules::ai::AiModule::new()?);
    manager.init()?;

    loop {
        let mut state = State::default();
        state.theme = config.theme.clone();

        manager.update(&mut state)?;
        update_global_plugins(&mut state)?;

        state.system.hostname = std::fs::read_to_string("/etc/hostname")
            .unwrap_or_default()
            .trim()
            .to_string();

        state.system.time = Local::now().format("%H:%M:%S").to_string();
        state.system.uptime = System::uptime();

        export::export(&state)?;

        sleep(Duration::from_millis(config.refresh_ms)).await;
    }
}
