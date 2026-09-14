use crate::state::{Control, State};
use anyhow::Result;

pub struct ControlModule {}

impl ControlModule {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    fn get_wifi_status(&self) -> (bool, String) {
        let output = std::process::Command::new("nmcli")
            .args(["radio", "wifi"])
            .output();

        let enabled = output
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim() == "enabled")
            .unwrap_or(false);

        let output = std::process::Command::new("nmcli")
            .args(["-t", "-f", "NAME,TYPE", "connection", "show", "--active"])
            .output();

        let mut ssid = String::new();
        if let Ok(out) = output {
            if let Ok(stdout) = String::from_utf8(out.stdout) {
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 2 && (parts[1] == "802-11-wireless" || parts[1] == "wifi") {
                        ssid = parts[0].to_string();
                        break;
                    }
                }
            }
        }

        (enabled, ssid)
    }

    fn get_bluetooth_status(&self) -> (bool, String) {
        let output = std::process::Command::new("bluetoothctl")
            .args(["show"])
            .output();

        let powered = output
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|stdout| stdout.lines().any(|l| l.trim().starts_with("Powered: yes")))
            .unwrap_or(false);

        let output = std::process::Command::new("bluetoothctl")
            .args(["info"])
            .output();

        let stdout = output
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();

        let connected = stdout
            .lines()
            .any(|l| l.trim().starts_with("Connected: yes"));

        let mut device_name = String::new();
        if connected {
            for line in stdout.lines() {
                if line.trim().starts_with("Name: ") {
                    device_name = line.trim().strip_prefix("Name: ").unwrap_or("").to_string();
                    break;
                }
            }
        }

        (powered, device_name)
    }

    fn get_power_profile(&self) -> String {
        let output = std::process::Command::new("powerprofilesctl")
            .arg("get")
            .output()
            .ok();

        output
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
            .unwrap_or_else(|| "unknown".to_string())
    }

    // Toggle methods
    pub fn toggle_wifi(&self) -> Result<bool> {
        let output = std::process::Command::new("nmcli")
            .args(["radio", "wifi"])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let enabled = stdout == "enabled";
        let new_state = !enabled;

        std::process::Command::new("nmcli")
            .args(["radio", "wifi", if new_state { "on" } else { "off" }])
            .output()?;

        Ok(new_state)
    }

    pub fn toggle_bluetooth(&self) -> Result<bool> {
        let output = std::process::Command::new("bluetoothctl")
            .args(["show"])
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let powered = stdout.lines().any(|l| l.trim().starts_with("Powered: yes"));
        let new_state = !powered;

        std::process::Command::new("bluetoothctl")
            .args(["power", if new_state { "on" } else { "off" }])
            .output()?;

        Ok(new_state)
    }

    pub fn set_power_profile(&self, profile: &str) -> Result<()> {
        std::process::Command::new("powerprofilesctl")
            .args(["set", profile])
            .output()?;
        Ok(())
    }
}

impl crate::modules::Module for ControlModule {
    fn name(&self) -> &'static str {
        "control"
    }

    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        let (wifi_enabled, wifi_ssid) = self.get_wifi_status();
        let (bluetooth_enabled, bluetooth_connected) = self.get_bluetooth_status();
        let power_profile = self.get_power_profile();

        state.control = Control {
            wifi_enabled,
            wifi_ssid,
            bluetooth_enabled,
            bluetooth_connected,
            night_light_enabled: false,
            night_light_temp: 4000,
            power_profile,
        };

        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
