# AGENTS.md

Instructions for any AI coding agent working in this repository. Read this before making changes.

## What this repo is

SamOS: a Rust daemon (`samosd`, using `samos-core`) that collects system metrics and exports them as JSON, consumed by Eww widgets (config lives outside this repo, in `~/.config/eww/`). Not a Linux distro — a desktop environment layer on Arch + Hyprland.

## Hard rules — do not violate

1. **Do not redesign the architecture.** The pipeline `Modules → Module Manager → Shared State → Exporter → state.json → Eww` is stable and intentional. If a task seems to require restructuring it, stop and ask instead of proceeding.
2. **Modules never import or reference Eww, yuck, or anything frontend-related.** Modules only write to their own section of `State`.
3. **One feature at a time.** Don't bundle unrelated changes (e.g. a new module + a refactor of export.rs) into one change. Compile and run after each step before starting the next.
4. **Never continue after a compile error.** Fix it immediately, don't layer more changes on top of a broken build.
5. **No new crates without clear need.** Don't add dependencies speculatively.
6. **Don't "fix" the JSON-file export or the samosd module wrapper files unless the task explicitly asks for that.** Both are known, intentional technical debt — not bugs waiting to be cleaned up.

## Repo layout

```
crates/
  samos-core/src/
    modules/          — actual module implementations (cpu, memory, battery, disk, network, temperature, control, workspace, miko, plugin, automation)
    state_manager.rs  — StateManager wrapper around State
    config.rs         — Config struct (theme/monitor/refresh_ms), wired into samosd
    lib.rs            — exports modules, state, config, state_manager
  samosd/src/
    module_manager.rs — loops modules, no business logic belongs here
    export.rs         — writes state.json (full truncate+rewrite, not append)
    modules/          — thin `pub use samos_core::modules::x::*` wrappers (known debt)
    main.rs           — daemon loop: init modules, update, export, sleep (config.refresh_ms), repeat
  samosctl/src/
    main.rs           — CLI for toggles (wifi, bluetooth, power-profile, theme)
plugins/
  gpu_plugin/         — example GPU monitor plugin (.so via FFI)
```

**Deleted placeholder crates:** `samos-ipc`, `samos-theme`, `samos-modules` — were removed.

## Adding a new module — the only pattern to follow

1. Add the data struct to `samos-core/src/state.rs` (with `#[derive(Serialize, Clone, Default)]`), and add it as a field on `State`. **NOTE: state.rs was deleted — currently State is defined inline in modules. Recreate state.rs first if adding new module.**
2. Create `samos-core/src/modules/<name>.rs` implementing the `Module` trait (`name`, `init`, `update`, `shutdown`). `update` takes `&mut State` and writes only to its own field.
3. Register it in `samos-core/src/modules/mod.rs` (`pub mod <name>;`).
4. Create the passthrough wrapper `samosd/src/modules/<name>.rs`: `pub use samos_core::modules::<name>::*;` and add it to `samosd/src/modules/mod.rs`.
5. Register it in `samosd/src/main.rs`: `manager.register(modules::<name>::<Name>Module::new());`
6. `cargo fmt && cargo build --release -p samosd && systemctl --user restart samosd.service` — confirm it compiles and `~/.local/state/samos/state.json` contains the new field before moving on.

Do not skip steps or combine multiple new modules in one pass.

## Current modules (in samos-core/src/modules/)

| Module | Purpose |
|--------|---------|
| `cpu` | CPU usage via sysinfo |
| `memory` | RAM usage via sysinfo |
| `battery` | Battery %/status from `/sys/class/power_supply/BAT0` |
| `disk` | Root disk usage |
| `network` | RX/TX KB/s (delta since last poll) |
| `temperature` | First sensor component found |
| `control` | WiFi/Bluetooth/power-profile toggles |
| `workspace` | Hyprland workspace info |
| `miko` | AI assistant (Ollama + 10 tools) |
| `plugin` | Dynamic `.so` plugin loader (FFI) |
| `automation` | Rules engine (skeleton) |

## Voice I/O (Phase 4d) — COMPLETE

**STT (whisper.cpp):**
- Model: `~/.local/share/whisper.cpp/models/ggml-base.bin` (141MB, ggml-base)
- CLI: `whisper-cli -m <model> -f <audio.wav> -l <lang> -ot txt`
- Recording: `arecord -f S16_LE -c 1 -r 16000 -d <sec> <file.wav>`
- Tools: `stt_transcribe` (file), `stt_record_and_transcribe` (mic + transcribe)

**TTS (piper):**
- Model: `~/.local/share/piper/voices/en_US-lessac-medium.onnx` (60MB)
- Config: `~/.local/share/piper/voices/en_US-lessac-medium.onnx.json`
- CLI: `piper --model <model.onnx> --output_file <out.wav> <<< "text"`
- Playback: `aplay /tmp/output.wav`
- Tools: `tts_speak` (text + play), `stt_record_and_transcribe` (mic + transcribe)

