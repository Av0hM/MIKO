use anyhow::Result;
use std::fs;

use crate::{modules::Module, state::State};

pub struct BatteryModule;

impl BatteryModule {
    pub fn new() -> Self {
        Self
    }
}

impl Module for BatteryModule {
    fn name(&self) -> &'static str {
        "battery"
    }

    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        let base = "/sys/class/power_supply/BAT0";
        let capacity = fs::read_to_string(format!("{base}/capacity"))
            .unwrap_or_default()
            .trim()
            .parse::<f32>()
            .unwrap_or(0.0);
        let status = fs::read_to_string(format!("{base}/status"))
            .unwrap_or_default()
            .trim()
            .to_string();

        state.battery.percent = capacity;
        state.battery.status = status;

        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
