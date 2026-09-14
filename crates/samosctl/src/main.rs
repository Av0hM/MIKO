use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use toml;

#[derive(Parser)]
#[command(name = "samosctl", about = "SamOS control utility")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Toggle WiFi radio
    WifiToggle,
    /// Toggle Bluetooth radio
    BluetoothToggle,
    /// Set power profile (balanced, power-saver, performance)
    PowerProfile { profile: String },
    /// Get current theme
    ThemeGet,
    /// Set theme
    ThemeSet { theme: String },
    /// List available themes
    ThemeList,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::WifiToggle => toggle_wifi()?,
        Commands::BluetoothToggle => toggle_bluetooth()?,
        Commands::PowerProfile { profile } => set_power_profile(&profile)?,
        Commands::ThemeGet => theme_get()?,
        Commands::ThemeSet { theme } => theme_set(&theme)?,
        Commands::ThemeList => theme_list()?,
    }

    Ok(())
}

fn toggle_wifi() -> Result<()> {
    let output = std::process::Command::new("nmcli")
        .args(["radio", "wifi"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let enabled = stdout == "enabled";
    let new_state = !enabled;

    std::process::Command::new("nmcli")
        .args(["radio", "wifi", if new_state { "on" } else { "off" }])
        .output()?;

    println!("WiFi: {}", if new_state { "enabled" } else { "disabled" });
    Ok(())
}

fn toggle_bluetooth() -> Result<()> {
    let output = std::process::Command::new("bluetoothctl")
        .args(["show"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let powered = stdout.lines().any(|l| l.trim().starts_with("Powered: yes"));
    let new_state = !powered;

    std::process::Command::new("bluetoothctl")
        .args(["power", if new_state { "on" } else { "off" }])
        .output()?;

    println!(
        "Bluetooth: {}",
        if new_state { "enabled" } else { "disabled" }
    );
    Ok(())
}

fn set_power_profile(profile: &str) -> Result<()> {
    std::process::Command::new("powerprofilesctl")
        .args(["set", profile])
        .output()?;

    println!("Power profile set to: {}", profile);
    Ok(())
}

fn theme_get() -> Result<()> {
    let home = std::env::var("HOME")?;
    let path = PathBuf::from(home).join(".config/samos/config.toml");

    if path.exists() {
        let content = fs::read_to_string(&path)?;
        let config: toml::Value = toml::from_str(&content)?;
        if let Some(theme) = config.get("theme").and_then(|v| v.as_str()) {
            println!("Current theme: {}", theme);
        } else {
            println!("Current theme: hud (default)");
        }
    } else {
        println!("Current theme: hud (default)");
    }
    Ok(())
}

fn theme_list() -> Result<()> {
    let home = std::env::var("HOME")?;
    let themes_dir = PathBuf::from(home).join(".config/samos/themes");

    if themes_dir.exists() {
        for entry in fs::read_dir(&themes_dir)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".toml") {
                    let theme_name = name.trim_end_matches(".toml");
                    println!("{}", theme_name);
                }
            }
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct ConfigToml {
    theme: String,
    monitor: String,
    refresh_ms: u64,
}

fn theme_set(theme: &str) -> Result<()> {
    let home = std::env::var("HOME")?;
    let config_path = PathBuf::from(&home).join(".config/samos/config.toml");
    let theme_path = PathBuf::from(&home)
        .join(".config/samos/themes")
        .join(format!("{}.toml", theme));

    if !theme_path.exists() {
        eprintln!("Theme '{}' not found", theme);
        return Ok(());
    }

    let config = ConfigToml {
        theme: theme.to_string(),
        monitor: "focused".to_string(),
        refresh_ms: 1000,
    };

    fs::write(&config_path, toml::to_string_pretty(&config)?)?;
    println!("Theme set to: {}", theme);

    // Restart samosd to pick up new theme
    std::process::Command::new("systemctl")
        .args(["--user", "restart", "samosd.service"])
        .output()?;

    // Generate theme SCSS variables
    std::process::Command::new("sh")
        .args([
            "-c",
            &format!("~/.config/eww/scripts/theme_switch.sh {}", theme),
        ])
        .output()?;

    // Reload Eww to apply new theme
    std::process::Command::new("eww")
        .args(["reload"])
        .output()?;

    println!("Theme applied and services reloaded");
    Ok(())
}
