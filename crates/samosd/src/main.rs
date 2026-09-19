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
    if let Err(error) = init_global_plugin_manager(&plugin_dir) {
        eprintln!("[plugins] Initialization failed: {error}");
    }

    let mut manager = module_manager::ModuleManager::new();
    manager.register(modules::cpu::CpuModule::new());
    manager.register(modules::memory::MemoryModule::new());
    manager.register(modules::battery::BatteryModule::new());
    manager.register(modules::disk::DiskModule::new());
    manager.register(modules::network::NetworkModule::new());
    manager.register(modules::temperature::TemperatureModule::new());
    match modules::control::ControlModule::new() {
        Ok(module) => manager.register(module),
        Err(error) => eprintln!("[module:control] Unavailable: {error}"),
    }
    match modules::workspace::WorkspaceModule::new() {
        Ok(module) => manager.register(module),
        Err(error) => eprintln!("[module:workspace] Unavailable: {error}"),
    }
    match modules::automation::AutomationModule::new() {
        Ok(module) => manager.register(module),
        Err(error) => eprintln!("[module:automation] Unavailable: {error}"),
    }
    match modules::visualizer::VisualizerModule::new() {
        Ok(module) => manager.register(module),
        Err(error) => eprintln!("[module:visualizer] Unavailable: {error}"),
    }
    match modules::ai::AiModule::new() {
        Ok(module) => manager.register(module),
        Err(error) => eprintln!("[module:ai] Unavailable: {error}"),
    }
    manager.init()?;

    let mut ipc_server = ipc::StateIpcServer::new()?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    let outcome: Result<()> = async {
        loop {
            match Config::load(&config_path) {
                Ok(updated) => config = updated,
                Err(error) => eprintln!("[config] Keeping last valid settings: {error}"),
            }
            let mut state = State::default();
            state.theme = config.theme.clone();

            state.system.hostname = std::fs::read_to_string("/etc/hostname")
                .unwrap_or_default()
                .trim()
                .to_string();

            state.system.time = Local::now().format("%H:%M:%S").to_string();
            state.system.uptime = System::uptime();
            manager.update(&mut state)?;
            if let Err(error) = update_global_plugins(&mut state) {
                eprintln!("[plugins] Update failed: {error}");
            }

            export::export(&state)?;
            ipc_server.update_state(&state);

            tokio::select! {
                _ = sleep(Duration::from_millis(config.refresh_ms)) => {},
                _ = terminate.recv() => break,
                _ = interrupt.recv() => break,
            }
        }
        Ok(())
    }
    .await;
    ipc_server.shutdown();
    let _ = manager.shutdown();
    let _ = shutdown_global_plugins();
    eprintln!("SamOS daemon shutdown complete");
    outcome
}
