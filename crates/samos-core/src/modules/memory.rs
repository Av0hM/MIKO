use anyhow::Result;
use sysinfo::System;

use crate::{modules::Module, state::State};

pub struct MemoryModule {
    sys: System,
}

impl MemoryModule {
    pub fn new() -> Self {
        Self {
            sys: System::new_all(),
        }
    }
}

impl Module for MemoryModule {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn init(&mut self) -> Result<()> {
        self.sys.refresh_memory();
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        self.sys.refresh_memory();

        let total = self.sys.total_memory() / 1024 / 1024;
        let used = self.sys.used_memory() / 1024 / 1024;

        state.memory.total_mb = total;
        state.memory.used_mb = used;
        state.memory.used_percent = if total > 0 {
            (used as f32 / total as f32) * 100.0
        } else {
            0.0
        };

        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
