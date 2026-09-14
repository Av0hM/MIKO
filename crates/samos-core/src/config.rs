use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub theme: String,
    pub monitor: String,
    pub refresh_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "hud".into(),
            monitor: "focused".into(),
            refresh_ms: 1000,
        }
    }
}
