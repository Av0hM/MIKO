use anyhow::Result;
use samos_core::state::State;
use std::{fs, path::PathBuf};

pub fn export(state: &State) -> Result<()> {
    let home = std::env::var("HOME")?;
    let path = PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("samos");

    fs::create_dir_all(&path)?;

    fs::write(
        path.join("state.json"),
        serde_json::to_string_pretty(state)?,
    )?;

    Ok(())
}
