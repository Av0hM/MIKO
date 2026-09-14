use anyhow::Result;
use sysinfo::System;

use crate::{modules::Module, state::State};

pub struct CpuModule {
    sys: System,
}

impl CpuModule {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_cpu_all();
        Self { sys }
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

    fn update(&mut self, state: &mut State) -> Result<()> {
        self.sys.refresh_cpu_usage();
        state.cpu.usage = self.sys.global_cpu_usage();
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
