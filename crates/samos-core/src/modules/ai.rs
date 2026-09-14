use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::io::{BufRead, BufReader, Write};

use crate::{modules::Module, state::State};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<Tool>>,
    stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Tool {
    #[serde(rename = "type")]
    tool_type: String,
    function: FunctionDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FunctionDef {
    name: String,
    description: String,
    parameters: ToolParameters,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolParameters {
    #[serde(rename = "type")]
    param_type: String,
    properties: HashMap<String, ToolProperty>,
    required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolProperty {
    #[serde(rename = "type")]
    prop_type: String,
    description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaChatResponse {
    model: String,
    message: ChatMessage,
    done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCall {
    id: String,
    #[serde(rename = "type", default)]
    tool_type: String,
    function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCallFunction {
    #[serde(default)]
    index: u32,
    name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AiRequest {
    Chat { message: String, conversation_id: Option<String> },
    ConfirmTool { tool_call_id: String, confirmed: bool },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AiResponse {
    Token { text: String },
    ToolCall { tool_call: ToolCall },
    ToolResult { tool_call_id: String, result: String },
    Done { summary: Option<String> },
    Error { message: String },
}

pub struct AiModule {
    client: reqwest::Client,
    ollama_url: String,
    model: String,
    db: Arc<Mutex<rusqlite::Connection>>,
    pending_confirmations: Arc<Mutex<HashMap<String, ToolCall>>>,
    socket_path: String,
    listener_handle: Option<thread::JoinHandle<()>>,
    pending_chats: Arc<Mutex<HashMap<String, Vec<ChatMessage>>>>,
}

impl AiModule {
    pub fn new() -> Result<Self> {
        let home = std::env::var("HOME").unwrap_or_default();
        let db_path = format!("{}/.local/state/samos/ai.db", home);

        std::fs::create_dir_all(std::path::Path::new(&db_path).parent().unwrap())?;

        let conn = rusqlite::Connection::open(&db_path)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS reminders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                due_at INTEGER,
                created_at INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS conversations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                summary TEXT
            )",
            [],
        )?;

        let socket_path = format!("{}/.local/state/samos/ai.sock", home);
        std::fs::create_dir_all(std::path::Path::new(&socket_path).parent().unwrap())?;

        Ok(Self {
            client: reqwest::Client::new(),
            ollama_url: "http://localhost:11434".to_string(),
            model: "qwen2.5:7b".to_string(),
            db: Arc::new(Mutex::new(conn)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
            socket_path,
            listener_handle: None,
            pending_chats: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn get_tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                tool_type: "function".to_string(),
                function: FunctionDef {
                    name: "get_system_state".to_string(),
                    description: "Get current system state (CPU, memory, battery, disk, network, temperature)".to_string(),
                    parameters: ToolParameters {
                        param_type: "object".to_string(),
                        properties: HashMap::new(),
                        required: vec![],
                    },
                },
            },
            Tool {
                tool_type: "function".to_string(),
                function: FunctionDef {
                    name: "launch_app".to_string(),
                    description: "Launch an application by name (requires confirmation)".to_string(),
                    parameters: ToolParameters {
                        param_type: "object".to_string(),
                        properties: {
                            let mut props = HashMap::new();
                            props.insert("app_name".to_string(), ToolProperty {
                                prop_type: "string".to_string(),
                                description: "Name of the application to launch".to_string(),
                            });
                            props
                        },
                        required: vec!["app_name".to_string()],
                    },
                },
            },
            Tool {
                tool_type: "function".to_string(),
                function: FunctionDef {
                    name: "switch_workspace".to_string(),
                    description: "Switch to a different workspace (requires confirmation)".to_string(),
                    parameters: ToolParameters {
                        param_type: "object".to_string(),
                        properties: {
                            let mut props = HashMap::new();
                            props.insert("workspace_id".to_string(), ToolProperty {
                                prop_type: "integer".to_string(),
                                description: "Workspace ID to switch to".to_string(),
                            });
                            props
                        },
                        required: vec!["workspace_id".to_string()],
                    },
                },
            },
            Tool {
                tool_type: "function".to_string(),
                function: FunctionDef {
                    name: "set_theme".to_string(),
                    description: "Change the theme (requires confirmation)".to_string(),
                    parameters: ToolParameters {
                        param_type: "object".to_string(),
                        properties: {
                            let mut props = HashMap::new();
                            props.insert("theme".to_string(), ToolProperty {
                                prop_type: "string".to_string(),
                                description: "Theme name (hud, hacker, elegant, motivation, love, movie)".to_string(),
                            });
                            props
                        },
                        required: vec!["theme".to_string()],
                    },
                },
            },
            Tool {
                tool_type: "function".to_string(),
                function: FunctionDef {
                    name: "add_reminder".to_string(),
                    description: "Add a reminder with optional due time".to_string(),
                    parameters: ToolParameters {
                        param_type: "object".to_string(),
                        properties: {
                            let mut props = HashMap::new();
                            props.insert("text".to_string(), ToolProperty {
                                prop_type: "string".to_string(),
                                description: "Reminder text".to_string(),
                            });
                            props.insert("due_at".to_string(), ToolProperty {
                                prop_type: "integer".to_string(),
                                description: "Unix timestamp for due time (optional)".to_string(),
                            });
                            props
                        },
                        required: vec!["text".to_string()],
                    },
                },
            },
            Tool {
                tool_type: "function".to_string(),
                function: FunctionDef {
                    name: "list_reminders".to_string(),
                    description: "List all active reminders".to_string(),
                    parameters: ToolParameters {
                        param_type: "object".to_string(),
                        properties: HashMap::new(),
                        required: vec![],
                    },
                },
            },
        ]
    }

    async fn execute_tool(&self, tool_call: &ToolCall, state: &State) -> Result<String> {
        let args: Value = tool_call.function.arguments.clone();

        match tool_call.function.name.as_str() {
            "get_system_state" => {
                Ok(format!(
                    "CPU: {:.1}%, RAM: {:.1}%, Battery: {:.0}% ({}), Disk: {:.1}%, Net: {}KB ↑{}KB, Temp: {:.1}°C",
                    state.cpu.usage,
                    state.memory.used_percent,
                    state.battery.percent,
                    state.battery.status,
                    state.disk.used_percent,
                    state.network.rx_kb,
                    state.network.tx_kb,
                    state.temperature.celsius
                ))
            }
            "launch_app" => {
                let app_name = args.get("app_name").and_then(|v| v.as_str()).unwrap_or("");
                let output = std::process::Command::new("sh")
                    .args(["-c", &format!("gtk-launch {} 2>/dev/null || echo 'failed'", app_name)])
                    .output()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(stdout.trim().to_string())
            }
            "switch_workspace" => {
                let workspace_id = args.get("workspace_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let output = std::process::Command::new("hyprctl")
                    .args(["dispatch", "workspace", &workspace_id.to_string()])
                    .output()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(stdout.trim().to_string())
            }
            "set_theme" => {
                let theme = args.get("theme").and_then(|v| v.as_str()).unwrap_or("");
                let output = std::process::Command::new("samosctl")
                    .args(["theme-set", theme])
                    .output()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(stdout.trim().to_string())
            }
            "add_reminder" => {
                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let due_at = args.get("due_at").and_then(|v| v.as_i64());

                let conn = self.db.lock().unwrap();
                let created_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs() as i64;

                conn.execute(
                    "INSERT INTO reminders (text, due_at, created_at) VALUES (?, ?, ?)",
                    rusqlite::params![text, due_at, created_at],
                )?;

                Ok(format!("Reminder added: {}", text))
            }
            "list_reminders" => {
                let conn = self.db.lock().unwrap();
                let mut stmt = conn.prepare(
                    "SELECT id, text, due_at FROM reminders ORDER BY created_at DESC"
                )?;

                let reminders: Vec<String> = stmt.query_map([], |row| {
                    let id: i64 = row.get(0)?;
                    let text: String = row.get(1)?;
                    let due_at: Option<i64> = row.get(2)?;

                    let due_str = due_at.map(|d| format!(" (due: {})",
                        chrono::DateTime::from_timestamp(d, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| d.to_string())
                    )).unwrap_or_default();

                    Ok(format!("[{}] {}{}", id, text, due_str))
                })?.collect::<Result<Vec<_>, _>>()?;

                if reminders.is_empty() {
                    Ok("No reminders".to_string())
                } else {
                    Ok(reminders.join("\n"))
                }
            }
            _ => Ok(format!("Unknown tool: {}", tool_call.function.name)),
        }
    }

    async fn chat_with_ollama(&self, messages: &[ChatMessage], state: &State) -> Result<String> {
        let system_prompt = format!(
            "You are MIKO, an AI assistant integrated into the SamOS Linux desktop environment. \
             You have access to system information and can control the desktop. \
             Use tools when appropriate. Be concise and helpful.\n\n\
             Current system state: CPU {:.1}%, RAM {:.1}%, Battery {}% ({}), Disk {:.1}%",
            state.cpu.usage,
            state.memory.used_percent,
            state.battery.percent,
            state.battery.status,
            state.disk.used_percent
        );

        let mut full_messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt, tool_calls: None },
        ];
        full_messages.extend(messages.iter().cloned());

        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages: full_messages,
            tools: Some(self.get_tools()),
            stream: false,
        };

        let response = self.client
            .post(&format!("{}/api/chat", self.ollama_url))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let err = response.text().await.unwrap_or_default();
            return Err(anyhow!("Ollama error: {}", err));
        }

        let chat_response: OllamaChatResponse = response.json().await?;

        let tool_calls = chat_response.message.tool_calls.clone();
        let message_content = chat_response.message.content.clone();

        if let Some(tool_calls) = tool_calls {
            for tool_call in tool_calls {
                if tool_call.function.name == "launch_app"
                    || tool_call.function.name == "switch_workspace"
                    || tool_call.function.name == "set_theme"
                {
                    self.pending_confirmations.lock().unwrap()
                        .insert(tool_call.id.clone(), tool_call.clone());
                    return Ok(format!("CONFIRMATION_REQUIRED:{}", tool_call.id));
                }

                let result = self.execute_tool(&tool_call, state).await?;
                self.save_tool_result(&tool_call, &result)?;

                return Box::pin(self.chat_with_ollama(&[
                    ChatMessage {
                        role: "assistant".to_string(),
                        content: message_content.clone(),
                        tool_calls: None,
                    },
                    ChatMessage {
                        role: "tool".to_string(),
                        content: result,
                        tool_calls: None,
                    },
                ], state)).await;
            }
        }

        Ok(message_content)
    }

    fn save_tool_result(&self, tool_call: &ToolCall, result: &str) -> Result<()> {
        let conn = self.db.lock().unwrap();
        conn.execute(
            "INSERT INTO conversations (role, content, timestamp, summary) VALUES (?, ?, ?, ?)",
            rusqlite::params!["tool", format!("{}: {}", tool_call.function.name, result),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64,
                result],
        )?;
        Ok(())
    }

    fn save_message(&self, role: &str, content: &str) -> Result<()> {
        let conn = self.db.lock().unwrap();
        conn.execute(
            "INSERT INTO conversations (role, content, timestamp, summary) VALUES (?, ?, ?, ?)",
            rusqlite::params![role, content,
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64,
                content],
        )?;
        Ok(())
    }

    fn get_recent_context(&self, limit: usize) -> Result<String> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT role, content FROM conversations ORDER BY timestamp DESC LIMIT ?"
        )?;

        let messages = stmt.query_map([limit], |row| {
            let role: String = row.get(0)?;
            let content: String = row.get(1)?;
            Ok(format!("{}: {}", role, content))
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(messages.into_iter().rev().collect::<Vec<_>>().join("\n"))
    }

    fn start_socket_listener(&mut self) -> Result<()> {
        let socket_path = self.socket_path.clone();
        if Path::new(&socket_path).exists() {
            std::fs::remove_file(&socket_path)?;
        }

        let listener = UnixListener::bind(&socket_path)?;

        let pending_confirmations = self.pending_confirmations.clone();
        let pending_chats = self.pending_chats.clone();
        let client = self.client.clone();
        let ollama_url = self.ollama_url.clone();
        let model = self.model.clone();
        let db = self.db.clone();
        let tools = self.get_tools();

        let handle = thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        let mut reader = BufReader::new(&stream);
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_ok() {
                            if let Ok(request) = serde_json::from_str::<AiRequest>(line.trim()) {
                                let response = Self::handle_request(
                                    request,
                                    &pending_confirmations,
                                    &pending_chats,
                                    &client,
                                    &ollama_url,
                                    &model,
                                    &db,
                                    &tools,
                                );
                                let response_json = serde_json::to_string(&response).unwrap_or_else(|_| {
                                    r#"{"type":"Error","message":"Failed to serialize response"}"#.to_string()
                                });
                                let _ = stream.write_all(format!("{}\n", response_json).as_bytes());
                                let _ = stream.flush();
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[ai] Socket accept error: {}", e);
                    }
                }
            }
        });

        self.listener_handle = Some(handle);
        Ok(())
    }

    fn handle_request(
        request: AiRequest,
        pending_confirmations: &Arc<Mutex<HashMap<String, ToolCall>>>,
        pending_chats: &Arc<Mutex<HashMap<String, Vec<ChatMessage>>>>,
        client: &reqwest::Client,
        ollama_url: &str,
        model: &str,
        db: &Arc<Mutex<rusqlite::Connection>>,
        tools: &[Tool],
    ) -> AiResponse {
        match request {
            AiRequest::Chat { message, conversation_id } => {
                let conv_id = conversation_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                
                let mut chats = pending_chats.lock().unwrap();
                let history = chats.entry(conv_id.clone()).or_insert_with(Vec::new);
                history.push(ChatMessage { role: "user".to_string(), content: message, tool_calls: None });

                let state = State::default();
                let rt = tokio::runtime::Runtime::new().unwrap();
                match rt.block_on(Self::process_chat(history.clone(), &state, client, ollama_url, model, tools, db, pending_confirmations)) {
                    Ok(response) => {
                        history.push(ChatMessage { role: "assistant".to_string(), content: response.clone(), tool_calls: None });
                        AiResponse::Done { summary: Some(response) }
                    }
                    Err(e) => {
                        eprintln!("[ai] process_chat error: {}", e);
                        AiResponse::Error { message: e.to_string() }
                    }
                }
            }
            AiRequest::ConfirmTool { tool_call_id, confirmed } => {
                let mut pending = pending_confirmations.lock().unwrap();
                if let Some(tool_call) = pending.remove(&tool_call_id) {
                    if confirmed {
                        let state = State::default();
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        match rt.block_on(Self::execute_tool_static(&tool_call, &state, db)) {
                            Ok(result) => AiResponse::ToolResult { tool_call_id, result },
                            Err(e) => AiResponse::Error { message: format!("Tool execution failed: {}", e) },
                        }
                    } else {
                        AiResponse::Error { message: "Tool execution denied by user".to_string() }
                    }
                } else {
                    AiResponse::Error { message: "Unknown tool call ID".to_string() }
                }
            }
        }
    }

    async fn process_chat(
        messages: Vec<ChatMessage>,
        state: &State,
        client: &reqwest::Client,
        ollama_url: &str,
        model: &str,
        tools: &[Tool],
        db: &Arc<Mutex<rusqlite::Connection>>,
        pending_confirmations: &Arc<Mutex<HashMap<String, ToolCall>>>,
    ) -> Result<String> {
        let system_prompt = format!(
            "You are MIKO, an AI assistant integrated into the SamOS Linux desktop environment. \
             You have access to system information and can control the desktop. \
             Use tools when appropriate. Be concise and helpful.\n\n\
             Current system state: CPU {:.1}%, RAM {:.1}%, Battery {}% ({}), Disk {:.1}%",
            state.cpu.usage,
            state.memory.used_percent,
            state.battery.percent,
            state.battery.status,
            state.disk.used_percent
        );

        let mut full_messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt, tool_calls: None },
        ];
        full_messages.extend(messages);

        let request = OllamaChatRequest {
            model: model.to_string(),
            messages: full_messages,
            tools: Some(tools.to_vec()),
            stream: false,
        };

        let response = client
            .post(&format!("{}/api/chat", ollama_url))
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let err = response.text().await.unwrap_or_default();
            return Err(anyhow!("Ollama error: {}", err));
        }

        let response_text = response.text().await?;
        eprintln!("[ai] Ollama response: {}", response_text);
        let chat_response: OllamaChatResponse = serde_json::from_str(&response_text)?;

        let tool_calls = chat_response.message.tool_calls.clone();
        let message_content = chat_response.message.content.clone();

        if let Some(tool_calls) = tool_calls {
            for tool_call in tool_calls {
                if tool_call.function.name == "launch_app"
                    || tool_call.function.name == "switch_workspace"
                    || tool_call.function.name == "set_theme"
                {
                    pending_confirmations.lock().unwrap()
                        .insert(tool_call.id.clone(), tool_call.clone());
                    return Ok(format!("CONFIRMATION_REQUIRED:{}", tool_call.id));
                }

                let result = Self::execute_tool_static(&tool_call, state, db).await?;
                return Box::pin(Self::process_chat(vec![
                    ChatMessage {
                        role: "assistant".to_string(),
                        content: message_content.clone(),
                        tool_calls: None,
                    },
                    ChatMessage {
                        role: "tool".to_string(),
                        content: result,
                        tool_calls: None,
                    },
                ], state, client, ollama_url, model, tools, db, pending_confirmations)).await;
            }
        }

        Ok(message_content)
    }

    async fn execute_tool_static(
        tool_call: &ToolCall,
        state: &State,
        db: &Arc<Mutex<rusqlite::Connection>>,
    ) -> Result<String> {
        let args: Value = tool_call.function.arguments.clone();

        match tool_call.function.name.as_str() {
            "get_system_state" => Ok(format!(
                "CPU: {:.1}%, RAM: {:.1}%, Battery: {:.0}% ({}), Disk: {:.1}%, Net: {}KB ↑{}KB, Temp: {:.1}°C",
                state.cpu.usage,
                state.memory.used_percent,
                state.battery.percent,
                state.battery.status,
                state.disk.used_percent,
                state.network.rx_kb,
                state.network.tx_kb,
                state.temperature.celsius
            )),
            "launch_app" => {
                let app_name = args.get("app_name").and_then(|v| v.as_str()).unwrap_or("");
                let output = std::process::Command::new("sh")
                    .args(["-c", &format!("gtk-launch {} 2>/dev/null || echo 'failed'", app_name)])
                    .output()?;
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            }
            "switch_workspace" => {
                let workspace_id = args.get("workspace_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let output = std::process::Command::new("hyprctl")
                    .args(["dispatch", "workspace", &workspace_id.to_string()])
                    .output()?;
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            }
            "set_theme" => {
                let theme = args.get("theme").and_then(|v| v.as_str()).unwrap_or("");
                let home = std::env::var("HOME").unwrap_or_default();
                eprintln!("[ai] Setting theme to: {}", theme);
                
                let config_path = std::path::PathBuf::from(&home).join(".config/samos/config.toml");
                let theme_path = std::path::PathBuf::from(&home)
                    .join(".config/samos/themes")
                    .join(format!("{}.toml", theme));

                if !theme_path.exists() {
                    return Ok(format!("Theme '{}' not found", theme));
                }

                let config = format!(
                    "theme = \"{}\"\nmonitor = \"focused\"\nrefresh_ms = 1000\n",
                    theme
                );

                if let Err(e) = std::fs::write(&config_path, config) {
                    return Ok(format!("Failed to write config: {}", e));
                }

                // Generate theme SCSS variables
                let _ = std::process::Command::new("sh")
                    .args(["-c", &format!("~/.config/eww/scripts/theme_switch.sh {}", theme)])
                    .output();

                // Reload Eww to apply new theme
                let _ = std::process::Command::new("eww")
                    .args(["reload"])
                    .output();

                Ok(format!("Theme set to: {}", theme))
            }
            "add_reminder" => {
                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let due_at = args.get("due_at").and_then(|v| v.as_i64());
                let conn = db.lock().unwrap();
                let created_at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs() as i64;
                conn.execute(
                    "INSERT INTO reminders (text, due_at, created_at) VALUES (?, ?, ?)",
                    rusqlite::params![text, due_at, created_at],
                )?;
                Ok(format!("Reminder added: {}", text))
            }
            "list_reminders" => {
                let conn = db.lock().unwrap();
                let mut stmt = conn.prepare(
                    "SELECT id, text, due_at FROM reminders ORDER BY created_at DESC"
                )?;
                let reminders: Vec<String> = stmt.query_map([], |row| {
                    let id: i64 = row.get(0)?;
                    let text: String = row.get(1)?;
                    let due_at: Option<i64> = row.get(2)?;
                    let due_str = due_at.map(|d| format!(" (due: {})",
                        chrono::DateTime::from_timestamp(d, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| d.to_string())
                    )).unwrap_or_default();
                    Ok(format!("[{}] {}{}", id, text, due_str))
                })?.collect::<Result<Vec<_>, _>>()?;
                if reminders.is_empty() { Ok("No reminders".to_string()) }
                else { Ok(reminders.join("\n")) }
            }
            _ => Ok(format!("Unknown tool: {}", tool_call.function.name)),
        }
    }
}

impl Module for AiModule {
    fn name(&self) -> &'static str {
        "ai"
    }

    fn init(&mut self) -> Result<()> {
        self.start_socket_listener()?;
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        state.ai.status = "ready".to_string();
        state.ai.model = self.model.clone();
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        if let Some(handle) = self.listener_handle.take() {
            let _ = handle.join();
        }
        let _ = std::fs::remove_file(&self.socket_path);
        Ok(())
    }
}