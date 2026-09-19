use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
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
    let output = std::process::Command::new("timeout")
        .args(["8s", "nmcli"])
        .env("LC_ALL", "C")
        .args(["radio", "wifi"])
        .output()?;

    ensure!(
        output.status.success(),
        "Status query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let enabled = stdout == "enabled";
    let new_state = !enabled;

    checked(
        "nmcli",
        &["radio", "wifi", if new_state { "on" } else { "off" }],
    )?;

    println!("WiFi: {}", if new_state { "enabled" } else { "disabled" });
    Ok(())
}

fn toggle_bluetooth() -> Result<()> {
    let output = std::process::Command::new("timeout")
        .args(["8s", "bluetoothctl"])
        .env("LC_ALL", "C")
        .args(["show"])
        .output()?;

    ensure!(
        output.status.success(),
        "Status query failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let powered = stdout.lines().any(|l| l.trim().starts_with("Powered: yes"));
    let new_state = !powered;

    checked(
        "bluetoothctl",
        &["power", if new_state { "on" } else { "off" }],
    )?;

    println!(
        "Bluetooth: {}",
        if new_state { "enabled" } else { "disabled" }
    );
    Ok(())
}

fn set_power_profile(profile: &str) -> Result<()> {
    ensure!(
        ["balanced", "power-saver", "performance"].contains(&profile),
        "Invalid power profile"
    );
    checked("powerprofilesctl", &["set", profile])?;

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

fn theme_set(theme: &str) -> Result<()> {
    samos_core::config::validate_name(theme)?;
    let home = std::env::var("HOME")?;
    let theme_path = PathBuf::from(home)
        .join(".config/samos/themes")
        .join(format!("{theme}.toml"));
    ensure!(theme_path.is_file(), "Theme '{theme}' not found");
    samos_core::config::set_theme_at(&samos_core::config::Config::path()?, theme)?;
    println!("Theme set to: {theme}");
    Ok(())
}

fn checked(program: &str, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new("timeout")
        .args(["8s", program])
        .args(args)
        .env("LC_ALL", "C")
        .output()?;
    ensure!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
