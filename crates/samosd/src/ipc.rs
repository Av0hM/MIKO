use anyhow::{Result, ensure};
use samos_core::state::State;
use std::io::Write;
use std::os::unix::{fs::PermissionsExt, net::{UnixListener,UnixStream}};
use std::path::PathBuf;
use std::sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}};
use std::thread;
use std::time::Duration;

pub struct StateIpcServer {
    listener_handle: Option<thread::JoinHandle<()>>,
    socket_path: PathBuf,
    latest_state: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
}
impl StateIpcServer {
    pub fn new()->Result<Self>{
        let socket_path=PathBuf::from(std::env::var("HOME")?).join(".local/state/samos/state.sock");
        std::fs::create_dir_all(socket_path.parent().expect("socket path has a parent"))?;
        if socket_path.exists(){
            ensure!(UnixStream::connect(&socket_path).is_err(),"Another samosd is already serving state");
            std::fs::remove_file(&socket_path)?;
        }
        let listener=UnixListener::bind(&socket_path)?;
        listener.set_nonblocking(true)?;
        std::fs::set_permissions(&socket_path,std::fs::Permissions::from_mode(0o600))?;
        let latest_state=Arc::new(Mutex::new(serde_json::to_string(&State::default())?));
        let cache=latest_state.clone();let stop=Arc::new(AtomicBool::new(false));let stopping=stop.clone();
        let handle=thread::spawn(move ||{
            while !stopping.load(Ordering::Relaxed){
                match listener.accept(){
                    Ok((mut stream,_))=>{
                        let _=stream.set_write_timeout(Some(Duration::from_secs(1)));
                        if let Ok(json)=cache.lock(){let _=writeln!(stream,"{json}");}
                    },
                    Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>thread::sleep(Duration::from_millis(40)),
                    Err(e)=>{eprintln!("[ipc] {e}");thread::sleep(Duration::from_millis(100));}
                }
            }
        });
        Ok(Self{listener_handle:Some(handle),socket_path,latest_state,stop})
    }
    pub fn update_state(&self,state:&State){
        if let (Ok(json),Ok(mut cache))=(serde_json::to_string(state),self.latest_state.lock()){*cache=json;}
    }
    pub fn shutdown(&mut self){
        self.stop.store(true,Ordering::Relaxed);
        if let Some(handle)=self.listener_handle.take(){let _=handle.join();let _=std::fs::remove_file(&self.socket_path);}
    }
}
impl Drop for StateIpcServer {fn drop(&mut self){self.shutdown();}}
