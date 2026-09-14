use anyhow::Result;
use samos_core::state::State;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::io::Write;

pub struct StateIpcServer {
    listener_handle: Option<thread::JoinHandle<()>>,
    socket_path: PathBuf,
    latest_state: Arc<Mutex<Option<State>>>,
}

impl StateIpcServer {
    pub fn new() -> Result<Self> {
        let home = std::env::var("HOME")?;
        let socket_path = PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("samos")
            .join("state.sock");

        std::fs::create_dir_all(socket_path.parent().unwrap())?;

        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }

        let listener = UnixListener::bind(&socket_path)?;

        let latest_state = Arc::new(Mutex::new(None::<State>));
        let latest_state_clone = latest_state.clone();

        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        let state = latest_state_clone.lock().unwrap().clone();
                        if let Some(state) = state {
                            let json = match serde_json::to_string(&state) {
                                Ok(j) => j,
                                Err(_) => continue,
                            };
                            let _ = stream.write_all(format!("{}\n", json).as_bytes());
                            let _ = stream.flush();
                        }
                    }
                    Err(e) => {
                        eprintln!("[ipc] Socket accept error: {}", e);
                    }
                }
            }
        });

        Ok(Self {
            listener_handle: Some(handle),
            socket_path,
            latest_state,
        })
    }

    pub fn update_state(&self, state: &State) {
        *self.latest_state.lock().unwrap() = Some(state.clone());
    }

    pub fn shutdown(&mut self) {
        if let Some(handle) = self.listener_handle.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}