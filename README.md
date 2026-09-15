# SamOS

A modular desktop layer built on Hyprland (Arch Linux). Rust daemon collects system metrics and exports JSON/Unix socket, consumed by Eww widgets. Not a Linux distro — a desktop environment layer on Arch + Hyprland.

**Status:** Development - Core daemon + Voice I/O (STT/TTS) + AI Assistant (MIKO) + Automation Engine + Audio Visualizer + IPC migration working.

## Architecture

```
┌─────────────┐    ┌──────────────┐    ┌─────────────┐    ┌────────────┐    ┌──────────────┐
│   Modules   │───▶│ Module Mgr   │───▶│ Shared State │───▶│  Exporter  │───▶│  state.json  │
│ (CPU, Mem,  │    │ (1s loop)    │    │ (single src) │    │ (JSON file)│    │  (1s poll)   │
│  Bat, Disk, │    │              │    │ of truth)    │    │            │    │              │
│  Net, Temp, │    │              │    │              │    │            │    │              │
│  Ctrl, WS,  │    │              │    │              │    │            │    │              │
│  AI, Vis,   │    │              │    │              │    │            │    │              │
│  Autom)     │    │              │    │              │    │            │    │              │
└─────────────┘    └──────────────┘    └─────────────┘    └────────────┘    └──────┬───────┘
                                                                                     │
                                       ┌────────────────────────────────────────────┘
                                       │
                                       ▼
                        ┌─────────────────────────────────────────────┐
                        │              IPC Layer                      │
                        │  ┌─────────────┐  ┌─────────────────────┐  │
                        │  │ state.sock  │  │      ai.sock        │  │
                        │  │ (full state)│  │ (AI chat + tools)   │  │
                        │  └─────────────┘  └─────────────────────┘  │
                        └─────────────────────────────────────────────┘
                                       │
                                       ▼
                        ┌─────────────────────────────────────────────┐
                        │             Eww Widgets                     │
                        │ (HUD, Launcher, Control, WS, Notifications, │
                        │  AI/MIKO Chat, Voice I/O, Visualizer,       │
                        │  Automation Rules UI)                       │
                        └─────────────────────────────────────────────┘
```

## Current Components

| Component | Status | Description |
|-----------|--------|-------------|
| **samosd** (daemon) | ✅ Complete | 1s poll daemon, 10 modules, systemd service |
| **Core Modules** | ✅ Complete | CPU, Memory, Battery, Disk, Network, Temperature, Control, Workspace |
| **Theme Engine** | ✅ Complete | 6 themes (hud/hacker/elegant/motivation/love/movie), runtime switching |
| **Control Center** | ✅ Complete | WiFi/BT status, toggles via `samosctl` |
| **Workspace Manager** | ✅ Complete | hyprctl-based, click-to-switch |
| **Audio Visualizer** | ✅ Complete | cava spectrum (32 bars) + playerctl now-playing (MPRIS) |
| **Automation Engine** | ✅ Complete | Rules engine with conditions/actions, validation, security allowlist |
| **AI Assistant (v1)** | ✅ Complete | Request-driven AiModule, Unix socket IPC, 5 tools, SQLite memory |
| **Voice STT** | ✅ Complete | whisper.cpp (ggml-base), file + microphone recording |
| **Voice TTS** | ✅ Complete | piper (en_US-lessac-medium), eSpeak-ng backend |
| **Plugin System** | ✅ Skeleton | Dynamic `.so` loading, GPU monitor example |

## Voice I/O (Phase 4d - Complete)

| Component | Tool | Model | Notes |
|-----------|------|-------|-------|
| **STT** | whisper.cpp | ggml-base.bin (141MB) | File + mic recording via `arecord` |
| **TTS** | piper | en_US-lessac-medium (60MB) | eSpeak-ng phonemizer, 22kHz |
| **Recording** | arecord | S16_LE, 16kHz, mono | 5s default, configurable |

**Voice Commands (via MIKO tools):**
- `stt_transcribe` - Transcribe audio file
- `tts_speak` - Convert text to speech + play
- `stt_record_and_transcribe` - Record mic + transcribe

