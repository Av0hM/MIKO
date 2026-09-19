use anyhow::Result;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::{modules::Module, state::State};

pub struct VisualizerModule {
    spectrum: Arc<Mutex<Vec<u8>>>,
    cava_handle: Option<thread::JoinHandle<()>>,
    child: Arc<Mutex<Option<Child>>>,
}

impl VisualizerModule {
    pub fn new() -> Result<Self> {
        Ok(Self {
            spectrum: Arc::new(Mutex::new(vec![0; 120])),
            cava_handle: None,
            child: Arc::new(Mutex::new(None)),
        })
    }

    fn get_now_playing(&self) -> (String, String, String) {
        if let Ok(output) = crate::process::run(
            "playerctl",
            &["metadata", "--format", "{{title}}|{{artist}}|{{status}}"],
            None,
            2,
        ) {
            let parts: Vec<_> = output.split('|').collect();
            if parts.len() == 3 {
                return (parts[0].into(), parts[1].into(), parts[2].into());
            }
        }
        (String::new(), String::new(), "Stopped".to_string())
    }
}

impl Module for VisualizerModule {
    fn name(&self) -> &'static str {
        "visualizer"
    }

    fn init(&mut self) -> Result<()> {
        let config = format!("{}/.config/cava/cava_samos.conf", std::env::var("HOME")?);
        let mut child = Command::new("cava")
            .args(["-p", &config])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().expect("piped Cava stdout");
        *self
            .child
            .lock()
            .map_err(|_| anyhow::anyhow!("Cava unavailable"))? = Some(child);
        let spectrum = self.spectrum.clone();
        self.cava_handle = Some(thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                let values: Vec<u8> = line.split(';').filter_map(|v| v.parse().ok()).collect();
                if values.len() == 120 {
                    if let Ok(mut data) = spectrum.lock() {
                        *data = values;
                    }
                }
            }
            if let Ok(mut data) = spectrum.lock() {
                data.fill(0);
            }
        }));
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
        if let Some(mut child) = self
            .child
            .lock()
            .map_err(|_| anyhow::anyhow!("Cava unavailable"))?
            .take()
        {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(handle) = self.cava_handle.take() {
            let _ = handle.join();
        }
        Ok(())
    }
}

impl Drop for VisualizerModule {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
