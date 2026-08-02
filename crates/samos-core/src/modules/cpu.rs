use anyhow::Result;
use serde::Serialize;
use sysinfo::System;

use crate::modules::Module;

#[derive(Serialize, Clone, Default)]
pub struct CpuState {
    pub usage: f32,
}

pub struct CpuModule {
    sys: System,
    pub state: CpuState,
}

impl CpuModule {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_cpu_all();

        Self {
            sys,
            state: CpuState::default(),
        }
    }
}

impl Module for CpuModule {
    fn name(&self) -> &'static str {
        "cpu"
    }

    fn init(&mut self) -> Result<()> {
        self.sys.refresh_cpu_all();
        Ok(())
    }

    fn update(&mut self) -> Result<()> {
        self.sys.refresh_cpu_usage();
        self.state.usage = self.sys.global_cpu_usage();
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
