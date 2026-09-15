//! Low-frequency spectrum snapshot and media metadata; frontend audio animation is separate.
use anyhow::Result;
use std::io::{BufRead,BufReader};
use std::process::{Child,Command,Stdio};
use std::sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}};
use std::thread;
use std::time::Duration;
use crate::{modules::Module,state::State};

pub struct VisualizerModule {
    spectrum:Arc<Mutex<Vec<u8>>>,
    child:Arc<Mutex<Option<Child>>>,
    stop:Arc<AtomicBool>,
    handle:Option<thread::JoinHandle<()>>,
}
impl VisualizerModule {
    /// Construct a spectrum collector with supervised cava and bounded metadata queries.
    pub fn new()->Result<Self>{Ok(Self{spectrum:Arc::new(Mutex::new(vec![0;120])),child:Arc::new(Mutex::new(None)),stop:Arc::new(AtomicBool::new(false)),handle:None})}
}
impl Module for VisualizerModule {
    fn name(&self)->&'static str{"visualizer"}
    fn init(&mut self)->Result<()> {
        let directory=std::path::PathBuf::from(std::env::var("HOME")?).join(".local/state/samos");std::fs::create_dir_all(&directory)?;
        let config=directory.join("cava-backend.conf");
        std::fs::write(&config,"[general]\nframerate=10\nbars=120\n[input]\nmethod=pulse\nsource=auto\n[output]\nmethod=raw\nraw_target=/dev/stdout\ndata_format=ascii\nascii_max_range=100\n")?;
        let spectrum=self.spectrum.clone();let child=self.child.clone();let stop=self.stop.clone();
        self.handle=Some(thread::spawn(move ||{
            while !stop.load(Ordering::Relaxed){
                match Command::new("cava").args(["-p",&config.to_string_lossy()]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn(){
                    Ok(mut process)=>{
                        let stdout=process.stdout.take();
                        if let Ok(mut current)=child.lock(){*current=Some(process);}
                        if stop.load(Ordering::Relaxed) {
                            if let Ok(mut current)=child.lock(){if let Some(mut process)=current.take(){let _=process.kill();let _=process.wait();}}
                            break;
                        }
                        if let Some(stdout)=stdout {
                            for line in BufReader::new(stdout).lines(){
                                if stop.load(Ordering::Relaxed){break;}
                                let Ok(line)=line else{break;};
                                let values:Vec<u8>=line.split(';').filter_map(|v|v.parse::<u8>().ok()).map(|n|n.min(100)).collect();
                                if values.len()==120 {if let Ok(mut output)=spectrum.lock(){*output=values;}}
                            }
                        }
                        if let Ok(mut current)=child.lock(){if let Some(mut process)=current.take(){let _=process.kill();let _=process.wait();}}
                    },
                    Err(e)=>eprintln!("[visualizer] cava unavailable: {e}"),
                }
                if let Ok(mut output)=spectrum.lock(){output.fill(0);}
                for _ in 0..30 {if stop.load(Ordering::Relaxed){break;}thread::sleep(Duration::from_millis(100));}
            }
        }));Ok(())
    }
    fn update(&mut self,state:&mut State)->Result<()> {
        state.visualizer.spectrum=self.spectrum.lock().map(|s|s.clone()).unwrap_or_else(|_|vec![0;120]);
        let metadata=crate::process::run("playerctl",&["metadata","--format","{{title}}\n{{artist}}\n{{status}}"],None,2).unwrap_or_default();
        let mut lines=metadata.lines();state.visualizer.now_playing_title=lines.next().unwrap_or_default().into();state.visualizer.now_playing_artist=lines.next().unwrap_or_default().into();state.visualizer.now_playing_status=lines.next().unwrap_or("Stopped").into();
        Ok(())
    }
    fn shutdown(&mut self)->Result<()> {
        self.stop.store(true,Ordering::Relaxed);
        if let Ok(mut current)=self.child.lock(){if let Some(mut process)=current.take(){let _=process.kill();let _=process.wait();}}
        if let Some(handle)=self.handle.take(){let _=handle.join();}Ok(())
    }
}
impl Drop for VisualizerModule{fn drop(&mut self){let _=self.shutdown();}}
