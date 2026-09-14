# SamOS

A modular desktop layer built on Hyprland (Arch Linux). Rust daemon collects system metrics and exports JSON, consumed by Eww widgets. Not a Linux distro — a desktop environment layer on Arch + Hyprland.

**Status:** Development - Core daemon + Voice I/O (STT/TTS) + AI Assistant (MIKO) working end-to-end.

## Architecture

```
��─────────────��    ��──────────────��    ��─────────────��    ��────────────��    ��──────────────��
│   Modules   │───��│ Module Mgr   │───��│ Shared State │───��│  Exporter  │───��│  state.json  │
│ (CPU, Mem,  │    │ (1s loop)    │    │ (single src) │    │ (JSON file)│    │  (1s poll)   │
│  Bat, Disk, │    │              │    │ of truth)    │    │            │    │              │
│  Net, Temp, │    │              │    │              │    │            │    │              │
│  Ctrl, WS,  │    │              │    │              │    │            │    │              │
│  AI/Miko)   │    │              │    │              │    │            │    │              │
��─────────────��    └──────────────��    └─────────────��    └────────────��    └──────��───────��
                                                                                    ��
                                                                      ��───────────────────��
                                                                      │   Eww Widgets     │
                                                                      │ (HUD, Launcher,   │
                                                                      │  Control, WS,    │
                                                                      │  Notifications,  │
                                                                      │  AI/MIKO Chat,   │
                                                                      │  Voice I/O)      │
                                                                      └───────────────────��
```

## Current Components

| Component | Status | Description |
|-----------|--------|-------------|
| **samosd** (daemon) | �� Complete | 1s poll daemon, 7 modules, systemd service |
| **Core Modules** | �� Complete | CPU, Memory, Battery, Disk, Network, Temperature, Control, Workspace |
| **Theme Engine** | �� Complete | 6 themes (hud/hacker/elegant/motivation/love/movie), runtime switching |
| **Control Center** | �� Complete | WiFi/BT status, toggles via `samosctl` |
| **Workspace Manager** | �� Complete | hyprctl-based, click-to-switch |
| **AI Assistant (MIKO)** | �� Complete | Local Ollama (qwen2.5:7b), 8 tools, SQLite memory |
| **Voice STT** | �� Complete | whisper.cpp (ggml-base), file + microphone recording |
| **Voice TTS** | �� Complete | piper (en_US-lessac-medium), eSpeak-ng backend |
| **Plugin System** | �� Skeleton | Dynamic `.so` loading, GPU monitor example |
| **Theme Engine** | �� Complete | Runtime SCSS variables, 6 extracted themes |

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

## AI Assistant (MIKO)

**Local-first:** Ollama (qwen2.5:7b) via REST API, no cloud calls.

**Tools (8):**
1. `get_system_state` - Read live metrics
2. `launch_app` - Launch via `.desktop` (confirmation)
3. `switch_workspace` - hyprctl dispatch (confirmation)
4. `set_theme` - Theme switch (confirmation)
5. `add_reminder` / `list_reminders` - SQLite persisted
6. `stt_transcribe` - Audio file → text
7. `tts_speak` - Text → speech + play
8. `stt_record_and_transcribe` - Record + transcribe

**Memory:** SQLite (`~/.local/state/samos/ai.db`) - reminders + conversation summaries (no vector DB yet).

**Confirmation Gate:** Mutating tools require explicit confirm/deny in chat UI.

## Quick Start

```bash
# Build & install
cd ~/Projects/SamOS
cargo build --release -p samosd -p samosctl

# Start daemon (systemd)
systemctl --user restart samosd.service

# Start Eww (separate terminal)
eww daemon && eww open samos_hud_window samos_control_window samos_workspace_window

# Test voice
piper --model ~/.local/share/piper/voices/en_US-lessac-medium.onnx --output_file /tmp/test.wav <<< "Hello MIKO"
aplay /tmp/test.wav

whisper-cli -m ~/.local/share/whisper.cpp/models/ggml-base.bin -f audio.wav -l en -ot txt

# Test MIKO chat (requires Ollama running)
ollama serve &
systemctl --user restart samosd.service
# MIKO available via Eww chat widget or samosctl (when wired)
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
eww open samos_hud_window samos_control_window samos_workspace_window samos_ai_chat_window
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
│   │       ├── modules/    # CPU, Memory, Battery, Disk, Network, Temperature, Control, Workspace, MIKO, Plugin
│   │       ├── state.rs    # Single State struct (source of truth)
│   │       ├── config.rs   # Config struct (theme/monitor/refresh_ms)
│   │       ├── plugin.rs   # Plugin trait + manager (dynamic .so loading)
│   │       └── lib.rs
│   ├── samosd/             # Daemon binary
│   │   └── src/
│   │       ├── main.rs           # Daemon loop
│   │       ├── module_manager.rs # Module registration + loop
│   │       ├── export.rs         # JSON export to state.json
│   │       └── modules/          # Wrapper re-exports
│   ├── samosctl/           # CLI for toggles (wifi, bt, theme, power)
│   ├── samos-ipc/          # Placeholder (future Unix socket IPC)
│   ├── samos-theme/        # Placeholder (theme library)
│   └── samos-modules/      # Placeholder (module library)
├── plugins/
│   └── gpu_plugin/         # Example GPU monitor plugin (.so)
├── ~/.config/samos/        # Config + themes
├── ~/.local/state/samos/   # state.json + ai.db
├── ~/.local/share/         # Models, voices, espeak-ng-data
├── ~/.local/bin/           # piper, whisper-cli, samosctl
��── ~/.config/samosctl/     # samosctl config (future)
```

## Roadmap

| Phase | Target | Status |
|-------|--------|--------|
| Phase 3 | Control Center, Workspace Manager, Theme Engine | �� Done |
| Phase 4a | Theme Engine (runtime) | �� Done |
| Phase 4b | Plugin System | �� Skeleton |
| Phase 4c | AI Assistant (MIKO) | �� Done |
| Phase 4d | Voice I/O (STT/TTS) | �� Done |
| Phase 4e | Automation/Rules Engine | ��� Next |
| Phase 5 | IPC Migration (Unix socket), Packaging, Testing, AUR | ��� Pending |

## Known Issues / Technical Debt

- `samosd` wrapper modules are redundant re-exports (known, tracked)
- JSON file export (1s full rewrite) - planned Unix socket migration
- Battery hardcoded to `/sys/class/power_supply/BAT0`
- Temperature picks first sensor only
- Ollama not in systemd (runs manually)
- MIKO tools `stt_*` and `tts_speak` need Eww widget integration
- No CI/CD, no tests yet
- Theme switching restarts samosd + reloads Eww (intentional for now)

## License

MIT