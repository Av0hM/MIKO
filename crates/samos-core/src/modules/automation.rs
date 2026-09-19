//! Edge-triggered rules, hot reload and bounded off-thread action execution.
use crate::{
    modules::Module,
    state::{Rule, RuleAction, State},
};
use anyhow::{Result, ensure};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::Duration,
};

pub struct AutomationModule {
    path: PathBuf,
    rules: Arc<Mutex<Vec<Rule>>>,
    last: HashMap<String, bool>,
    loaded: Option<Vec<u8>>,
    rejected: Option<Vec<u8>>,
    queue: Option<SyncSender<Rule>>,
    worker: Option<thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}
impl AutomationModule {
    pub fn new() -> Result<Self> {
        Ok(Self::at(
            PathBuf::from(std::env::var("HOME")?).join(".config/samos/rules.json"),
        ))
    }
    fn at(path: PathBuf) -> Self {
        Self {
            path,
            rules: Arc::new(Mutex::new(vec![])),
            last: HashMap::new(),
            loaded: None,
            rejected: None,
            queue: None,
            worker: None,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
    fn reload(&mut self) -> Result<()> {
        let bytes = match std::fs::read(&self.path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => b"[]".to_vec(),
            Err(e) => return Err(e.into()),
        };
        if self.loaded.as_ref() == Some(&bytes) || self.rejected.as_ref() == Some(&bytes) {
            return Ok(());
        }
        let parsed = (|| -> Result<Vec<Rule>> {
            ensure!(bytes.len() <= 262144, "Rules file exceeds 256 KiB");
            let rules: Vec<Rule> = serde_json::from_slice(&bytes)?;
            crate::rules::validate(&rules)?;
            Ok(rules)
        })();
        match parsed {
            Ok(rules) => {
                let mut current = self
                    .rules
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Rules unavailable"))?;
                self.last.retain(|id, _| {
                    current
                        .iter()
                        .find(|r| &r.id == id)
                        .zip(rules.iter().find(|r| &r.id == id))
                        .is_some_and(|(a, b)| {
                            serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
                        })
                });
                *current = rules;
                self.loaded = Some(bytes);
                self.rejected = None;
            }
            Err(e) => {
                self.rejected = Some(bytes);
                return Err(e);
            }
        }
        Ok(())
    }
    fn triggered(&mut self, state: &State) -> Result<Vec<Rule>> {
        let state = serde_json::to_value(state)?;
        let mut found = vec![];
        for rule in self
            .rules
            .lock()
            .map_err(|_| anyhow::anyhow!("Rules unavailable"))?
            .iter()
        {
            let matches = rule.enabled
                && rule
                    .conditions
                    .iter()
                    .all(|c| crate::rules::evaluate(c, &state));
            let before = self.last.insert(rule.id.clone(), matches).unwrap_or(false);
            if matches && !before {
                found.push(rule.clone());
            }
        }
        Ok(found)
    }
}
fn execute(action: &RuleAction, stop: &AtomicBool) -> Result<()> {
    use RuleAction::*;
    let home = PathBuf::from(std::env::var("HOME")?);
    let ctl = home.join(".local/bin/samosctl");
    let ctl = if ctl.is_file() {
        ctl.to_string_lossy().into_owned()
    } else {
        "samosctl".into()
    };
    let run = |program: &str, args: &[&str]| {
        crate::process::run_cancellable(program, args, None, 8, stop).map(|_| ())
    };
    match action {
        RunCommand { command } => execute(&crate::rules::parse_command(command)?, stop),
        Notify => run("notify-send", &["SamOS", "Rule triggered"]),
        Log { message } => {
            eprintln!("[automation] {message}");
            Ok(())
        }
        SetTheme { theme } => run(&ctl, &["theme-set", theme]),
        SetPowerProfile { profile } => run("powerprofilesctl", &["set", profile]),
        ToggleWifi => run(&ctl, &["wifi-toggle"]),
        ToggleBluetooth => run(&ctl, &["bluetooth-toggle"]),
        SwitchWorkspace { workspace_id } => run(
            "hyprctl",
            &["dispatch", "workspace", &workspace_id.to_string()],
        ),
        LaunchApp { app_id } => run("gtk-launch", &[app_id]),
        Speak { text } => {
            let audio = crate::audio::shared();
            let guard = audio.acquire(crate::audio::SYNTHESIZING, Duration::from_secs(2))?;
            let dir = std::env::temp_dir().join(format!("samos-rule-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&dir)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
            let result = (|| -> Result<()> {
                let wav = dir.join("speech.wav");
                let voice = home.join(".local/share/piper/voices/en_US-lessac-medium.onnx");
                let binary = home.join(".local/bin/piper");
                let espeak = format!(
                    "ESPEAK_DATA_PATH={}",
                    home.join(".local/share/espeak-ng-data").display()
                );
                let libs = format!(
                    "LD_LIBRARY_PATH={}:{}",
                    home.join(".local/lib").display(),
                    std::env::var("LD_LIBRARY_PATH").unwrap_or_default()
                );
                crate::process::run_cancellable(
                    "env",
                    &[
                        &espeak,
                        &libs,
                        &binary.to_string_lossy(),
                        "--model",
                        &voice.to_string_lossy(),
                        "--output_file",
                        &wav.to_string_lossy(),
                    ],
                    Some(text),
                    60,
                    stop,
                )?;
                guard.phase(crate::audio::SPEAKING);
                crate::process::run_cancellable(
                    "aplay",
                    &["-q", &wav.to_string_lossy()],
                    None,
                    90,
                    stop,
                )?;
                Ok(())
            })();
            let _ = std::fs::remove_dir_all(dir);
            result
        }
    }
}
impl Module for AutomationModule {
    fn name(&self) -> &'static str {
        "automation"
    }
    fn init(&mut self) -> Result<()> {
        if let Err(e) = self.reload() {
            eprintln!("[automation] Retaining last valid rules: {e}");
        }
        let (tx, rx) = mpsc::sync_channel::<Rule>(16);
        self.queue = Some(tx);
        let stop = self.stop.clone();
        let rules = self.rules.clone();
        self.worker = Some(thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let rule = match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(rule) => rule,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                for action in &rule.actions {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let still_valid = rules.lock().is_ok_and(|rs| {
                        rs.iter().any(|r| {
                            r.id == rule.id
                                && r.enabled
                                && serde_json::to_value(r).ok() == serde_json::to_value(&rule).ok()
                        })
                    });
                    if !still_valid {
                        break;
                    }
                    if let Err(e) = execute(action, &stop) {
                        eprintln!("[automation:{}] Action failed: {e}", rule.id);
                    }
                }
            }
        }));
        Ok(())
    }
    fn update(&mut self, state: &mut State) -> Result<()> {
        if let Err(e) = self.reload() {
            eprintln!("[automation] Retaining last valid rules: {e}");
        }
        state.automation.enabled = true;
        state.automation.rules_count = self
            .rules
            .lock()
            .map_err(|_| anyhow::anyhow!("Rules unavailable"))?
            .len();
        for rule in self.triggered(state)? {
            if let Some(queue) = &self.queue {
                if queue.try_send(rule).is_err() {
                    eprintln!("[automation] Queue full; trigger dropped until condition rearms");
                }
            }
        }
        Ok(())
    }
    fn shutdown(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        self.queue.take();
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
        Ok(())
    }
}
impl Drop for AutomationModule {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edges_reload_and_invalid_updates() {
        let dir = std::env::temp_dir().join(format!("samos-rules-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("rules.json");
        let content = r#"[{"id":"cpu","name":"CPU","enabled":true,"conditions":[{"field":"cpu.usage","operator":"greater_than","value":50}],"actions":[{"type":"log","message":"high"}]}]"#;
        std::fs::write(&path, content).unwrap();
        let mut module = AutomationModule::at(path.clone());
        module.reload().unwrap();
        let mut state = State::default();
        state.cpu.usage = 80.;
        assert_eq!(module.triggered(&state).unwrap().len(), 1);
        state.cpu.usage = 90.;
        assert!(module.triggered(&state).unwrap().is_empty());
        state.cpu.usage = 10.;
        module.triggered(&state).unwrap();
        state.cpu.usage = 80.;
        assert_eq!(module.triggered(&state).unwrap().len(), 1);
        std::fs::write(&path, "invalid").unwrap();
        assert!(module.reload().is_err());
        assert_eq!(module.rules.lock().unwrap().len(), 1);
        std::fs::remove_file(&path).unwrap();
        module.reload().unwrap();
        assert!(module.rules.lock().unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
