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
        self.modules.retain_mut(|module| match module.init() {
            Ok(()) => true,
            Err(error) => { eprintln!("[{}] Initialization failed; disabled: {error}", module.name()); let _ = module.shutdown(); false }
        });
        Ok(())
    }

    pub fn update(&mut self, state: &mut State) -> Result<()> {
        for module in &mut self.modules {
            if let Err(error) = module.update(state) { eprintln!("[{}] Update failed: {error}", module.name()); }
        }
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<()> {
        for module in &mut self.modules {
            if let Err(error) = module.shutdown() { eprintln!("[{}] Shutdown failed: {error}", module.name()); }
        }
        Ok(())
    }
}
