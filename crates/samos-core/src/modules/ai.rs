//! Request-driven local assistant. IPC never executes shell text from a model.
use crate::{modules::Module, state::State};
use anyhow::{Context, Result, bail, ensure};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

const MODEL: &str = "qwen2.5:1.5b";
const MAX_TURNS: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    #[serde(default)]
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
}
impl Message {
    fn new(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
            tool_name: None,
        }
    }
    fn result(call: &ToolCall, result: String) -> Self {
        Self {
            tool_name: Some(call.function.name.clone()),
            ..Self::new("tool", result)
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ToolCall {
    #[serde(default)]
    id: String,
    function: Function,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Function {
    name: String,
    #[serde(default)]
    arguments: Value,
}
#[derive(Deserialize)]
#[serde(tag = "type")]
enum Request {
    Chat {
        message: String,
        conversation_id: Option<String>,
    },
    ConfirmTool {
        tool_call_id: String,
        confirmed: bool,
        conversation_id: Option<String>,
    },
    Voice {
        conversation_id: Option<String>,
        speak_reply: Option<bool>,
    },
    History {
        conversation_id: Option<String>,
    },
    ForgetHistory {
        conversation_id: Option<String>,
        confirmed: bool,
    },
}
struct Pending {
    call: ToolCall,
    remaining: Vec<ToolCall>,
    created: Instant,
}
#[derive(Default)]
struct Session {
    messages: Vec<Message>,
    pending: Option<Pending>,
    touched: Option<Instant>,
    speak_reply: bool,
}
struct Assistant {
    client: reqwest::Client,
    db: Mutex<rusqlite::Connection>,
    sessions: Mutex<HashMap<String, Arc<Mutex<Session>>>>,
    home: PathBuf,
    url: String,
    model: String,
    profile: Mutex<crate::ai_profile::ProfileCache>,
    busy: AtomicUsize,
    audio: Arc<crate::audio::AudioCoordinator>,
    cancel: AtomicBool,
    health: Mutex<String>,
}
impl Assistant {
    fn new(home: PathBuf, url: String) -> Result<Self> {
        let directory = home.join(".local/state/samos");
        std::fs::create_dir_all(&directory)?;
        let mut db = rusqlite::Connection::open(directory.join("ai.db"))?;
        std::fs::set_permissions(
            directory.join("ai.db"),
            std::fs::Permissions::from_mode(0o600),
        )?;
        db.busy_timeout(Duration::from_secs(2))?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS reminders (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT NOT NULL, due_at INTEGER, created_at INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS conversation_summaries (conversation_id TEXT PRIMARY KEY, summary TEXT NOT NULL, updated_at INTEGER NOT NULL);")?;
        let migration = db.transaction()?;
        migration.execute_batch("CREATE TABLE IF NOT EXISTS ai_migrations (version INTEGER PRIMARY KEY); CREATE TABLE IF NOT EXISTS conversation_history (conversation_id TEXT PRIMARY KEY, messages TEXT NOT NULL, updated_at INTEGER NOT NULL); INSERT OR IGNORE INTO ai_migrations VALUES (1);")?;
        migration.commit()?;
        let model = std::env::var("SAMOS_MODEL").unwrap_or_else(|_| MODEL.into());
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(90))
                .build()?,
            db: Mutex::new(db),
            sessions: Mutex::new(HashMap::new()),
            home,
            url,
            model,
            profile: Mutex::new(crate::ai_profile::ProfileCache::default()),
            busy: AtomicUsize::new(0),
            audio: crate::audio::shared(),
            cancel: AtomicBool::new(false),
            health: Mutex::new("idle".into()),
        })
    }
    fn live_state(&self) -> Result<State> {
        let path = self.home.join(".local/state/samos/state.json");
        ensure!(
            path.metadata()?.modified()?.elapsed()?.as_secs() < 15,
            "System metrics are stale"
        );
        for _ in 0..3 {
            if let Ok(state) = serde_json::from_slice(&std::fs::read(&path)?) {
                return Ok(state);
            }
            thread::sleep(Duration::from_millis(15));
        }
        bail!("System metrics are being refreshed; please retry")
    }
    fn session(&self, id: &str) -> Result<Arc<Mutex<Session>>> {
        ensure!(!id.is_empty() && id.len() <= 128, "Invalid conversation ID");
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("Session unavailable"))?;
        if !sessions.contains_key(id) && sessions.len() >= 64 {
            sessions.retain(|_, session| {
                session.try_lock().map_or(true, |s| {
                    s.touched
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(3600))
                })
            });
            ensure!(sessions.len() < 64, "Too many conversations; retry later");
        }
        Ok(sessions.entry(id.into()).or_default().clone())
    }
    async fn ask(&self, messages: &[Message], tools: bool) -> Result<Message> {
        let request = json!({"model":self.model,"messages":messages,"stream":false,"tools":if tools {tool_definitions()} else {Vec::new()},"options":{"num_predict":1024,"num_ctx":4096}});
        let response = self
            .client
            .post(format!("{}/api/chat", self.url))
            .json(&request)
            .send()
            .await
            .context("Cannot reach local Ollama")?;
        ensure!(
            response.status().is_success(),
            "Ollama returned {}. Check that {} is installed.",
            response.status(),
            self.model
        );
        let body: Value = response.json().await?;
        Ok(serde_json::from_value(
            body.get("message")
                .context("Missing Ollama message")?
                .clone(),
        )?)
    }
    async fn handle(&self, request: Request) -> Result<Value> {
        let id = match &request {
            Request::Chat {
                conversation_id, ..
            }
            | Request::Voice {
                conversation_id, ..
            }
            | Request::History { conversation_id }
            | Request::ForgetHistory {
                conversation_id, ..
            }
            | Request::ConfirmTool {
                conversation_id, ..
            } => conversation_id.clone().unwrap_or_else(|| {
                if matches!(&request, Request::Voice { .. }) {
                    "miko".into()
                } else {
                    "terminal".into()
                }
            }),
        };
        let cell = self.session(&id)?;
        let mut session = cell
            .try_lock()
            .map_err(|_| anyhow::anyhow!("This conversation is busy"))?;
        session.touched = Some(Instant::now());
        match request {
            Request::History { .. } => {
                return Ok(
                    json!({"type":"History", "conversation_id":id, "messages":self.load_history(&id)?.unwrap_or_default()}),
                );
            }
            Request::ForgetHistory { confirmed, .. } => {
                ensure!(
                    confirmed,
                    "Explicit confirmation is required to forget this conversation"
                );
                ensure!(
                    session.pending.is_none(),
                    "Confirm or deny the pending action before forgetting history"
                );
                let mut db = self
                    .db
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Memory unavailable"))?;
                let tx = db.transaction()?;
                tx.execute(
                    "DELETE FROM conversation_history WHERE conversation_id=?",
                    [&id],
                )?;
                tx.execute(
                    "DELETE FROM conversation_summaries WHERE conversation_id=?",
                    [&id],
                )?;
                tx.commit()?;
                session.messages.clear();
                return Ok(
                    json!({"type":"Done", "conversation_id":id, "summary":"Conversation history forgotten; reminders and profile are unchanged."}),
                );
            }
            Request::Chat { message, .. } => {
                ensure!(
                    session.pending.is_none(),
                    "Confirm or deny the pending action first"
                );
                ensure!(
                    !message.trim().is_empty() && message.len() <= 4096,
                    "Message must contain 1–4096 bytes"
                );
                session.speak_reply = false;
                self.begin(&id, &mut session)?;
                session.messages.push(Message::new("user", message));
            }
            Request::Voice { speak_reply, .. } => {
                ensure!(
                    session.pending.is_none(),
                    "Confirm or deny the pending action first"
                );
                session.speak_reply = speak_reply.unwrap_or(false);
                let text = self.record(5, "en")?;
                self.begin(&id, &mut session)?;
                session.messages.push(Message::new("user", text));
            }
            Request::ConfirmTool {
                tool_call_id,
                confirmed,
                ..
            } => {
                let pending = session
                    .pending
                    .as_ref()
                    .context("No pending action in this conversation")?;
                ensure!(
                    pending.call.id == tool_call_id,
                    "Confirmation does not match this conversation"
                );
                let pending = session
                    .pending
                    .take()
                    .context("Pending action disappeared")?;
                if pending.created.elapsed() > Duration::from_secs(300) {
                    session.messages.push(Message::result(
                        &pending.call,
                        "Confirmation expired; action was not executed".into(),
                    ));
                    session.messages.clear();
                    bail!("Confirmation expired; ask again");
                }
                let result = if confirmed {
                    self.execute(&pending.call)
                        .unwrap_or_else(|error| format!("Tool failed: {error}"))
                } else {
                    "Denied by the user. Do not retry this action.".into()
                };
                session
                    .messages
                    .push(Message::result(&pending.call, result));
                if let Some(response) = self.run_calls(&id, &mut session, pending.remaining)? {
                    return Ok(response);
                }
            }
        }
        for _ in 0..MAX_TURNS {
            let mut answer = self.ask(&session.messages, true).await?;
            let mut calls = answer.tool_calls.take().unwrap_or_default();
            for call in &mut calls {
                call.id = uuid::Uuid::new_v4().to_string();
            }
            answer.tool_calls = if calls.is_empty() {
                None
            } else {
                Some(calls.clone())
            };
            session.messages.push(answer.clone());
            if calls.is_empty() {
                let mut response =
                    json!({"type":"Done","summary":answer.content,"conversation_id":id});
                self.save_history(&id, &session.messages)?;
                session.messages.clear();
                if std::mem::take(&mut session.speak_reply) {
                    if let Err(error) = self.speak(&answer.content, "en_US-lessac-medium") {
                        response["speech_error"] = json!(error.to_string());
                    }
                }
                return Ok(response);
            }
            ensure!(calls.len() <= 8, "Model requested too many tools");
            if let Some(response) = self.run_calls(&id, &mut session, calls)? {
                return Ok(response);
            }
        }
        session.messages.clear();
        bail!("Stopped after {MAX_TURNS} tool rounds. Please simplify the request.")
    }
    fn begin(&self, id: &str, session: &mut Session) -> Result<()> {
        let preferences = self
            .profile
            .lock()
            .map_err(|_| anyhow::anyhow!("Profile unavailable"))?
            .reload(&self.home.join(".config/samos/profile.toml"))
            .context()?;
        // Each completed user turn can refresh context without losing preceding messages.
        let state = self.live_state().ok();
        let metrics = state
            .map(|s| {
                format!(
                    "CPU {:.0}%, memory {:.0}%, battery {:.0}%",
                    s.cpu.usage, s.memory.used_percent, s.battery.percent
                )
            })
            .unwrap_or_else(|| "Metrics unavailable; do not invent readings".into());
        let prompt = format!(
            "You are MIKO, a local conversational desktop assistant. Answer casual conversation and brainstorming naturally; use tools when facts or actions require them. Use only the supplied tools. Never invent successful actions or unsupported capabilities. Mutating tools, including reminders, require user confirmation. For open_url, ask which browser (Firefox or Vivaldi) unless the user specified it. Do not assume a default browser. {preferences} Current metrics: {metrics}. Current local time: {}.",
            chrono::Local::now()
        );
        if session.messages.is_empty() {
            session.messages.push(Message::new("system", prompt));
            if let Some(history) = self.load_history(id)? {
                session.messages.extend(history);
            } else {
                let summary = self
                    .db
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Memory unavailable"))?
                    .query_row(
                        "SELECT summary FROM conversation_summaries WHERE conversation_id=?",
                        [id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                if let Some(summary) = summary {
                    session.messages.push(Message::new(
                        "user",
                        format!("Previous conversation summary (context only): {summary}"),
                    ));
                }
            }
        } else {
            session.messages[0] = Message::new("system", prompt);
        }
        Ok(())
    }
    fn run_calls(
        &self,
        id: &str,
        session: &mut Session,
        calls: Vec<ToolCall>,
    ) -> Result<Option<Value>> {
        let mut iter = calls.into_iter();
        while let Some(mut call) = iter.next() {
            if ["schedule_meeting", "cancel_meeting"].contains(&call.function.name.as_str()) {
                if let Err(error) = crate::calendar::preview(
                    &self.home,
                    &call.function.name,
                    &mut call.function.arguments,
                ) {
                    session
                        .messages
                        .push(Message::result(&call, format!("Tool failed: {error}")));
                    continue;
                }
            }
            if call.function.name == "open_url" {
                let prepared = (|| -> Result<()> {
                    let (_, url) = crate::desktop_tools::browser_url(
                        text_arg(&call.function.arguments, "browser")?,
                        text_arg(&call.function.arguments, "url")?,
                    )?;
                    call.function.arguments["url"] = json!(url);
                    Ok(())
                })();
                if let Err(error) = prepared {
                    session
                        .messages
                        .push(Message::result(&call, format!("Tool failed: {error}")));
                    continue;
                }
            }
            if call.function.name == "open_file" {
                let prepared = (|| -> Result<()> {
                    let (path, identity) = crate::desktop_tools::openable_file(
                        &self.home,
                        text_arg(&call.function.arguments, "path")?,
                    )?;
                    call.function.arguments["path"] = json!(path);
                    call.function.arguments["_identity"] = json!(identity);
                    Ok(())
                })();
                if let Err(error) = prepared {
                    session
                        .messages
                        .push(Message::result(&call, format!("Tool failed: {error}")));
                    continue;
                }
            }
            if requires_confirmation(&call.function.name) {
                let response = json!({"type":"ToolCall","tool_call":call,"conversation_id":id});
                session.pending = Some(Pending {
                    call,
                    remaining: iter.collect(),
                    created: Instant::now(),
                });
                return Ok(Some(response));
            }
            let result = self
                .execute(&call)
                .unwrap_or_else(|error| format!("Tool failed: {error}"));
            session.messages.push(Message::result(&call, result));
        }
        Ok(None)
    }
    fn load_history(&self, id: &str) -> Result<Option<Vec<Message>>> {
        let encoded = self
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("Memory unavailable"))?
            .query_row(
                "SELECT messages FROM conversation_history WHERE conversation_id=?",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        encoded
            .map(|data| {
                ensure!(data.len() <= 65536, "Stored history exceeds its size limit");
                Ok(serde_json::from_str(&data)?)
            })
            .transpose()
    }
    fn save_history(&self, id: &str, messages: &[Message]) -> Result<()> {
        ensure!(
            messages
                .last()
                .is_some_and(|m| m.role == "assistant" && m.tool_calls.is_none()),
            "Only completed exchanges can be saved"
        );
        let mut history = messages
            .iter()
            .filter(|m| m.role != "system")
            .cloned()
            .collect::<Vec<_>>();
        // Trim only at user-turn boundaries: never orphan a tool result from its call.
        loop {
            let encoded = serde_json::to_string(&history)?;
            if history.len() <= 32 && encoded.len() <= 16384 {
                break;
            }
            if let Some(next) = history
                .iter()
                .enumerate()
                .skip(1)
                .find_map(|(i, m)| (m.role == "user").then_some(i))
            {
                history.drain(..next);
            } else {
                break;
            }
        }
        let encoded = serde_json::to_string(&history)?;
        ensure!(
            encoded.len() <= 65536,
            "Completed exchange is too large to retain; history was not changed"
        );
        let mut db = self
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("Memory unavailable"))?;
        let tx = db.transaction()?;
        tx.execute("INSERT INTO conversation_history VALUES(?,?,?) ON CONFLICT(conversation_id) DO UPDATE SET messages=excluded.messages,updated_at=excluded.updated_at", rusqlite::params![id,encoded,chrono::Utc::now().timestamp_millis()])?;
        tx.execute("DELETE FROM conversation_history WHERE conversation_id IN (SELECT conversation_id FROM conversation_history WHERE conversation_id<>? ORDER BY updated_at DESC,conversation_id LIMIT -1 OFFSET 127)", [id])?;
        tx.commit()?;
        Ok(())
    }
    fn execute(&self, call: &ToolCall) -> Result<String> {
        let args = &call.function.arguments;
        match call.function.name.as_str() {
            "list_schedule" => Ok(crate::calendar::list(&self.home, args)?.to_string()),
            "schedule_meeting" | "cancel_meeting" => {
                crate::calendar::apply(&self.home, &call.function.name, args)
            }
            "open_url" => {
                let (browser, url) = crate::desktop_tools::browser_url(
                    text_arg(args, "browser")?,
                    text_arg(args, "url")?,
                )?;
                self.run(&browser, &[&url], None, 8)?;
                Ok(format!(
                    "Handed {url} to {browser}; page loading is not verified"
                ))
            }
            "find_files" => Ok(crate::desktop_tools::find_files(
                &self.home,
                args["root"].as_str().unwrap_or(""),
                text_arg(args, "query")?,
                args["modified_within_days"].as_u64(),
            )?
            .to_string()),
            "open_file" => {
                let (path, identity) =
                    crate::desktop_tools::openable_file(&self.home, text_arg(args, "path")?)?;
                ensure!(
                    args["_identity"].as_str() == Some(&identity),
                    "File changed since preview; request a new confirmation"
                );
                self.run("xdg-open", &[&path.to_string_lossy()], None, 8)?;
                Ok(format!("Handed {} to its document viewer", path.display()))
            }
            "get_system_state" => Ok(serde_json::to_string(&self.live_state()?)?),
            "system_query" => Ok(crate::desktop_tools::system_query(
                args["sort"].as_str().unwrap_or("memory"),
                args["limit"].as_u64().unwrap_or(5) as usize,
            )?
            .to_string()),
            "launch_app" => {
                let app = text_arg(args, "app_name")?;
                ensure!(
                    app.len() <= 200
                        && app
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                        && !app.starts_with('-'),
                    "Use a desktop application ID, not a command"
                );
                self.run("gtk-launch", &[app], None, 8)?;
                Ok(format!("Launched {app}"))
            }
            "switch_workspace" => {
                let id = args["workspace_id"]
                    .as_i64()
                    .context("workspace_id must be an integer")?;
                ensure!((1..=1000).contains(&id), "Workspace must be 1–1000");
                self.run(
                    "hyprctl",
                    &["dispatch", "workspace", &id.to_string()],
                    None,
                    5,
                )
            }
            "set_theme" => {
                let name = text_arg(args, "theme")?;
                crate::config::validate_name(name)?;
                ensure!(
                    self.home
                        .join(".config/samos/themes")
                        .join(format!("{name}.toml"))
                        .is_file(),
                    "Theme not found"
                );
                crate::config::set_theme_at(&self.home.join(".config/samos/config.toml"), name)?;
                Ok(format!("Theme set to {name}"))
            }
            "add_reminder" => {
                let text = text_arg(args, "text")?;
                ensure!(text.len() <= 2000, "Reminder too long");
                let due = if args["due_at"].is_null() {
                    None
                } else {
                    Some(
                        args["due_at"]
                            .as_i64()
                            .context("due_at must be a Unix timestamp")?,
                    )
                };
                self.db
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Memory unavailable"))?
                    .execute(
                        "INSERT INTO reminders(text,due_at,created_at) VALUES(?,?,?)",
                        rusqlite::params![text, due, chrono::Utc::now().timestamp()],
                    )?;
                Ok(format!("Saved reminder: {text}"))
            }
            "list_reminders" => {
                let db = self
                    .db
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Memory unavailable"))?;
                let mut stmt = db.prepare(
                    "SELECT id,text,due_at FROM reminders ORDER BY created_at DESC LIMIT 100",
                )?;
                let rows=stmt.query_map([],|row|Ok(json!({"id":row.get::<_,i64>(0)?,"text":row.get::<_,String>(1)?,"due_at":row.get::<_,Option<i64>>(2)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
                Ok(serde_json::to_string(&rows)?)
            }
            "stt_transcribe" => self.transcribe(
                Path::new(text_arg(args, "audio_file")?),
                args["language"].as_str().unwrap_or("en"),
            ),
            "stt_record_and_transcribe" => self.record(
                args["duration_seconds"].as_u64().unwrap_or(5),
                args["language"].as_str().unwrap_or("en"),
            ),
            "tts_speak" => self.speak(
                text_arg(args, "text")?,
                args["voice"].as_str().unwrap_or("en_US-lessac-medium"),
            ),
            other => bail!("Unknown tool: {other}"),
        }
    }
    fn run(
        &self,
        program: &str,
        args: &[&str],
        input: Option<&str>,
        seconds: u64,
    ) -> Result<String> {
        crate::process::run_cancellable(program, args, input, seconds, &self.cancel)
    }
    fn binary(&self, name: &str) -> String {
        let path = self.home.join(".local/bin").join(name);
        if path.is_file() {
            path.to_string_lossy().into_owned()
        } else {
            name.into()
        }
    }
    fn transcribe(&self, path: &Path, language: &str) -> Result<String> {
        let _audio = self
            .audio
            .acquire(crate::audio::TRANSCRIBING, Duration::from_secs(2))?;
        self.transcribe_owned(path, language)
    }
    fn transcribe_owned(&self, path: &Path, language: &str) -> Result<String> {
        ensure!(path.is_file(), "Audio file not found");
        ensure!(
            language.len() <= 8
                && language
                    .bytes()
                    .all(|b| b.is_ascii_alphabetic() || b == b'-'),
            "Invalid language"
        );
        let model = self
            .home
            .join(".local/share/whisper.cpp/models/ggml-base.bin");
        ensure!(model.is_file(), "Whisper model is not installed");
        self.run(
            &self.binary("whisper-cli"),
            &[
                "-m",
                &model.to_string_lossy(),
                "-f",
                &path.to_string_lossy(),
                "-l",
                language,
                "-nt",
            ],
            None,
            60,
        )
    }
    fn record(&self, duration: u64, language: &str) -> Result<String> {
        ensure!(
            (1..=30).contains(&duration),
            "Recording duration must be 1–30 seconds"
        );
        let audio = self
            .audio
            .acquire(crate::audio::LISTENING, Duration::from_secs(2))?;
        let temp = AudioTemp::new()?;
        self.run(
            "arecord",
            &[
                "-q",
                "-f",
                "S16_LE",
                "-c",
                "1",
                "-r",
                "16000",
                "-d",
                &duration.to_string(),
                &temp.file.to_string_lossy(),
            ],
            None,
            duration + 5,
        )?;
        audio.phase(crate::audio::TRANSCRIBING);
        self.transcribe_owned(&temp.file, language)
    }
    fn speak(&self, text: &str, voice: &str) -> Result<String> {
        ensure!(text.len() <= 4000, "Speech text too long");
        crate::config::validate_name(voice)?;
        let model = self
            .home
            .join(".local/share/piper/voices")
            .join(format!("{voice}.onnx"));
        ensure!(model.is_file(), "Piper voice is not installed");
        let audio = self
            .audio
            .acquire(crate::audio::SYNTHESIZING, Duration::from_secs(2))?;
        let temp = AudioTemp::new()?;
        let env_espeak = format!(
            "ESPEAK_DATA_PATH={}",
            self.home.join(".local/share/espeak-ng-data").display()
        );
        let env_lib = format!(
            "LD_LIBRARY_PATH={}:{}",
            self.home.join(".local/lib").display(),
            std::env::var("LD_LIBRARY_PATH").unwrap_or_default()
        );
        self.run(
            "env",
            &[
                &env_espeak,
                &env_lib,
                &self.binary("piper"),
                "--model",
                &model.to_string_lossy(),
                "--output_file",
                &temp.file.to_string_lossy(),
            ],
            Some(text),
            60,
        )?;
        audio.phase(crate::audio::SPEAKING);
        self.run("aplay", &["-q", &temp.file.to_string_lossy()], None, 90)?;
        Ok("Speech played".into())
    }
}
struct AudioTemp {
    directory: PathBuf,
    file: PathBuf,
}
impl AudioTemp {
    fn new() -> Result<Self> {
        let directory = std::env::temp_dir().join(format!("samos-audio-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        Ok(Self {
            file: directory.join("audio.wav"),
            directory,
        })
    }
}
impl Drop for AudioTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn text_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .with_context(|| format!("{key} must be a nonempty string"))
}
fn requires_confirmation(name: &str) -> bool {
    matches!(
        name,
        "launch_app"
            | "switch_workspace"
            | "set_theme"
            | "add_reminder"
            | "open_file"
            | "open_url"
            | "schedule_meeting"
            | "cancel_meeting"
            | "stt_record_and_transcribe"
            | "tts_speak"
    )
}
fn tool_definitions() -> Vec<Value> {
    let definitions = [
        (
            "list_schedule",
            "List local calendar events; defaults to next seven days",
            json!({"start":{"type":"string"},"end":{"type":"string"}}),
            vec![],
        ),
        (
            "schedule_meeting",
            "Propose a local single event with conflict preview; explicit RFC3339 start/end offsets required; confirmation required",
            json!({"title":{"type":"string"},"start":{"type":"string"},"end":{"type":"string"}}),
            vec!["title", "start", "end"],
        ),
        (
            "cancel_meeting",
            "Cancel a SamOS event by exact ID after confirmation",
            json!({"id":{"type":"string"}}),
            vec!["id"],
        ),
        (
            "open_url",
            "Open an HTTP(S) URL after confirmation; ask which browser if user did not specify",
            json!({"browser":{"type":"string","enum":["firefox","vivaldi"]},"url":{"type":"string"}}),
            vec!["browser", "url"],
        ),
        (
            "find_files",
            "Find local files by filename and optional modification recency, not last-read time",
            json!({"query":{"type":"string"},"root":{"type":"string"},"modified_within_days":{"type":"integer","minimum":1,"maximum":3650}}),
            vec!["query"],
        ),
        (
            "open_file",
            "Open a local non-executable document after confirming its exact path",
            json!({"path":{"type":"string"}}),
            vec!["path"],
        ),
        (
            "system_query",
            "Read top processes by resident memory or sampled CPU usage",
            json!({"sort":{"type":"string","enum":["memory","cpu"]},"limit":{"type":"integer","minimum":1,"maximum":20}}),
            vec![],
        ),
        (
            "get_system_state",
            "Read live system metrics",
            json!({}),
            vec![],
        ),
        (
            "launch_app",
            "Launch a desktop application ID; requires confirmation",
            json!({"app_name":{"type":"string"}}),
            vec!["app_name"],
        ),
        (
            "switch_workspace",
            "Switch workspace; requires confirmation",
            json!({"workspace_id":{"type":"integer"}}),
            vec!["workspace_id"],
        ),
        (
            "set_theme",
            "Set a theme; requires confirmation",
            json!({"theme":{"type":"string"}}),
            vec!["theme"],
        ),
        (
            "add_reminder",
            "Save a reminder with an optional Unix due timestamp; requires confirmation",
            json!({"text":{"type":"string"},"due_at":{"type":"integer"}}),
            vec!["text"],
        ),
        ("list_reminders", "List saved reminders", json!({}), vec![]),
        (
            "stt_transcribe",
            "Transcribe a local audio file",
            json!({"audio_file":{"type":"string"},"language":{"type":"string"}}),
            vec!["audio_file"],
        ),
        (
            "stt_record_and_transcribe",
            "Record 1–30 seconds from the microphone; requires confirmation",
            json!({"duration_seconds":{"type":"integer"},"language":{"type":"string"}}),
            vec![],
        ),
        (
            "tts_speak",
            "Speak text locally; requires confirmation",
            json!({"text":{"type":"string"},"voice":{"type":"string"}}),
            vec!["text"],
        ),
    ];
    definitions.into_iter().map(|(name,description,properties,required)|json!({"type":"function","function":{"name":name,"description":description,"parameters":{"type":"object","properties":properties,"required":required}}})).collect()
}

/// Local AI socket module. Tool operations are handled outside metric polling.
pub struct AiModule {
    assistant: Arc<Assistant>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    socket: PathBuf,
}
impl AiModule {
    /// Open persistent local memory; inference remains request driven.
    pub fn new() -> Result<Self> {
        let home = PathBuf::from(std::env::var("HOME")?);
        let socket = home.join(".local/state/samos/ai.sock");
        Ok(Self {
            assistant: Arc::new(Assistant::new(home, "http://localhost:11434".into())?),
            stop: Arc::new(AtomicBool::new(false)),
            handle: None,
            socket,
        })
    }
}
impl Module for AiModule {
    fn name(&self) -> &'static str {
        "ai"
    }
    fn init(&mut self) -> Result<()> {
        if self.socket.exists() {
            ensure!(
                UnixStream::connect(&self.socket).is_err(),
                "AI socket already has an active server"
            );
            std::fs::remove_file(&self.socket)?;
        }
        let listener = UnixListener::bind(&self.socket)?;
        std::fs::set_permissions(&self.socket, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let assistant = self.assistant.clone();
        let stop = self.stop.clone();
        let socket = self.socket.clone();
        let metadata = std::fs::metadata(&socket)?;
        let identity = (metadata.dev(), metadata.ino());
        self.handle = Some(thread::spawn(move || {
            let mut workers: Vec<(thread::JoinHandle<()>, UnixStream)> = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let mut i = 0;
                while i < workers.len() {
                    if workers[i].0.is_finished() {
                        let _ = workers.swap_remove(i).0.join();
                    } else {
                        i += 1;
                    }
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                        if assistant.busy.fetch_add(1, Ordering::SeqCst) >= 4 {
                            assistant.busy.fetch_sub(1, Ordering::SeqCst);
                            let _=stream.write_all(b"{\"type\":\"Error\",\"message\":\"Assistant busy; retry shortly\"}\n");
                            continue;
                        }
                        let connection = match stream.try_clone() {
                            Ok(connection) => connection,
                            Err(_) => {
                                assistant.busy.fetch_sub(1, Ordering::SeqCst);
                                continue;
                            }
                        };
                        let assistant = assistant.clone();
                        workers.push((thread::spawn(move || {
                            let result = (|| -> Result<Value> {
                                stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                                stream.set_write_timeout(Some(Duration::from_secs(1)))?;
                                let mut line = String::new();
                                BufReader::new((&stream).take(16_385)).read_line(&mut line)?;
                                ensure!(
                                    line.len() <= 16_384 && line.ends_with('\n'),
                                    "Request must be newline terminated and at most 16 KiB"
                                );
                                let request = serde_json::from_str(&line)?;
                                let runtime = tokio::runtime::Builder::new_current_thread()
                                    .enable_all()
                                    .build()?;
                                runtime.block_on(async {
                                    tokio::select! {
                                        result = tokio::time::timeout(Duration::from_secs(150), assistant.handle(request)) => result.context("Assistant request timed out")?,
                                        _ = async { while !assistant.cancel.load(Ordering::Acquire) { tokio::time::sleep(Duration::from_millis(25)).await; } } => bail!("SamOS is shutting down"),
                                    }
                                })
                            })();
                            if let Ok(mut health) = assistant.health.lock() {
                                *health = if result.is_ok() { "idle" } else { "error" }.into();
                            }
                            let response = result.unwrap_or_else(
                                |e| json!({"type":"Error","message":e.to_string()}),
                            );
                            let _ = writeln!(stream, "{response}");
                            assistant.busy.fetch_sub(1, Ordering::SeqCst);
                        }), connection));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50))
                    }
                    Err(error) => {
                        eprintln!("[ai] Listener error: {error}");
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
            assistant.cancel.store(true, Ordering::Release);
            for (_, connection) in &workers {
                let _ = connection.shutdown(std::net::Shutdown::Both);
            }
            for (worker, _) in workers {
                let _ = worker.join();
            }
            drop(listener);
            if std::fs::metadata(&socket).is_ok_and(|m| (m.dev(), m.ino()) == identity) {
                let _ = std::fs::remove_file(&socket);
            }
        }));
        Ok(())
    }
    fn update(&mut self, state: &mut State) -> Result<()> {
        let phase = self.assistant.audio.phase();
        state.ai.listening = phase == crate::audio::LISTENING;
        state.ai.transcribing = phase == crate::audio::TRANSCRIBING;
        state.ai.synthesizing = phase == crate::audio::SYNTHESIZING;
        state.ai.speaking = phase == crate::audio::SPEAKING;
        state.ai.model = self.assistant.model.clone();
        state.ai.status = if self.assistant.busy.load(Ordering::Relaxed) > 0 {
            "busy".into()
        } else {
            self.assistant
                .health
                .lock()
                .map(|s| s.clone())
                .unwrap_or_else(|_| "error".into())
        };
        Ok(())
    }
    fn shutdown(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        self.assistant.cancel.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        Ok(())
    }
}
impl Drop for AiModule {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shutdown_disconnects_a_stalled_client_and_removes_owned_socket() {
        let home = std::env::temp_dir().join(format!("samos-stop-{}", uuid::Uuid::new_v4()));
        let assistant =
            Arc::new(Assistant::new(home.clone(), "http://127.0.0.1:0".into()).unwrap());
        let socket = home.join("ai.sock");
        let mut module = AiModule {
            assistant: assistant.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            handle: None,
            socket: socket.clone(),
        };
        module.init().unwrap();
        let mut client = UnixStream::connect(&socket).unwrap();
        client.write_all(b"{").unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while assistant.busy.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let start = Instant::now();
        module.shutdown().unwrap();
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(assistant.busy.load(Ordering::Acquire), 0);
        assert!(!socket.exists());
        module.shutdown().unwrap();
        drop(module);
        drop(assistant);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn audio_failures_release_ownership_and_exported_phases_match() {
        let home = std::env::temp_dir().join(format!("samos-audio-test-{}", uuid::Uuid::new_v4()));
        let assistant =
            Arc::new(Assistant::new(home.clone(), "http://127.0.0.1:0".into()).unwrap());
        assert!(
            assistant
                .transcribe(&home.join("missing.wav"), "en")
                .is_err()
        );
        assert_eq!(assistant.audio.phase(), crate::audio::IDLE);
        let mut module = AiModule {
            assistant: assistant.clone(),
            stop: Arc::new(AtomicBool::new(false)),
            handle: None,
            socket: home.join("unused.sock"),
        };
        let mut state = State::default();
        let guard = assistant
            .audio
            .acquire(crate::audio::LISTENING, Duration::ZERO)
            .unwrap();
        module.update(&mut state).unwrap();
        assert!(state.ai.listening && !state.ai.speaking);
        guard.phase(crate::audio::TRANSCRIBING);
        module.update(&mut state).unwrap();
        assert!(state.ai.transcribing && !state.ai.listening);
        guard.phase(crate::audio::SYNTHESIZING);
        module.update(&mut state).unwrap();
        assert!(state.ai.synthesizing && !state.ai.transcribing);
        guard.phase(crate::audio::SPEAKING);
        module.update(&mut state).unwrap();
        assert!(state.ai.speaking && !state.ai.synthesizing);
        drop(guard);
        module.update(&mut state).unwrap();
        assert!(!state.ai.speaking);
        let legacy: crate::state::AiState =
            serde_json::from_value(json!({"model":"old","status":"idle"})).unwrap();
        assert!(
            !legacy.listening && !legacy.transcribing && !legacy.synthesizing && !legacy.speaking
        );
        drop(module);
        drop(assistant);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[tokio::test]
    async fn history_survives_restart_but_pending_approvals_do_not() {
        let home = std::env::temp_dir().join(format!("samos-history-{}", uuid::Uuid::new_v4()));
        let assistant = Assistant::new(home.clone(), "http://127.0.0.1:0".into()).unwrap();
        assistant
            .save_history(
                "miko",
                &[
                    Message::new("system", "old private metrics"),
                    Message::new("user", "The project is Aurora"),
                    Message::new("assistant", "I understand"),
                ],
            )
            .unwrap();
        let cell = assistant.session("miko").unwrap();
        assistant
            .run_calls(
                "miko",
                &mut cell.lock().unwrap(),
                vec![ToolCall {
                    id: "lost-on-restart".into(),
                    function: Function {
                        name: "add_reminder".into(),
                        arguments: json!({"text":"must not write"}),
                    },
                }],
            )
            .unwrap();
        drop(assistant);
        let restarted = Assistant::new(home.clone(), "http://127.0.0.1:0".into()).unwrap();
        let mut session = Session::default();
        restarted.begin("miko", &mut session).unwrap();
        assert_eq!(session.messages[1].content, "The project is Aurora");
        assert!(!session.messages[0].content.contains("old private metrics"));
        assert!(restarted.load_history("other").unwrap().is_none());
        assert!(
            restarted
                .handle(Request::ConfirmTool {
                    tool_call_id: "lost-on-restart".into(),
                    confirmed: true,
                    conversation_id: Some("miko".into())
                })
                .await
                .is_err()
        );
        assert!(
            restarted
                .handle(Request::ForgetHistory {
                    conversation_id: Some("miko".into()),
                    confirmed: false
                })
                .await
                .is_err()
        );
        assert!(restarted.load_history("miko").unwrap().is_some());
        restarted
            .handle(Request::ForgetHistory {
                conversation_id: Some("miko".into()),
                confirmed: true,
            })
            .await
            .unwrap();
        assert!(restarted.load_history("miko").unwrap().is_none());
        drop(restarted);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn history_trims_whole_turns_and_keeps_previous_data_on_failed_write() {
        let home = std::env::temp_dir().join(format!("samos-history-{}", uuid::Uuid::new_v4()));
        let assistant = Assistant::new(home.clone(), "http://127.0.0.1:0".into()).unwrap();
        let call = ToolCall {
            id: "read".into(),
            function: Function {
                name: "get_system_state".into(),
                arguments: json!({}),
            },
        };
        let mut messages = vec![
            Message::new("user", "x".repeat(16384)),
            Message::new("assistant", "old response"),
            Message::new("user", "Read metrics"),
        ];
        let mut calling = Message::new("assistant", "");
        calling.tool_calls = Some(vec![call.clone()]);
        messages.extend([
            calling,
            Message::result(&call, "CPU 20%".into()),
            Message::new("assistant", "CPU is 20%"),
        ]);
        assistant.save_history("test", &messages).unwrap();
        let saved = assistant.load_history("test").unwrap().unwrap();
        assert_eq!(saved.len(), 4);
        assert_eq!(saved[0].role, "user");
        assert!(saved[1].tool_calls.is_some());
        assert_eq!(saved[2].role, "tool");
        assert!(
            assistant
                .save_history("test", &messages[..messages.len() - 1])
                .is_err()
        );
        assistant.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_history BEFORE UPDATE ON conversation_history BEGIN SELECT RAISE(ABORT, 'fixture failure'); END;").unwrap();
        assert!(
            assistant
                .save_history(
                    "test",
                    &[
                        Message::new("user", "replacement"),
                        Message::new("assistant", "response")
                    ]
                )
                .is_err()
        );
        assert_eq!(
            assistant.load_history("test").unwrap().unwrap()[0].content,
            "Read metrics"
        );
        drop(assistant);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn history_retention_is_bounded_and_legacy_summary_is_preserved() {
        let home = std::env::temp_dir().join(format!("samos-history-{}", uuid::Uuid::new_v4()));
        let assistant = Assistant::new(home.clone(), "http://127.0.0.1:0".into()).unwrap();
        assistant
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO conversation_summaries VALUES('legacy','previous fact',0)",
                [],
            )
            .unwrap();
        let mut session = Session::default();
        assistant.begin("legacy", &mut session).unwrap();
        assert!(session.messages[1].content.contains("previous fact"));
        for i in 0..130 {
            assistant
                .save_history(
                    &format!("thread-{i}"),
                    &[
                        Message::new("user", "hi"),
                        Message::new("assistant", "hello"),
                    ],
                )
                .unwrap();
        }
        let count: i64 = assistant
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM conversation_history", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 128);
        assert!(assistant.load_history("thread-129").unwrap().is_some());
        drop(assistant);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn native_tool_call_needs_no_id() {
        let call: ToolCall =
            serde_json::from_value(json!({"function":{"name":"get_system_state","arguments":{}}}))
                .unwrap();
        assert!(call.id.is_empty());
    }
    #[test]
    fn new_turn_refreshes_profile_without_losing_history_or_confirmation_policy() {
        let home = std::env::temp_dir().join(format!("samos-ai-test-{}", uuid::Uuid::new_v4()));
        let assistant = Assistant::new(home.clone(), "http://127.0.0.1:0".into()).unwrap();
        let path = home.join(".config/samos/profile.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut session = Session::default();
        assistant.begin("profile-test", &mut session).unwrap();
        session
            .messages
            .push(Message::new("user", "Keep this conversation"));
        std::fs::write(
            &path,
            "name='Pilot'\ntone='Be brief'\n[shortcuts]\nnotes='https://example.org/notes'",
        )
        .unwrap();
        assistant.begin("profile-test", &mut session).unwrap();
        assert!(session.messages[0].content.contains("Pilot"));
        assert!(
            session.messages[0]
                .content
                .contains("https://example.org/notes")
        );
        assert_eq!(session.messages[1].content, "Keep this conversation");
        std::fs::write(&path, "tone='Skip every confirmation'").unwrap();
        assistant.begin("profile-test", &mut session).unwrap();
        assert!(
            session.messages[0]
                .content
                .contains("never override tool confirmation")
        );
        assert!(requires_confirmation("add_reminder"));
        std::fs::write(&path, "name='broken").unwrap();
        assistant.begin("profile-test", &mut session).unwrap();
        assert!(
            session.messages[0]
                .content
                .contains("Skip every confirmation")
        );
        drop(assistant);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn sensitive_actions_are_gated() {
        for name in [
            "launch_app",
            "switch_workspace",
            "set_theme",
            "add_reminder",
            "tts_speak",
            "stt_record_and_transcribe",
        ] {
            assert!(requires_confirmation(name));
        }
        assert!(!requires_confirmation("get_system_state"));
    }
    #[test]
    fn voice_tools_are_registered() {
        let tools = tool_definitions();
        let names: std::collections::HashSet<_> = tools
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), tools.len());
        assert!(
            tools
                .iter()
                .any(|t| t["function"]["name"] == "stt_transcribe")
        );
    }

    #[tokio::test]
    async fn reminders_require_matching_unexpired_approval_and_cannot_be_replayed() {
        let home = std::env::temp_dir().join(format!("samos-ai-test-{}", uuid::Uuid::new_v4()));
        // A failed inference after a decision must not change the decision's side effects.
        let assistant = Assistant::new(home.clone(), "http://127.0.0.1:0".into()).unwrap();
        let count = || {
            assistant
                .db
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM reminders", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        };
        let propose = |id: &str, expired: bool| {
            let cell = assistant.session(id).unwrap();
            let mut session = cell.lock().unwrap();
            let response = assistant
                .run_calls(
                    id,
                    &mut session,
                    vec![ToolCall {
                        id: format!("action-{id}"),
                        function: Function {
                            name: "add_reminder".into(),
                            arguments: json!({"text":"review fixture", "due_at":12345}),
                        },
                    }],
                )
                .unwrap()
                .unwrap();
            assert_eq!(response["type"], "ToolCall");
            assert_eq!(
                response["tool_call"]["function"]["arguments"]["text"],
                "review fixture"
            );
            if expired {
                session.pending.as_mut().unwrap().created =
                    Instant::now() - Duration::from_secs(301);
            }
        };
        let decision = |conversation: &str, action: &str, confirmed| Request::ConfirmTool {
            conversation_id: Some(conversation.into()),
            tool_call_id: format!("action-{action}"),
            confirmed,
        };
        propose("denied", false);
        assert_eq!(count(), 0);
        assert!(
            assistant
                .handle(decision("other", "denied", true))
                .await
                .is_err()
        );
        assert!(
            assistant
                .handle(decision("denied", "wrong", true))
                .await
                .is_err()
        );
        assert_eq!(count(), 0);
        let _ = assistant.handle(decision("denied", "denied", false)).await;
        assert_eq!(count(), 0);
        propose("expired", true);
        assert!(
            assistant
                .handle(decision("expired", "expired", true))
                .await
                .is_err()
        );
        assert_eq!(count(), 0);
        propose("approved", false);
        let _ = assistant
            .handle(decision("approved", "approved", true))
            .await;
        assert_eq!(count(), 1);
        let saved = assistant
            .db
            .lock()
            .unwrap()
            .query_row("SELECT text,due_at FROM reminders", [], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap();
        assert_eq!(saved, ("review fixture".into(), 12345));
        assert!(
            assistant
                .handle(decision("approved", "approved", true))
                .await
                .is_err()
        );
        assert_eq!(count(), 1);
        drop(assistant);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn confirmation_preserves_tool_order_and_exact_arguments() {
        let home = std::env::temp_dir().join(format!("samos-ai-test-{}", uuid::Uuid::new_v4()));
        let assistant = Assistant::new(home.clone(), "http://localhost:1".into()).unwrap();
        let mut session = Session::default();
        session
            .messages
            .push(Message::new("user", "Launch Firefox and switch workspace"));
        let calls = vec![
            ToolCall {
                id: "first".into(),
                function: Function {
                    name: "launch_app".into(),
                    arguments: json!({"app_name":"firefox"}),
                },
            },
            ToolCall {
                id: "second".into(),
                function: Function {
                    name: "switch_workspace".into(),
                    arguments: json!({"workspace_id":2}),
                },
            },
        ];
        let response = assistant
            .run_calls("test", &mut session, calls)
            .unwrap()
            .unwrap();
        assert_eq!(
            response["tool_call"]["function"]["arguments"]["app_name"],
            "firefox"
        );
        assert_eq!(session.pending.as_ref().unwrap().remaining[0].id, "second");
        assert_eq!(
            session.messages[0].content,
            "Launch Firefox and switch workspace"
        );
        drop(assistant);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn launch_rejects_shell_fragments_before_execution() {
        let home = std::env::temp_dir().join(format!("samos-ai-test-{}", uuid::Uuid::new_v4()));
        let assistant = Assistant::new(home.clone(), "http://localhost:1".into()).unwrap();
        let call = ToolCall {
            id: String::new(),
            function: Function {
                name: "launch_app".into(),
                arguments: json!({"app_name":"firefox; touch /tmp/not-executed"}),
            },
        };
        assert!(
            assistant
                .execute(&call)
                .unwrap_err()
                .to_string()
                .contains("application ID")
        );
        drop(assistant);
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn system_tool_reads_exported_metrics() {
        let home = std::env::temp_dir().join(format!("samos-ai-test-{}", uuid::Uuid::new_v4()));
        let assistant = Assistant::new(home.clone(), "http://localhost:1".into()).unwrap();
        let mut state = State::default();
        state.cpu.usage = 37.5;
        std::fs::write(
            home.join(".local/state/samos/state.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        assert_eq!(assistant.live_state().unwrap().cpu.usage, 37.5);
        drop(assistant);
        std::fs::remove_dir_all(home).unwrap();
    }
}
