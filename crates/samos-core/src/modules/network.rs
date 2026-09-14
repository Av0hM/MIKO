use anyhow::Result;
use sysinfo::Networks;

use crate::{modules::Module, state::State};

pub struct NetworkModule {
    networks: Networks,
}

impl NetworkModule {
    pub fn new() -> Self {
        Self {
            networks: Networks::new_with_refreshed_list(),
        }
    }
}

impl Module for NetworkModule {
    fn name(&self) -> &'static str {
        "network"
    }

    fn init(&mut self) -> Result<()> {
        self.networks.refresh(true);
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        self.networks.refresh(true);

        let mut rx = 0u64;
        let mut tx = 0u64;

        for (_, data) in self.networks.iter() {
            rx += data.received();
            tx += data.transmitted();
        }

        state.network.rx_kb = rx / 1024;
        state.network.tx_kb = tx / 1024;

        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
