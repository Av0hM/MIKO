use anyhow::Result;
use sysinfo::Components;

use crate::{modules::Module, state::State};

pub struct TemperatureModule {
    components: Components,
}

impl TemperatureModule {
    pub fn new() -> Self {
        Self {
            components: Components::new_with_refreshed_list(),
        }
    }
}

impl Module for TemperatureModule {
    fn name(&self) -> &'static str {
        "temperature"
    }

    fn init(&mut self) -> Result<()> {
        self.components.refresh(true);
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        self.components.refresh(true);

        if let Some(component) = self.components.list().first() {
            state.temperature.celsius = component.temperature().unwrap_or(0.0);
        }

        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