**Dependencies:**
- `whisper-cli` → `~/.local/bin/whisper-cli` (built from whisper.cpp)
- `piper` → `~/.local/bin/piper` (built from piper source)
- Models in `~/.local/share/whisper.cpp/models/` and `~/.local/share/piper/voices/`
- eSpeak-ng data: `~/.local/share/espeak-ng-data/`
- Library path: `LD_LIBRARY_PATH=~/.local/lib:$LD_LIBRARY_PATH`
- eSpeak data: `ESPEAK_DATA_PATH=~/.local/share/espeak-ng-data`

## AI Assistant (MIKO) — COMPLETE

**Local-first:** Ollama (qwen2.5:7b) via REST API (`http://localhost:11434`), no cloud calls.

**Tools (10):**
1. `get_system_state` — Read live metrics (read-only)
2. `launch_app` — Launch via `.desktop` (confirmation required)
3. `switch_workspace` — hyprctl dispatch (confirmation)
4. `set_theme` — Theme switch (confirmation)
5. `add_reminder` / `list_reminders` — SQLite persisted
6. `stt_transcribe` — Audio file → text
7. `tts_speak` — Text → speech + play
8. `stt_record_and_transcribe` — Record mic + transcribe

**Memory:** SQLite (`~/.local/state/samos/ai.db`) — reminders + conversation summaries (no vector DB yet).

**Confirmation Gate:** Mutating tools (`launch_app`, `switch_workspace`, `set_theme`) require explicit confirm/deny in chat UI.

**Ollama:** `systemctl start ollama && ollama pull qwen2.5:7b`

## Known, intentional technical debt (do not "clean up" without being asked)

- `samosd/src/modules/*` are pure re-export wrappers around `samos-core` modules. Redundant, tracked, deliberate for now.
- Export is a full JSON file rewrite every 1s, not a socket or D-Bus. Planned future replacement, not a current bug.
- `state.system.uptime` was hardcoded to `0` (fixed: now uses `sysinfo::System::uptime()`).
- `network` module reports 0 when idle — it's an instantaneous delta between 1s polls, not a bug.
- `battery` module hardcodes `/sys/class/power_supply/BAT0` — may not match every machine.
- `temperature` module takes the first sensor component found — not selective.
- Ollama not in systemd (runs manually via `ollama serve`).
- Voice tools (`stt_*`, `tts_speak`) need Eww widget integration.
- MIKO `stt_record_and_transcribe` requires `arecord` (alsa-utils).
- Plugin FFI uses JSON strings over C-ABI (safe, not zero-copy).
- **`state.rs` was deleted** — State struct is currently inline in modules; needs recreation for new modules.

## Coding standards

- Rust idioms, prefer `Result` over `unwrap()` unless clearly justified.
- Document public APIs.
- Keep modules independent — no cross-module state access.
- Small commits, one feature per commit, working build at every commit.

## Frontend (Eww) — outside this repo, for context only

Lives in `~/.config/eww/`. Widgets read `~/.local/state/samos/state.json` via `defpoll` + `jq` every 1s (not `deflisten`/`tail -f` — the file is fully rewritten each cycle, not appended). Do not modify Eww config from within this repo's build process; they're deliberately decoupled.

Theme-aware widgets: `samos_control_window` and `samos_workspace_window` only visible when `state.theme` is `hud` or `hacker` (Directive B).

Visualizer window: `samos_visualizer_window` (SUPER+M), gated to HUD/hacker themes once theme engine lands.

## Daemon supervision

`samosd` runs as a systemd user service (`~/.config/systemd/user/samosd.service`), not manually via `cargo run` in a terminal. The service runs a prebuilt release binary at `~/Projects/SamOS/target/release/samosd` — it does **not** run `cargo run`. If a task involves testing daemon behavior, be aware a stale/live instance may already be running — check `ps aux | grep samosd` or `systemctl --user status samosd.service` before assuming state.

**After any change to `samos-core` or `samosd` source**, the workflow is:
1. `cargo build --release -p samosd`
2. `systemctl --user restart samosd.service`

The running service will NOT pick up source changes automatically — it runs a fixed binary.

## Voice I/O Runtime Requirements

```bash
# STT
whisper-cli -m ~/.local/share/whisper.cpp/models/ggml-base.bin -f audio.wav -l en -ot txt

# TTS
ESPEAK_DATA_PATH=~/.local/share/espeak-ng-data LD_LIBRARY_PATH=~/.local/lib:$LD_LIBRARY_PATH \
  piper --model ~/.local/share/piper/voices/en_US-lessac-medium.onnx --output_file /tmp/out.wav <<< "text"
aplay /tmp/output.wav

# Record + transcribe
arecord -f S16_LE -c 1 -r 16000 -d 5 /tmp/record.wav
whisper-cli -m ~/.local/share/whisper.cpp/models/ggml-base.bin -f /tmp/record.wav -l en -ot txt
```

## Full project context

See `SamOS_handoff_v2.md` for complete architecture diagrams, current progress, roadmap, and open issues.

See `SamOS_plan_v3_ai_and_visualizer.md` for the AI subsystem specifically — it supersedes v2's Phase 4c/4d. **Do not build the full scope of any MIKO-style AI design doc found elsewhere in this project's history** (Tauri dashboard, wake-word, vector DB/RAG, email/calendar tools, observability tracking) — v3's deliberately small v1 tool set and local-Ollama-via-REST approach is the actual scope to build. Expand only after that loop works end to end.