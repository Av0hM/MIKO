pub mod cpu;

pub trait Module {
    fn name(&self) -> &'static str;
    fn init(&mut self) -> anyhow::Result<()>;
    fn update(&mut self) -> anyhow::Result<()>;
    fn shutdown(&mut self) -> anyhow::Result<()>;
}