## AI Assistant (v1 - Complete)

**Backend:** Request-driven `AiModule` in `samos-core`, Unix socket IPC at `~/.local/state/samos/ai.sock`, calls Ollama REST API (`http://localhost:11434`, qwen2.5:7b).

**Tool Set (v1):**
| Tool | Description | Confirmation |
|------|-------------|--------------|
| `get_system_state` | Reads current State (CPU/RAM/battery/etc.) | No (read-only) |
| `launch_app` | Launch via `.desktop` (gtk-launch) | Yes |
| `switch_workspace` | Via hyprctl dispatch | Yes |
| `set_theme` | Writes config.toml, reloads Eww | Yes |
| `add_reminder` / `list_reminders` | SQLite-backed | No |

**Memory:** SQLite (`~/.local/state/samos/ai.db`) - reminders + conversation summaries.

**Confirmation Gate:** Mutating tools require explicit confirm/deny via `ConfirmTool` IPC message.

**IPC Protocol:**
```json
// Request
{"type": "Chat", "message": "Switch theme to hacker", "conversation_id": "abc"}
// Response (confirmation needed)
{"type": "Done", "summary": "CONFIRMATION_REQUIRED:call_xyz"}
// Confirm
{"type": "ConfirmTool", "tool_call_id": "call_xyz", "confirmed": true}
// Final response
{"type": "ToolResult", "tool_call_id": "call_xyz", "result": "Theme set to: hacker"}
```

## Automation Engine (Phase 4e - Complete)

**Rules file:** `~/.config/samos/rules.json` - loaded at startup, hot-reloaded on change.

**Rule structure:**
```json
{
  "id": "unique-id",
  "name": "High CPU Alert",
  "enabled": true,
  "conditions": [
    {"field": "cpu.usage", "operator": "greater_than", "value": 90}
  ],
  "actions": [
    {"type": "notify"},
    {"type": "log", "message": "High CPU detected"}
  ]
}
```

**Supported operators:** `equals`, `not_equals`, `greater_than`, `less_than`, `contains`, `starts_with`, `ends_with`

**Actions:** `notify`, `log`, `run_command` (allowlist), `set_theme`, `set_power_profile`, `toggle_wifi`, `toggle_bluetooth`, `switch_workspace`, `launch_app`, `speak`

**Security:** `run_command` allowlist: `samosctl`, `hyprctl dispatch workspace`, `powerprofilesctl set`, `gtk-launch`, `notify-send`. Denylist blocks: `rm`, `sudo`, `dd`, `mkfs`, `shutdown`, `reboot`, `kill`, shell metacharacters.

## Audio Visualizer (Complete)

- `cava` reads live PipeWire/Pulse audio output, streams 32-bar spectrum (0-100 range) at 60fps
- `playerctl` provides now-playing via MPRIS (title, artist, status: Playing/Paused/Stopped)
- Exported in `state.visualizer` (spectrum + now_playing_*)
- Toggled with `SUPER+M` → `samos_visualizer_window`

## Quick Start

```bash
# Build & install
cd ~/Projects/SamOS
cargo build --release -p samosd -p samosctl

# Start daemon (systemd)
systemctl --user restart samosd.service

# Start Eww (separate terminal)
eww daemon && eww open samos_hud_window samos_control_window samos_workspace_window samos_visualizer_window

# Test voice
piper --model ~/.local/share/piper/voices/en_US-lessac-medium.onnx --output_file /tmp/test.wav <<< "Hello MIKO"
aplay /tmp/test.wav

whisper-cli -m ~/.local/share/whisper.cpp/models/ggml-base.bin -f audio.wav -l en -ot txt

# Test MIKO chat (requires Ollama running)
ollama serve &
systemctl --user restart samosd.service
# Connect to ~/.local/state/samos/ai.sock for AI chat
```

## Service Management

