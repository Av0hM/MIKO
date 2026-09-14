use crate::state::{State, Workspace, WorkspaceInfo};
use anyhow::Result;

pub struct WorkspaceModule {}

impl WorkspaceModule {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    fn get_workspaces(&self) -> Vec<WorkspaceInfo> {
        let output = std::process::Command::new("hyprctl")
            .args(["-j", "workspaces"])
            .output();

        let stdout = match output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => return Vec::new(),
        };

        let workspaces: Vec<serde_json::Value> = match serde_json::from_str(&stdout) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        let mut result = Vec::new();
        for ws in workspaces {
            if let (Some(id), Some(name), Some(monitor), Some(windows)) = (
                ws.get("id").and_then(|v| v.as_i64()),
                ws.get("name").and_then(|v| v.as_str()),
                ws.get("monitor").and_then(|v| v.as_str()),
                ws.get("windows").and_then(|v| v.as_i64()),
            ) {
                result.push(WorkspaceInfo {
                    id: id as i32,
                    name: name.to_string(),
                    monitor: monitor.to_string(),
                    windows: windows as i32,
                    active: false, // Will be set below
                });
            }
        }
        result
    }

    fn get_active_workspace_id(&self) -> i32 {
        let output = std::process::Command::new("hyprctl")
            .args(["-j", "activeworkspace"])
            .output();

        let stdout = match output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
            Err(_) => return 0,
        };

        let ws: serde_json::Value = match serde_json::from_str(&stdout) {
            Ok(v) => v,
            Err(_) => return 0,
        };

        ws.get("id")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
            .unwrap_or(0)
    }
}

impl crate::modules::Module for WorkspaceModule {
    fn name(&self) -> &'static str {
        "workspace"
    }

    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        let mut workspaces = self.get_workspaces();
        let active_id = self.get_active_workspace_id();

        // Mark the active workspace
        for ws in &mut workspaces {
            ws.active = ws.id == active_id;
        }

        state.workspace = Workspace {
            workspaces,
            active_id,
        };

        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}
