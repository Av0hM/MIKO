//! Request-driven local assistant. IPC never executes shell text from a model.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::thread;
use std::time::{Duration, Instant};
use crate::{modules::Module, state::State};

const MODEL: &str = "qwen2.5:7b";
const MAX_TURNS: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    #[serde(default)] content: String,
    #[serde(skip_serializing_if="Option::is_none")] tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if="Option::is_none")] tool_name: Option<String>,
}
impl Message {
    fn new(role: &str, content: impl Into<String>) -> Self {
        Self { role: role.into(), content: content.into(), tool_calls: None, tool_name: None }
    }
    fn result(call: &ToolCall, result: String) -> Self {
        Self { tool_name: Some(call.function.name.clone()), ..Self::new("tool", result) }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ToolCall {
    #[serde(default)] id: String,
    function: Function,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Function { name: String, #[serde(default)] arguments: Value }
#[derive(Deserialize)]
#[serde(tag="type")]
enum Request {
    Chat { message: String, conversation_id: Option<String> },
    ConfirmTool { tool_call_id: String, confirmed: bool, conversation_id: Option<String> },
    Voice { conversation_id: Option<String> },
}
struct Pending { call: ToolCall, remaining: Vec<ToolCall>, created: Instant }
#[derive(Default)]
struct Session { messages: Vec<Message>, pending: Option<Pending>, touched: Option<Instant> }
struct Assistant {
    client: reqwest::Client,
    db: Mutex<rusqlite::Connection>,
    sessions: Mutex<HashMap<String, Arc<Mutex<Session>>>>,
    home: PathBuf,
    url: String,
    busy: AtomicUsize,
    health: Mutex<String>,
}
impl Assistant {
    fn new(home: PathBuf, url: String) -> Result<Self> {
        let directory=home.join(".local/state/samos"); std::fs::create_dir_all(&directory)?;
        let db=rusqlite::Connection::open(directory.join("ai.db"))?;
        db.busy_timeout(Duration::from_secs(2))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS reminders (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL, due_at INTEGER, created_at INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS conversation_summaries (conversation_id TEXT PRIMARY KEY, summary TEXT NOT NULL, updated_at INTEGER NOT NULL);")?;
        Ok(Self { client: reqwest::Client::builder().connect_timeout(Duration::from_secs(3)).timeout(Duration::from_secs(90)).build()?, db: Mutex::new(db), sessions: Mutex::new(HashMap::new()), home, url, busy: AtomicUsize::new(0), health: Mutex::new("idle".into()) })
    }
    fn live_state(&self) -> Result<State> {
        let path=self.home.join(".local/state/samos/state.json");
        ensure!(path.metadata()?.modified()?.elapsed()?.as_secs()<15, "System metrics are stale");
        for _ in 0..3 {
            if let Ok(state)=serde_json::from_slice(&std::fs::read(&path)?) { return Ok(state); }
            thread::sleep(Duration::from_millis(15));
        }
        bail!("System metrics are being refreshed; please retry")
    }
    fn session(&self, id: &str) -> Result<Arc<Mutex<Session>>> {
        ensure!(!id.is_empty() && id.len()<=128, "Invalid conversation ID");
        let mut sessions=self.sessions.lock().map_err(|_|anyhow::anyhow!("Session unavailable"))?;
        if !sessions.contains_key(id) && sessions.len()>=64 {
            sessions.retain(|_, session| session.try_lock().map_or(true, |s| s.touched.is_some_and(|t| t.elapsed()<Duration::from_secs(3600))));
            ensure!(sessions.len()<64, "Too many conversations; retry later");
        }
        Ok(sessions.entry(id.into()).or_default().clone())
    }
    async fn ask(&self, messages: &[Message], tools: bool) -> Result<Message> {
        let request=json!({"model":MODEL,"messages":messages,"stream":false,"tools":if tools {tool_definitions()} else {Vec::new()},"options":{"num_predict":1024,"num_ctx":4096}});
        let response=self.client.post(format!("{}/api/chat",self.url)).json(&request).send().await.context("Cannot reach local Ollama")?;
        ensure!(response.status().is_success(),"Ollama returned {}. Check that {MODEL} is installed.",response.status());
        let body:Value=response.json().await?;
        Ok(serde_json::from_value(body.get("message").context("Missing Ollama message")?.clone())?)
    }
    async fn handle(&self, request: Request) -> Result<Value> {
        let id=match &request {
            Request::Chat{conversation_id,..}|Request::Voice{conversation_id}|Request::ConfirmTool{conversation_id,..} => conversation_id.clone().unwrap_or_else(||uuid::Uuid::new_v4().to_string()),
        };
        let cell=self.session(&id)?;
        let mut session=cell.try_lock().map_err(|_|anyhow::anyhow!("This conversation is busy"))?;
        session.touched=Some(Instant::now());
        match request {
            Request::Chat{message,..} => {
                ensure!(session.pending.is_none(),"Confirm or deny the pending action first");
                ensure!(!message.trim().is_empty() && message.len()<=4096,"Message must contain 1–4096 bytes");
                self.begin(&id,&mut session)?;
                session.messages.push(Message::new("user",message));
            }
            Request::Voice{..} => {
                ensure!(session.pending.is_none(),"Confirm or deny the pending action first");
                let text=self.record(5,"en")?;
                self.begin(&id,&mut session)?;
                session.messages.push(Message::new("user",text));
            }
            Request::ConfirmTool{tool_call_id,confirmed,..} => {
                let pending=session.pending.as_ref().context("No pending action in this conversation")?;
                ensure!(pending.call.id==tool_call_id,"Confirmation does not match this conversation");
                let pending=session.pending.take().context("Pending action disappeared")?;
                if pending.created.elapsed()>Duration::from_secs(300) {
                    session.messages.push(Message::result(&pending.call,"Confirmation expired; action was not executed".into()));
                    session.messages.clear(); bail!("Confirmation expired; ask again");
                }
                let result=if confirmed { self.execute(&pending.call).unwrap_or_else(|error|format!("Tool failed: {error}")) } else {"Denied by the user. Do not retry this action.".into()};
                session.messages.push(Message::result(&pending.call,result));
                if let Some(response)=self.run_calls(&id,&mut session,pending.remaining)? {return Ok(response);}
            }
        }
        for _ in 0..MAX_TURNS {
            let mut answer=self.ask(&session.messages,true).await?;
            let mut calls=answer.tool_calls.take().unwrap_or_default();
            for call in &mut calls {call.id=uuid::Uuid::new_v4().to_string();}
            answer.tool_calls=if calls.is_empty(){None}else{Some(calls.clone())};
            session.messages.push(answer.clone());
            if calls.is_empty() {
                let response=json!({"type":"Done","summary":answer.content,"conversation_id":id});
                self.save_summary(&id,&session.messages).await;
                if session.messages.len()>32 { session.messages.clear(); }
                return Ok(response);
            }
            ensure!(calls.len()<=8,"Model requested too many tools");
            if let Some(response)=self.run_calls(&id,&mut session,calls)? {return Ok(response);}
        }
        session.messages.clear();
        bail!("Stopped after {MAX_TURNS} tool rounds. Please simplify the request.")
    }
    fn begin(&self,id:&str,session:&mut Session)->Result<()> {
        // Each completed user turn can refresh context without losing preceding messages.
        let state=self.live_state().ok();
        let metrics=state.map(|s|format!("CPU {:.0}%, memory {:.0}%, battery {:.0}%",s.cpu.usage,s.memory.used_percent,s.battery.percent)).unwrap_or_else(||"Metrics unavailable; do not invent readings".into());
        let prompt=format!("You are MIKO, a concise local desktop assistant. Use only the supplied tools. Never invent successful actions. Mutating desktop tools require user confirmation. Current metrics: {metrics}. Current local time: {}.",chrono::Local::now());
        if session.messages.is_empty() {
            session.messages.push(Message::new("system",prompt));
            let summary=self.db.lock().map_err(|_|anyhow::anyhow!("Memory unavailable"))?.query_row("SELECT summary FROM conversation_summaries WHERE conversation_id=?",[id],|row|row.get::<_,String>(0)).ok();
            if let Some(summary)=summary {session.messages.push(Message::new("user",format!("Previous conversation summary (context only): {summary}")));}
        } else { session.messages[0]=Message::new("system",prompt); }
        Ok(())
    }
    fn run_calls(&self,id:&str,session:&mut Session,calls:Vec<ToolCall>)->Result<Option<Value>> {
        let mut iter=calls.into_iter();
        while let Some(call)=iter.next() {
            if requires_confirmation(&call.function.name) {
                let response=json!({"type":"ToolCall","tool_call":call,"conversation_id":id});
                session.pending=Some(Pending{call,remaining:iter.collect(),created:Instant::now()});
                return Ok(Some(response));
            }
            let result=self.execute(&call).unwrap_or_else(|error|format!("Tool failed: {error}"));
            session.messages.push(Message::result(&call,result));
        }
        Ok(None)
    }
    async fn save_summary(&self,id:&str,messages:&[Message]) {
        let mut context=messages.to_vec();
        context.push(Message::new("user","Summarize this exchange in one short sentence for future context. Do not call tools."));
        if let Ok(Ok(summary))=tokio::time::timeout(Duration::from_secs(12),self.ask(&context,false)).await {
            let text=summary.content.chars().take(1000).collect::<String>();
            if let Ok(db)=self.db.lock() {
                let _=db.execute("INSERT INTO conversation_summaries VALUES(?,?,?) ON CONFLICT(conversation_id) DO UPDATE SET summary=excluded.summary,updated_at=excluded.updated_at",rusqlite::params![id,text,chrono::Utc::now().timestamp()]);
                let _=db.execute("DELETE FROM conversation_summaries WHERE conversation_id NOT IN (SELECT conversation_id FROM conversation_summaries ORDER BY updated_at DESC LIMIT 128)",[]);
            }
        }
    }
    fn execute(&self,call:&ToolCall)->Result<String> {
        let args=&call.function.arguments;
        match call.function.name.as_str() {
            "get_system_state"=>Ok(serde_json::to_string(&self.live_state()?)?),
            "launch_app"=>{
                let app=text_arg(args,"app_name")?;
                ensure!(app.len()<=200 && app.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b)) && !app.starts_with('-'),"Use a desktop application ID, not a command");
                crate::process::run("gtk-launch",&[app],None,8)?; Ok(format!("Launched {app}"))
            }
            "switch_workspace"=>{
                let id=args["workspace_id"].as_i64().context("workspace_id must be an integer")?;
                ensure!((1..=1000).contains(&id),"Workspace must be 1–1000");
                crate::process::run("hyprctl",&["dispatch","workspace",&id.to_string()],None,5)
            }
            "set_theme"=>{
                let name=text_arg(args,"theme")?; crate::config::validate_name(name)?;
                ensure!(self.home.join(".config/samos/themes").join(format!("{name}.toml")).is_file(),"Theme not found");
                crate::config::set_theme_at(&self.home.join(".config/samos/config.toml"),name)?;
                Ok(format!("Theme set to {name}"))
            }
            "add_reminder"=>{
                let text=text_arg(args,"text")?; ensure!(text.len()<=2000,"Reminder too long");
                let due=if args["due_at"].is_null(){None}else{Some(args["due_at"].as_i64().context("due_at must be a Unix timestamp")?)};
                self.db.lock().map_err(|_|anyhow::anyhow!("Memory unavailable"))?.execute("INSERT INTO reminders(text,due_at,created_at) VALUES(?,?,?)",rusqlite::params![text,due,chrono::Utc::now().timestamp()])?;
                Ok(format!("Saved reminder: {text}"))
            }
            "list_reminders"=>{
                let db=self.db.lock().map_err(|_|anyhow::anyhow!("Memory unavailable"))?;
                let mut stmt=db.prepare("SELECT id,text,due_at FROM reminders ORDER BY created_at DESC LIMIT 100")?;
                let rows=stmt.query_map([],|row|Ok(json!({"id":row.get::<_,i64>(0)?,"text":row.get::<_,String>(1)?,"due_at":row.get::<_,Option<i64>>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
                Ok(serde_json::to_string(&rows)?)
            }
            "stt_transcribe"=>self.transcribe(Path::new(text_arg(args,"audio_file")?),args["language"].as_str().unwrap_or("en")),
            "stt_record_and_transcribe"=>self.record(args["duration_seconds"].as_u64().unwrap_or(5),args["language"].as_str().unwrap_or("en")),
            "tts_speak"=>self.speak(text_arg(args,"text")?,args["voice"].as_str().unwrap_or("en_US-lessac-medium")),
            other=>bail!("Unknown tool: {other}"),
        }
    }
    fn binary(&self,name:&str)->String {
        let path=self.home.join(".local/bin").join(name);
        if path.is_file(){path.to_string_lossy().into_owned()}else{name.into()}
    }
    fn transcribe(&self,path:&Path,language:&str)->Result<String> {
        ensure!(path.is_file(),"Audio file not found");
        ensure!(language.len()<=8&&language.bytes().all(|b|b.is_ascii_alphabetic()||b==b'-'),"Invalid language");
        let model=self.home.join(".local/share/whisper.cpp/models/ggml-base.bin");
        ensure!(model.is_file(),"Whisper model is not installed");
        crate::process::run(&self.binary("whisper-cli"),&["-m",&model.to_string_lossy(),"-f",&path.to_string_lossy(),"-l",language,"-nt"],None,60)
    }
    fn record(&self,duration:u64,language:&str)->Result<String> {
        ensure!((1..=30).contains(&duration),"Recording duration must be 1–30 seconds");
        let temp=AudioTemp::new()?;
        crate::process::run("arecord",&["-q","-f","S16_LE","-c","1","-r","16000","-d",&duration.to_string(),&temp.file.to_string_lossy()],None,duration+5)?;
        self.transcribe(&temp.file,language)
    }
    fn speak(&self,text:&str,voice:&str)->Result<String> {
        ensure!(text.len()<=4000,"Speech text too long"); crate::config::validate_name(voice)?;
        let model=self.home.join(".local/share/piper/voices").join(format!("{voice}.onnx"));
        ensure!(model.is_file(),"Piper voice is not installed");
        let temp=AudioTemp::new()?;
        let env_espeak=format!("ESPEAK_DATA_PATH={}",self.home.join(".local/share/espeak-ng-data").display());
        let env_lib=format!("LD_LIBRARY_PATH={}:{}",self.home.join(".local/lib").display(),std::env::var("LD_LIBRARY_PATH").unwrap_or_default());
        crate::process::run("env",&[&env_espeak,&env_lib,&self.binary("piper"),"--model",&model.to_string_lossy(),"--output_file",&temp.file.to_string_lossy()],Some(text),60)?;
        crate::process::run("aplay",&["-q",&temp.file.to_string_lossy()],None,90)?;
        Ok("Speech played".into())
    }
}
struct AudioTemp { directory:PathBuf, file:PathBuf }
impl AudioTemp {
    fn new()->Result<Self>{
        let directory=std::env::temp_dir().join(format!("samos-audio-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory)?;std::fs::set_permissions(&directory,std::fs::Permissions::from_mode(0o700))?;
        Ok(Self{file:directory.join("audio.wav"),directory})
    }
}
impl Drop for AudioTemp {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.directory);}}
fn text_arg<'a>(args:&'a Value,key:&str)->Result<&'a str>{
    args[key].as_str().filter(|s|!s.trim().is_empty()).with_context(||format!("{key} must be a nonempty string"))
}
fn requires_confirmation(name:&str)->bool {
    matches!(name,"launch_app"|"switch_workspace"|"set_theme"|"stt_record_and_transcribe"|"tts_speak")
}
fn tool_definitions()->Vec<Value>{
    let definitions=[
        ("get_system_state","Read live system metrics",json!({}),vec![]),
        ("launch_app","Launch a desktop application ID; requires confirmation",json!({"app_name":{"type":"string"}}),vec!["app_name"]),
        ("switch_workspace","Switch workspace; requires confirmation",json!({"workspace_id":{"type":"integer"}}),vec!["workspace_id"]),
        ("set_theme","Set a theme; requires confirmation",json!({"theme":{"type":"string"}}),vec!["theme"]),
        ("add_reminder","Save a reminder with an optional Unix due timestamp",json!({"text":{"type":"string"},"due_at":{"type":"integer"}}),vec!["text"]),
        ("list_reminders","List saved reminders",json!({}),vec![]),
        ("stt_transcribe","Transcribe a local audio file",json!({"audio_file":{"type":"string"},"language":{"type":"string"}}),vec!["audio_file"]),
        ("stt_record_and_transcribe","Record 1–30 seconds from the microphone; requires confirmation",json!({"duration_seconds":{"type":"integer"},"language":{"type":"string"}}),vec![]),
        ("tts_speak","Speak text locally; requires confirmation",json!({"text":{"type":"string"},"voice":{"type":"string"}}),vec!["text"]),
    ];
    definitions.into_iter().map(|(name,description,properties,required)|json!({"type":"function","function":{"name":name,"description":description,"parameters":{"type":"object","properties":properties,"required":required}}})).collect()
}

/// Local AI socket module. Tool operations are handled outside metric polling.
pub struct AiModule { assistant:Arc<Assistant>, stop:Arc<AtomicBool>, handle:Option<thread::JoinHandle<()>>, socket:PathBuf }
impl AiModule {
    /// Open persistent local memory; inference remains request driven.
    pub fn new()->Result<Self>{
        let home=PathBuf::from(std::env::var("HOME")?);
        let socket=home.join(".local/state/samos/ai.sock");
        Ok(Self{assistant:Arc::new(Assistant::new(home,"http://localhost:11434".into())?),stop:Arc::new(AtomicBool::new(false)),handle:None,socket})
    }
}
impl Module for AiModule {
    fn name(&self)->&'static str{"ai"}
    fn init(&mut self)->Result<()>{
        if self.socket.exists(){
            ensure!(UnixStream::connect(&self.socket).is_err(),"AI socket already has an active server");
            std::fs::remove_file(&self.socket)?;
        }
        let listener=UnixListener::bind(&self.socket)?;
        std::fs::set_permissions(&self.socket,std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let assistant=self.assistant.clone();let stop=self.stop.clone();
        self.handle=Some(thread::spawn(move || {
            while !stop.load(Ordering::Relaxed){
                match listener.accept(){
                    Ok((mut stream,_))=>{
                        if assistant.busy.fetch_add(1,Ordering::SeqCst)>=4 {assistant.busy.fetch_sub(1,Ordering::SeqCst);let _=stream.write_all(b"{\"type\":\"Error\",\"message\":\"Assistant busy; retry shortly\"}\n");continue;}
                        let assistant=assistant.clone();
                        thread::spawn(move || {
                            let result=(||->Result<Value>{
                                stream.set_read_timeout(Some(Duration::from_secs(5)))?;stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                                let mut line=String::new();BufReader::new((&stream).take(16_385)).read_line(&mut line)?;
                                ensure!(line.len()<=16_384 && line.ends_with('\n'),"Request must be newline terminated and at most 16 KiB");
                                let request=serde_json::from_str(&line)?;
                                let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build()?;
                                runtime.block_on(async {tokio::time::timeout(Duration::from_secs(150),assistant.handle(request)).await.context("Assistant request timed out")?})
                            })();
                            if let Ok(mut health)=assistant.health.lock(){*health=if result.is_ok(){"idle"}else{"error"}.into();}
                            let response=result.unwrap_or_else(|e|json!({"type":"Error","message":e.to_string()}));
                            let _=writeln!(stream,"{response}");assistant.busy.fetch_sub(1,Ordering::SeqCst);
                        });
                    }
                    Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>thread::sleep(Duration::from_millis(50)),
                    Err(error)=>{eprintln!("[ai] Listener error: {error}");thread::sleep(Duration::from_millis(100));}
                }
            }
        }));Ok(())
    }
    fn update(&mut self,state:&mut State)->Result<()>{
        state.ai.model=MODEL.into();state.ai.status=if self.assistant.busy.load(Ordering::Relaxed)>0{"busy".into()}else{self.assistant.health.lock().map(|s|s.clone()).unwrap_or_else(|_|"error".into())};Ok(())
    }
    fn shutdown(&mut self)->Result<()>{
        self.stop.store(true,Ordering::Relaxed);if let Some(handle)=self.handle.take(){let _=handle.join();}let _=std::fs::remove_file(&self.socket);Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn native_tool_call_needs_no_id(){let call:ToolCall=serde_json::from_value(json!({"function":{"name":"get_system_state","arguments":{}}})).unwrap();assert!(call.id.is_empty());}
    #[test] fn sensitive_actions_are_gated(){for name in ["launch_app","switch_workspace","set_theme","tts_speak","stt_record_and_transcribe"]{assert!(requires_confirmation(name));}assert!(!requires_confirmation("get_system_state"));}
    #[test] fn voice_tools_are_registered(){let tools=tool_definitions();assert_eq!(tools.len(),9);assert!(tools.iter().any(|t|t["function"]["name"]=="stt_transcribe"));}

    #[test] fn confirmation_preserves_tool_order_and_exact_arguments(){
        let home=std::env::temp_dir().join(format!("samos-ai-test-{}",uuid::Uuid::new_v4()));
        let assistant=Assistant::new(home.clone(),"http://localhost:1".into()).unwrap();
        let mut session=Session::default();
        session.messages.push(Message::new("user","Launch Firefox and switch workspace"));
        let calls=vec![ToolCall{id:"first".into(),function:Function{name:"launch_app".into(),arguments:json!({"app_name":"firefox"})}},ToolCall{id:"second".into(),function:Function{name:"switch_workspace".into(),arguments:json!({"workspace_id":2})}}];
        let response=assistant.run_calls("test",&mut session,calls).unwrap().unwrap();
        assert_eq!(response["tool_call"]["function"]["arguments"]["app_name"],"firefox");
        assert_eq!(session.pending.as_ref().unwrap().remaining[0].id,"second");
        assert_eq!(session.messages[0].content,"Launch Firefox and switch workspace");
        drop(assistant);std::fs::remove_dir_all(home).unwrap();
    }
    #[test] fn launch_rejects_shell_fragments_before_execution(){
        let home=std::env::temp_dir().join(format!("samos-ai-test-{}",uuid::Uuid::new_v4()));
        let assistant=Assistant::new(home.clone(),"http://localhost:1".into()).unwrap();
        let call=ToolCall{id:String::new(),function:Function{name:"launch_app".into(),arguments:json!({"app_name":"firefox; touch /tmp/not-executed"})}};
        assert!(assistant.execute(&call).unwrap_err().to_string().contains("application ID"));
        drop(assistant);std::fs::remove_dir_all(home).unwrap();
    }
    #[test] fn system_tool_reads_exported_metrics(){
        let home=std::env::temp_dir().join(format!("samos-ai-test-{}",uuid::Uuid::new_v4()));
        let assistant=Assistant::new(home.clone(),"http://localhost:1".into()).unwrap();
        let mut state=State::default();state.cpu.usage=37.5;
        std::fs::write(home.join(".local/state/samos/state.json"),serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(assistant.live_state().unwrap().cpu.usage,37.5);
        drop(assistant);std::fs::remove_dir_all(home).unwrap();
    }
}
