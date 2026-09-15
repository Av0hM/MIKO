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
    let mut config = Config::load(&config_path).unwrap_or_else(|e| { eprintln!("[config] {e}; starting with defaults"); Config::default() });
    let mut ipc_server = ipc::StateIpcServer::new()?;

    // Initialize plugin manager
    let plugin_dir = std::env::var("HOME").unwrap_or_default() + "/.local/lib/samos/plugins";
    if let Err(error) = init_global_plugin_manager(&plugin_dir) { eprintln!("[plugin] Plugin loading unavailable: {error}"); }

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
    match modules::ai::AiModule::new() { Ok(module) => manager.register(module), Err(error) => eprintln!("[ai] Disabled: {error}") }
    manager.init()?;

    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

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
        state.system.kernel = System::kernel_version().unwrap_or_default();

        manager.update(&mut state)?;
        update_global_plugins(&mut state)?;

        export::export(&state)?;
        ipc_server.update_state(&state);

        tokio::select! {
            _ = sleep(Duration::from_millis(config.refresh_ms)) => {},
            _ = terminate.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    manager.shutdown()?;
    shutdown_global_plugins()?;
    ipc_server.shutdown();
    Ok(())
}
