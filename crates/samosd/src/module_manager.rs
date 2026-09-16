use anyhow::Result;
use samos_core::{modules::Module, state::State};

pub struct ModuleManager {
    modules: Vec<Box<dyn Module>>,
}

impl ModuleManager {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    pub fn register<M: Module + 'static>(&mut self, module: M) {
        self.modules.push(Box::new(module));
    }

    pub fn init(&mut self) -> Result<()> {
        for module in &mut self.modules {
            module.init()?;
        }
        Ok(())
    }

    pub fn update(&mut self, state: &mut State) -> Result<()> {
        for module in &mut self.modules {
            module.update(state)?;
        }
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<()> {
        for module in &mut self.modules {
            module.shutdown()?;
        }
        Ok(())
    }
}
