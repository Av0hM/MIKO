mod export;
mod ipc;
mod module_manager;
mod modules;

use anyhow::Result;
use chrono::Local;
use samos_core::{
    config::Config,
    modules::{init_global_plugin_manager, shutdown_global_plugins, update_global_plugins},
    state::State,
};
use sysinfo::System;
use tokio::time::{Duration, sleep};

#[tokio::main]
async fn main() -> Result<()> {
    println!("SamOS daemon started");

    let config_path = Config::path()?;
    let mut config = Config::load(&config_path)?;

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
    manager.register(modules::automation::AutomationModule::new()?);
    manager.register(modules::visualizer::VisualizerModule::new()?);
    manager.register(modules::ai::AiModule::new()?);
    manager.init()?;

    let ipc_server = ipc::StateIpcServer::new()?;

    loop {
        match Config::load(&config_path) {
            Ok(updated) => config = updated,
            Err(error) => eprintln!("[config] Keeping last valid settings: {error}"),
        }
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
        ipc_server.update_state(&state);

        sleep(Duration::from_millis(config.refresh_ms)).await;
    }
    ipc_server.shutdown();
}
