use crate::state::State;
use anyhow::Result;

pub mod ai;
pub mod automation;
pub mod battery;
pub mod control;
pub mod cpu;
pub mod disk;
pub mod memory;
pub mod miko;
pub mod network;
pub mod plugin;
pub mod temperature;
pub mod visualizer;
pub mod workspace;

pub use plugin::{
    PluginManager, init_global_plugin_manager, shutdown_global_plugins, update_global_plugins,
};

pub trait Module {
    fn name(&self) -> &'static str;

    fn init(&mut self) -> Result<()>;

    fn update(&mut self, state: &mut State) -> Result<()>;

    fn shutdown(&mut self) -> Result<()>;
}

pub trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn description(&self) -> &'static str;

    fn init(&mut self) -> Result<()>;
    fn update(&mut self, state: &mut State) -> Result<()>;
    fn shutdown(&mut self) -> Result<()>;
}
