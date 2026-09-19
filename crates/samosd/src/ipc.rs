use anyhow::{Result, ensure};
use samos_core::state::State;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub struct StateIpcServer {
    listener_handle: Option<thread::JoinHandle<()>>,
    socket_path: PathBuf,
    stop: Arc<AtomicBool>,
    identity: (u64, u64),
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

        Self::at(socket_path)
    }

    fn at(socket_path: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(socket_path.parent().unwrap())?;

        if socket_path.exists() {
            ensure!(
                UnixStream::connect(&socket_path).is_err(),
                "State socket has an active owner"
            );
            std::fs::remove_file(&socket_path)?;
        }

        let listener = UnixListener::bind(&socket_path)?;
        listener.set_nonblocking(true)?;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
        let meta = std::fs::metadata(&socket_path)?;
        let identity = (meta.dev(), meta.ino());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();

        let latest_state = Arc::new(Mutex::new(None::<State>));
        let latest_state_clone = latest_state.clone();

        let handle = thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
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
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(e) => {
                        thread::sleep(Duration::from_millis(25));
                        eprintln!("[ipc] Socket accept error: {}", e);
                    }
                }
            }
        });

        Ok(Self {
            listener_handle: Some(handle),
            socket_path,
            stop,
            identity,
            latest_state,
        })
    }

    pub fn update_state(&self, state: &State) {
        *self.latest_state.lock().unwrap() = Some(state.clone());
    }

    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.listener_handle.take() {
            let _ = handle.join();
        }
        if std::fs::metadata(&self.socket_path).is_ok_and(|m| (m.dev(), m.ino()) == self.identity) {
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }
}

impl Drop for StateIpcServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shutdown_without_clients_and_active_owner_protection() {
        let dir = std::env::temp_dir().join(format!("samos-ipc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.sock");
        let mut server = StateIpcServer::at(path.clone()).unwrap();
        assert!(StateIpcServer::at(path.clone()).is_err());
        let started = std::time::Instant::now();
        server.shutdown();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(!path.exists());
        server.shutdown();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
