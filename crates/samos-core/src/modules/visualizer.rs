use anyhow::Result;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::io::{BufRead, BufReader};

use crate::{modules::Module, state::State};

pub struct VisualizerModule {
    spectrum: Arc<Mutex<Vec<u8>>>,
    cava_handle: Option<thread::JoinHandle<()>>,
}

impl VisualizerModule {
    pub fn new() -> Result<Self> {
        let spectrum = Arc::new(Mutex::new(vec![0u8; 32]));
        let spectrum_clone = spectrum.clone();

        let handle = thread::spawn(move || {
            let config_path = format!("{}/.config/cava/config", std::env::var("HOME").unwrap_or_default());
            
            let mut child = match Command::new("cava")
                .args(["-p", &config_path])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[visualizer] Failed to spawn cava: {}", e);
                    return;
                }
            };

            let stdout = match child.stdout.take() {
                Some(s) => s,
                None => {
                    eprintln!("[visualizer] Failed to capture cava stdout");
                    return;
                }
            };

            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        let values: Vec<u8> = l.split(';')
                            .filter_map(|s| s.parse::<u8>().ok())
                            .collect();
                        if values.len() == 32 {
                            *spectrum_clone.lock().unwrap() = values;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            spectrum,
            cava_handle: Some(handle),
        })
    }

    fn get_now_playing(&self) -> (String, String, String) {
        let output = Command::new("playerctl")
            .args(["metadata", "--format", "{{title}}|{{artist}}|{{status}}"])
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let parts: Vec<&str> = stdout.split('|').collect();
                if parts.len() == 3 {
                    return (parts[0].to_string(), parts[1].to_string(), parts[2].to_string());
                }
            }
            _ => {}
        }
        (String::new(), String::new(), "Stopped".to_string())
    }
}

impl Module for VisualizerModule {
    fn name(&self) -> &'static str {
        "visualizer"
    }

    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        let spectrum = self.spectrum.lock().unwrap().clone();
        state.visualizer.spectrum = spectrum;

        let (title, artist, status) = self.get_now_playing();
        state.visualizer.now_playing_title = title;
        state.visualizer.now_playing_artist = artist;
        state.visualizer.now_playing_status = status;

        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        if let Some(handle) = self.cava_handle.take() {
            let _ = handle.join();
        }
        Ok(())
    }
}