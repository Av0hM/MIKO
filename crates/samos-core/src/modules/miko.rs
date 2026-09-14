use crate::state::{AiState, State};
use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

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
    #[serde(rename = "type")]
    tool_type: String,
    function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCallFunction {
    name: String,
    arguments: String,
}

pub struct MikoModule {
    client: Client,
    ollama_url: String,
    model: String,
    db: Arc<Mutex<rusqlite::Connection>>,
    pending_confirmations: Arc<Mutex<HashMap<String, ToolCall>>>,
}

impl MikoModule {
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

        Ok(Self {
            client: Client::new(),
            ollama_url: "http://localhost:11434".to_string(),
            model: "qwen2.5:7b".to_string(),
            db: Arc::new(Mutex::new(conn)),
            pending_confirmations: Arc::new(Mutex::new(HashMap::new())),
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
            Tool {
                tool_type: "function".to_string(),
                function: FunctionDef {
                    name: "stt_transcribe".to_string(),
                    description: "Transcribe audio file to text using local whisper.cpp".to_string(),
                    parameters: ToolParameters {
                        param_type: "object".to_string(),
                        properties: {
                            let mut props = HashMap::new();
                            props.insert("audio_file".to_string(), ToolProperty {
                                prop_type: "string".to_string(),
                                description: "Path to audio file (wav, flac, mp3, ogg)".to_string(),
                            });
                            props.insert("language".to_string(), ToolProperty {
                                prop_type: "string".to_string(),
                                description: "Language code (e.g., 'en', 'es', 'fr')".to_string(),
                            });
                            props
                        },
                        required: vec!["audio_file".to_string()],
                    },
                },
            },
            Tool {
                tool_type: "function".to_string(),
                function: FunctionDef {
                    name: "tts_speak".to_string(),
                    description: "Convert text to speech using local piper TTS".to_string(),
                    parameters: ToolParameters {
                        param_type: "object".to_string(),
                        properties: {
                            let mut props = HashMap::new();
                            props.insert("text".to_string(), ToolProperty {
                                prop_type: "string".to_string(),
                                description: "Text to speak".to_string(),
                            });
                            props.insert("voice".to_string(), ToolProperty {
                                prop_type: "string".to_string(),
                                description: "Voice model to use (default: en_US-lessac-medium)".to_string(),
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
                    name: "stt_record_and_transcribe".to_string(),
                    description: "Record audio from microphone and transcribe to text".to_string(),
                    parameters: ToolParameters {
                        param_type: "object".to_string(),
                        properties: {
                            let mut props = HashMap::new();
                            props.insert("duration_seconds".to_string(), ToolProperty {
                                prop_type: "integer".to_string(),
                                description: "Recording duration in seconds (default: 5)".to_string(),
                            });
                            props.insert("language".to_string(), ToolProperty {
                                prop_type: "string".to_string(),
                                description: "Language code (e.g., 'en', 'es', 'fr')".to_string(),
                            });
                            props
                        },
                        required: vec![],
                    },
                },
            },
        ]
    }

    async fn execute_tool(&self, tool_call: &ToolCall, state: &State) -> Result<String> {
        let args: Value = serde_json::from_str(&tool_call.function.arguments)?;
        
        match tool_call.function.name.as_str() {
            "get_system_state" => {
                let sys = state;
                Ok(format!(
                    "CPU: {:.1}%, RAM: {:.1}%, Battery: {:.0}% ({}), Disk: {:.1}%, Net: ��{}KB ↑{}KB, Temp: {:.1}°C",
                    sys.cpu.usage,
                    sys.memory.used_percent,
                    sys.battery.percent,
                    sys.battery.status,
                    sys.disk.used_percent,
                    sys.network.rx_kb,
                    sys.network.tx_kb,
                    sys.temperature.celsius
                ))
            }
            "launch_app" => {
                let app_name = args.get("app_name").and_then(|v| v.as_str()).unwrap_or("");
                let output = Command::new("sh")
                    .args(["-c", &format!("gtk-launch {} 2>/dev/null || echo 'failed'", app_name)])
                    .output()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(stdout.trim().to_string())
            }
            "switch_workspace" => {
                let workspace_id = args.get("workspace_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let output = Command::new("hyprctl")
                    .args(["dispatch", "workspace", &workspace_id.to_string()])
                    .output()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(stdout.trim().to_string())
            }
            "set_theme" => {
                let theme = args.get("theme").and_then(|v| v.as_str()).unwrap_or("");
                let output = Command::new("samosctl")
                    .args(["theme-set", theme])
                    .output()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(stdout.trim().to_string())
            }
            "add_reminder" => {
                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let due_at = args.get("due_at").and_then(|v| v.as_i64());
                
                let conn = self.db.lock().unwrap();
                let created_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
                
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
            "stt_transcribe" => {
                let audio_file = args.get("audio_file").and_then(|v| v.as_str()).unwrap_or("");
                let language = args.get("language").and_then(|v| v.as_str()).unwrap_or("en");
                
                let output = Command::new("whisper-cli")
                    .args([
                        "-m", "~/.local/share/whisper.cpp/models/ggml-base.bin",
                        "-f", audio_file,
                        "-l", language,
                        "-ot", "txt",
                    ])
                    .output()?;
                
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(format!("Transcription: {}", stdout.trim()))
            }
            "tts_speak" => {
                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let voice = args.get("voice").and_then(|v| v.as_str()).unwrap_or("en_US-lessac-medium");
                
                let voice_model = format!("~/.local/share/piper/voices/{}.onnx", voice);
                let config_file = format!("{}.json", voice_model);
                
                let output = Command::new("sh")
                    .args(["-c", &format!(
                        "ESPEAK_DATA_PATH=~/.local/share/espeak-ng-data LD_LIBRARY_PATH=~/.local/lib:$LD_LIBRARY_PATH piper --model {} --output_file /tmp/tts_output.wav <<< '{}'",
                        voice_model, text
                    )])
                    .output()?;
                
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                
                if output.status.success() {
                    // Play the audio
                    let play_output = Command::new("aplay")
                        .args(["/tmp/tts_output.wav"])
                        .output()?;
                    let play_stdout = String::from_utf8_lossy(&play_output.stdout);
                    Ok(format!("Spoken: {} ({})", text, stdout.trim()))
                } else {
                    Ok(format!("TTS failed: {}", stderr.trim()))
                }
            }
            "stt_record_and_transcribe" => {
                let duration = args.get("duration_seconds").and_then(|v| v.as_i64()).unwrap_or(5);
                let language = args.get("language").and_then(|v| v.as_str()).unwrap_or("en");
                
                // Record audio using arecord
                let temp_file = format!("/tmp/stt_record_{}.wav", uuid::Uuid::new_v4());
                
                let record_output = Command::new("arecord")
                    .args([
                        "-f", "S16_LE",
                        "-c", "1",
                        "-r", "16000",
                        "-d", &duration.to_string(),
                        &temp_file,
                    ])
                    .output()?;
                
                if !record_output.status.success() {
                    let stderr = String::from_utf8_lossy(&record_output.stderr);
                    return Ok(format!("Recording failed: {}", stderr.trim()));
                }
                
                // Transcribe the recorded audio
                let transcribe_output = Command::new("whisper-cli")
                    .args([
                        "-m", "~/.local/share/whisper.cpp/models/ggml-base.bin",
                        "-f", &temp_file,
                        "-l", language,
                        "-ot", "txt",
                    ])
                    .output()?;
                
                // Clean up temp file
                let _ = std::fs::remove_file(&temp_file);
                
                let stdout = String::from_utf8_lossy(&transcribe_output.stdout);
                Ok(format!("Transcription: {}", stdout.trim()))
            }
            _ => Ok(format!("Unknown tool: {}", tool_call.function.name)),
        }
    }

    pub async fn chat(&mut self, user_message: &str, state: &mut State) -> Result<String> {
        self.save_message("user", user_message)?;

        let context = self.get_recent_context(10)?;

        let system_prompt = format!(
            "You are MIKO, an AI assistant integrated into the SamOS Linux desktop environment. \
             You have access to system information and can control the desktop. \
             Use tools when appropriate. Be concise and helpful.\n\n\
             Recent conversation:\n{}\n\n\
             Current system state: CPU {:.1}%, RAM {:.1}%, Battery {}% ({}), Disk {:.1}%",
            context,
            state.cpu.usage,
            state.memory.used_percent,
            state.battery.percent,
            state.battery.status,
            state.disk.used_percent
        );

        let messages = vec![
            ChatMessage { role: "system".to_string(), content: system_prompt, tool_calls: None },
            ChatMessage { role: "user".to_string(), content: user_message.to_string(), tool_calls: None },
        ];

        let request = OllamaChatRequest {
            model: self.model.clone(),
            messages,
            tools: Some(self.get_tools()),
            stream: false,
        };

        let response = self
            .client
            .post(&format!("{}/api/chat", self.ollama_url))
            .json(&request)
            .send()
            .await?;

        let ollama_response: OllamaChatResponse = response.json().await?;

        let mut final_response = ollama_response.message.content.clone();

        if let Some(tool_calls) = ollama_response.message.tool_calls {
            for tool_call in tool_calls {
                let requires_confirmation = matches!(
                    tool_call.function.name.as_str(),
                    "launch_app" | "switch_workspace" | "set_theme"
                );

                if requires_confirmation {
                    let confirmation_id = uuid::Uuid::new_v4().to_string();
                    self.pending_confirmations
                        .lock()
                        .unwrap()
                        .insert(confirmation_id.clone(), tool_call.clone());

                    final_response = format!(
                        "{} ������ **Confirmation required for {}**\n\
                         Arguments: {}\n\n\
                         Reply with `confirm <id>` to execute or `deny <id>` to cancel.",
                        final_response,
                        tool_call.function.name,
                        tool_call.function.arguments,
                    );
                } else {
                    let result = self.execute_tool(&tool_call, &state).await?;
                    final_response = format!("{}\n\n[Tool Result: {}]", final_response, result);
                    self.save_message("tool", &format!("{}: {}", tool_call.function.name, result))?;
                }
            }
        }

        self.save_message("assistant", &final_response)?;

        Ok(final_response)
    }

    fn save_message(&self, role: &str, content: &str) -> Result<()> {
        let conn = self.db.lock().unwrap();
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;

        conn.execute(
            "INSERT INTO conversations (role, content, timestamp) VALUES (?, ?, ?)",
            rusqlite::params![role, content, timestamp],
        )?;

        Ok(())
    }

    fn get_recent_context(&self, limit: usize) -> Result<String> {
        let conn = self.db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT role, content FROM conversations ORDER BY timestamp DESC LIMIT ?")?;

        let messages: Vec<String> = stmt
            .query_map([limit], |row| {
                let role: String = row.get(0)?;
                let content: String = row.get(1)?;
                Ok(format!("{}: {}", role, content))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(messages.into_iter().rev().collect::<Vec<_>>().join("\n"))
    }

    pub fn confirm_tool(&mut self, confirmation_id: &str, state: &State) -> Result<String> {
        let mut confirmations = self.pending_confirmations.lock().unwrap();

        if let Some(tool_call) = confirmations.remove(confirmation_id) {
            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(async { self.execute_tool(&tool_call, state).await })
            })?;

            self.save_message("tool", &format!("{}: {}", tool_call.function.name, result))?;
            Ok(format!("��� Executed: {}\n{}", tool_call.function.name, result))
        } else {
            Ok("Confirmation ID not found or already processed".to_string())
        }
    }

    pub fn deny_tool(&mut self, confirmation_id: &str) -> Result<String> {
        let mut confirmations = self.pending_confirmations.lock().unwrap();

        if confirmations.remove(confirmation_id).is_some() {
            Ok("��� Tool execution denied".to_string())
        } else {
            Ok("Confirmation ID not found".to_string())
        }
    }
}

impl crate::modules::Module for MikoModule {
    fn name(&self) -> &'static str {
        "miko"
    }

    fn init(&mut self) -> Result<()> {
        Ok(())
    }

    fn update(&mut self, state: &mut State) -> Result<()> {
        state.ai = AiState {
            model: self.model.clone(),
            status: "ready".to_string(),
        };
        Ok(())
    }

    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}