```bash
# Daemon
systemctl --user status samosd.service
systemctl --user restart samosd.service
journalctl --user -u samosd -f

# Ollama
systemctl start ollama
ollama list
ollama pull qwen2.5:7b

# Eww
eww reload
eww open samos_hud_window samos_control_window samos_workspace_window samos_visualizer_window samos_ai_chat_window
eww close samos_control_window samos_workspace_window
```

## Configuration

```toml
# ~/.config/samos/config.toml
theme = "hud"           # hud, hacker, elegant, motivation, love, movie
monitor = "focused"
refresh_ms = 1000
```

Themes live in `~/.config/samos/themes/{name}.toml` (auto-generated from system rice).

Automation rules: `~/.config/samos/rules.json`

## Development

```bash
# Add new module (see AGENTS.md for exact pattern)
# 1. Add struct to state.rs
# 2. Create module in samos-core/src/modules/
# 3. Register in samos-core/src/modules/mod.rs
# 4. Wrapper in samosd/src/modules/
# 5. Register in samosd/src/main.rs
# 6. cargo fmt && cargo build --release -p samosd && systemctl --user restart samosd.service

# Build
cargo build --release -p samosd
cargo build --release -p samosctl

# Test
cargo test
cargo fmt && cargo clippy
```

## Project Structure

```
~/Projects/SamOS/
├── crates/
│   ├── samos-core/         # Core logic, modules, state, config
│   │   └── src/
│   │       ├── modules/    # CPU, Memory, Battery, Disk, Network, Temperature, Control, Workspace, AI, Visualizer, Automation, Plugin, MIKO
│   │       ├── state.rs    # Single State struct (source of truth)
│   │       ├── config.rs   # Config struct (theme/monitor/refresh_ms)
│   │       ├── plugin.rs   # Plugin trait + manager (dynamic .so loading)
│   │       └── lib.rs
│   ├── samosd/             # Daemon binary
│   │   └── src/
│   │       ├── main.rs           # Daemon loop
│   │       ├── module_manager.rs # Module registration + loop
│   │       ├── export.rs         # JSON export to state.json
│   │       ├── ipc.rs            # Unix socket IPC server (state.sock)
│   │       └── modules/          # Wrapper re-exports
│   ├── samosctl/           # CLI for toggles (wifi, bt, theme, power)
├── plugins/
│   └── gpu_plugin/         # Example GPU monitor plugin (.so)
├── ~/.config/samos/        # Config + themes + rules.json
├── ~/.local/state/samos/   # state.json + ai.db + state.sock + ai.sock
├── ~/.local/share/         # Models, voices, espeak-ng-data
├── ~/.local/bin/           # piper, whisper-cli, samosctl
```

## Roadmap

| Phase | Target | Status |
|-------|--------|--------|
| Phase 3 | Control Center, Workspace Manager, Theme Engine | ✅ Done |
| Phase 4a | Theme Engine (runtime) | ✅ Done |
| Phase 4b | Plugin System | ✅ Skeleton |
| Phase 4c | AI Assistant (MIKO v1 - request-driven, Unix socket) | ✅ Done |
| Phase 4d | Voice I/O (STT/TTS) | ✅ Done |
| Phase 4e | Automation/Rules Engine | ✅ Done |
| Phase 4f | Audio Visualizer | ✅ Done |
| **Phase 5** | Full IPC migration (state.sock for all consumers), Packaging, Testing, AUR | 🔄 In Progress |
| Phase 6 | Voice v1.5 (push-to-talk), Eww AI chat panel, Wake-word | ⏳ Pending |

## Known Issues / Technical Debt

- `samosd` wrapper modules are redundant re-exports (known, tracked)
- JSON file export (1s full rewrite) - planned Unix socket migration (state.sock done for full state)
- Battery hardcoded to `/sys/class/power_supply/BAT0`
- Temperature picks first sensor only
- Ollama not in systemd (runs manually)
- MIKO tools `stt_*` and `tts_speak` need Eww widget integration
- No CI/CD, no tests yet
- Theme switching writes config.toml + reloads Eww (intentional for now)
- `process_chat` recursion requires `Box::pin` (async recursion)
- GPU plugin has duplicate definitions (pre-existing, not blocking core)

## License

MIT