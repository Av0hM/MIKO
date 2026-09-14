# SamOS Voice I/O & AI Assistant Setup Guide

## Overview

This document details the complete Voice I/O (Phase 4d) and AI Assistant (MIKO) implementation for SamOS.

## Voice I/O Components

### STT (Speech-to-Text) — whisper.cpp

**Model:** `~/.local/share/whisper.cpp/models/ggml-base.bin` (141MB, ggml-base)

**CLI Usage:**
```bash
# Transcribe audio file
whisper-cli -m ~/.local/share/whisper.cpp/models/ggml-base.bin -f audio.wav -l en -ot txt

# Record + transcribe (5 seconds)
arecord -f S16_LE -c 1 -r 16000 -d 5 /tmp/record.wav
whisper-cli -m ~/.local/share/whisper.cpp/models/ggml-base.bin -f /tmp/record.wav -l en -ot txt
```

**MIKO Tools:**
- `stt_transcribe` — Transcribe audio file (args: `audio_file`, `language`)
- `stt_record_and_transcribe` — Record mic + transcribe (args: `duration_seconds`, `language`)

### TTS (Text-to-Speech) — piper

**Model:** `~/.local/share/piper/voices/en_US-lessac-medium.onnx` (60MB)
**Config:** `~/.local/share/piper/voices/en_US-lessac-medium.onnx.json`

**CLI Usage:**
```bash
ESPEAK_DATA_PATH=~/.local/share/espeak-ng-data \
LD_LIBRARY_PATH=~/.local/lib:$LD_LIBRARY_PATH \
piper --model ~/.local/share/piper/voices/en_US-lessac-medium.onnx \
      --output_file /tmp/output.wav <<< "text to speak"
aplay /tmp/output.wav
```

**MIKO Tool:** `tts_speak` — args: `text`, `voice` (default: `en_US-lessac-medium`)

## AI Assistant (MIKO) — 8 Tools

### Core Tools (4)
1. **`get_system_state`** — Read live metrics (read-only)
2. **`launch_app`** — Launch via `.desktop` (confirmation required)
3. **`switch_workspace`** — hyprctl dispatch (confirmation required)
4. **`set_theme`** — Theme switch (confirmation required)

### Reminders (2)
5. **`add_reminder`** — args: `text`, `due_at` (optional unix timestamp)
6. **`list_reminders`** — No args

### Voice (4)
7. **`stt_transcribe`** — Audio file → text (args: `audio_file`, `language`)
8. **`tts_speak`** — Text → speech + play (args: `text`, `voice`)
9. **`stt_record_and_transcribe`** — Record mic + transcribe (args: `duration_seconds`, `language`)

### Confirmation Gate
Mutating tools (`launch_app`, `switch_workspace`, `set_theme`) require explicit confirm/deny in chat UI.

## Runtime Environment

```bash
# STT
whisper-cli -m ~/.local/share/whisper.cpp/models/ggml-base.bin -f audio.wav -l en -ot txt

# TTS
ESPEAK_DATA_PATH=~/.local/share/espeak-ng-data \
LD_LIBRARY_PATH=~/.local/lib:$LD_LIBRARY_PATH \
piper --model ~/.local/share/piper/voices/en_US-lessac-medium.onnx \
      --output_file /tmp/output.wav <<< "text"
aplay /tmp/output.wav

# Record + transcribe
arecord -f S16_LE -c 1 -r 16000 -d 5 /tmp/record.wav
whisper-cli -m ~/.local/share/whisper.cpp/models/ggml-base.bin -f /tmp/record.wav -l en -ot txt
```

## MIKO Module Integration

The MIKO module (`crates/samos-core/src/modules/miko.rs`) registers 10 tools:
- 6 core tools (4 core + 2 reminders)
- 4 voice tools (STT/TTS)

**Model:** `qwen2.5:7b` via Ollama REST API (`http://localhost:11434`)
**Memory:** SQLite (`~/.local/state/samos/ai.db`) — reminders + conversation summaries
**Confirmation Gate:** Mutating tools require explicit confirm/deny

## Ollama Setup

```bash
# Install
sudo pacman -S ollama
systemctl start ollama
ollama pull qwen2.5:7b

# Verify
ollama list
curl http://localhost:11434/api/tags
```

## Runtime Paths

| Component | Path |
|-----------|------|
| whisper model | `~/.local/share/whisper.cpp/models/ggml-base.bin` |
| piper model | `~/.local/share/piper/voices/en_US-lessac-medium.onnx` |
| piper config | `~/.local/share/piper/voices/en_US-lessac-medium.onnx.json` |
| espeak-ng data | `~/.local/share/espeak-ng-data/` |
| piper binary | `~/.local/bin/piper` |
| whisper binary | `~/.local/bin/whisper-cli` |
| Libraries | `~/.local/lib/` (libespeak-ng, libonnxruntime, libpiper_phonemize) |
| AI database | `~/.local/state/samos/ai.db` |

## MIKO Tools Reference

| Tool | Args | Confirmation |
|------|------|--------------|
| `get_system_state` | none | No |
| `launch_app` | `app_name` | Yes |
| `switch_workspace` | `workspace_id` | Yes |
| `set_theme` | `theme` | Yes |
| `add_reminder` | `text`, `due_at?` | No |
| `list_reminders` | none | No |
| `stt_transcribe` | `audio_file`, `language?` | No |
| `tts_speak` | `text`, `voice?` | No |
| `stt_record_and_transcribe` | `duration_seconds?`, `language?` | No |

## Testing Commands

```bash
# Test TTS
ESPEAK_DATA_PATH=~/.local/share/espeak-ng-data \
LD_LIBRARY_PATH=~/.local/lib:$LD_LIBRARY_PATH \
piper --model ~/.local/share/piper/voices/en_US-lessac-medium.onnx \
      --output_file /tmp/test.wav <<< "Hello, MIKO"
aplay /tmp/test.wav

# Test STT
piper --model ~/.local/share/piper/voices/en_US-lessac-medium.onnx \
      --output_file /tmp/stt_test.wav <<< "Testing STT"
whisper-cli -m ~/.local/share/whisper.cpp/models/ggml-base.bin \
            -f /tmp/stt_test.wav -l en -ot txt

# Test recording + STT
arecord -f S16_LE -c 1 -r 16000 -d 5 /tmp/record.wav
whisper-cli -m ~/.local/share/whisper.cpp/models/ggml-base.bin \
            -f /tmp/record.wav -l en -ot txt
```

## Ollama Setup

```bash
# Install & start
systemctl start ollama
ollama pull qwen2.5:7b

# Verify
ollama list
curl http://localhost:11434/api/tags
```