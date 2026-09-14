use anyhow::Result;
use sysinfo::Disks;

use crate::{modules::Module, state::State};

pub struct DiskModule {
    disks: Disks,
}

impl DiskModule {
    pub fn new() -> Self {
        Self {
            disks: Disks::new_with_refreshed_list(),
        }
    }
}

impl Module for DiskModule {
    fn name(&self) -> &'static str {
        "disk"
    }

    fn init(&mut self) -> Result<()> {
        self.disks.refresh(true);
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        self.disks.refresh(true);

        if let Some(disk) = self
            .disks
            .list()
            .iter()
            .find(|d| d.mount_point().to_str() == Some("/"))
        {
            let total = disk.total_space() / 1024 / 1024 / 1024;
            let available = disk.available_space() / 1024 / 1024 / 1024;
            let used = total.saturating_sub(available);

            state.disk.total_gb = total;
            state.disk.used_gb = used;
            state.disk.used_percent = if total > 0 {
                (used as f32 / total as f32) * 100.0
            } else {
                0.0
            };
        }

        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
