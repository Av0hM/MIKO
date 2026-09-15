use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Runtime settings. Omitted fields retain their documented defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub theme: String,
    pub monitor: String,
    pub refresh_ms: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self { theme: "hud".into(), monitor: "focused".into(), refresh_ms: 1000 }
    }
}
impl Config {
    /// Standard user configuration path.
    pub fn path() -> Result<PathBuf> {
        Ok(PathBuf::from(std::env::var("HOME")?).join(".config/samos/config.toml"))
    }
    /// Read and validate configuration without silently discarding malformed settings.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() { return Ok(Self::default()); }
        let config: Self = toml::from_str(&std::fs::read_to_string(path)?)
            .context("Invalid SamOS configuration")?;
        ensure!((100..=60_000).contains(&config.refresh_ms), "refresh_ms must be between 100 and 60000");
        validate_name(&config.theme)?;
        Ok(config)
    }
}
/// Accept plain theme identifiers, never paths or shell fragments.
pub fn validate_name(name: &str) -> Result<()> {
    ensure!(!name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "Invalid theme name");
    Ok(())
}
/// Change only the theme key, retaining all other user settings.
pub fn set_theme_at(path: &Path, theme: &str) -> Result<()> {
    validate_name(theme)?;
    let mut value: toml::Table = if path.exists() { toml::from_str(&std::fs::read_to_string(path)?)? } else { toml::Table::new() };
    value.insert("theme".into(), toml::Value::String(theme.into()));
    let parent = path.parent().context("Configuration needs a parent directory")?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".config-{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, toml::to_string_pretty(&value)?)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_config_preserves_theme() {
        let value: Config = toml::from_str("theme='hacker'").unwrap();
        assert_eq!(value.theme, "hacker"); assert_eq!(value.refresh_ms, 1000);
    }
    #[test]
    fn rejects_paths_and_shell_syntax() {
        for name in ["../hud", "hud;true", "", "a b", "a\"b"] { assert!(validate_name(name).is_err()); }
    }
    #[test]
    fn theme_preserves_other_settings() {
        let folder = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&folder).unwrap(); let path=folder.join("config.toml");
        std::fs::write(&path,"theme='hud'\nmonitor='HDMI-A-1'\nrefresh_ms=500\ncustom='keep'\n").unwrap();
        set_theme_at(&path,"hacker").unwrap();
        let value:toml::Table=toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["custom"].as_str(),Some("keep")); assert_eq!(value["refresh_ms"].as_integer(),Some(500));
        std::fs::remove_dir_all(folder).unwrap();
    }
}
