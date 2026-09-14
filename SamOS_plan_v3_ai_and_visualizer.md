# SamOS Plan v3 — AI subsystem (MIKO-informed, scoped down) + audio visualizer

This supersedes Phase 4c/4d in `SamOS_plan_v2_reviewed.md`. Everything else in v2 stands unchanged. This file adds one new completed item (F) and one revised future item (Phase 4c/4d replacement, Section G).

---

## F. Audio visualizer — done, documented for context

Already built and working, HUD-window-scoped for now (theme-gating to be added once 4a's `theme` state field exists — see v2 directive B).

- `cava` reads live PipeWire/Pulse audio output and streams a spectrum per frame — reacts to whatever's actually playing, not a canned animation.
- `playerctl` provides the now-playing strip via MPRIS (works with Spotify, browsers, most native players).
- Files: `~/.config/eww/samos_visualizer.yuck`, `.scss`, `~/.config/eww/scripts/cava_stream.sh`, `cava_samos.conf`, `nowplaying.sh`.
- Toggled with `SUPER+M`, window `samos_visualizer_window`, mirrored-bar neon gradient rendering.
- **Not yet done**: gating this window to only appear in HUD/hacker theme mode, per directive B. Do this as part of 4a once the theme engine's visibility mechanism is decided — don't build a second, separate gating mechanism just for this widget.

---

## G. AI subsystem — replaces Phase 4c/4d from v2

### Design philosophy (kept from MIKO)

- **Fully local.** Ollama running a local model, no cloud API calls, no telemetry.
- **LLM never executes anything directly.** It requests a tool call by name + args; a deterministic Rust function executes it and returns a result. This mirrors the existing `Module` trait pattern — a `Tool` trait is a natural sibling, not a new paradigm.
- **Memory is compressed, not raw.** Store summaries of interactions, not full transcripts. Start as flat SQLite rows (`timestamp, summary`) — no vector DB yet.
- **Event-driven.** The AI module only runs inference when invoked (chat input, voice trigger, or a rule from the future automation phase) — it is not a background reasoning loop.
- **Confirmation gate on sensitive actions.** Any tool that modifies state outside the daemon's own data (launching apps, killing processes, sending anything) requires an explicit confirm step in the Eww UI before executing. Read-only tools (query state, search notes) don't need confirmation.

### What ships in v1 (MVP)

**Backend**
- `AiModule` in `samos-core`, following the existing `Module` trait shape for consistency, but note: unlike other modules it's request-driven (invoked on user input) rather than polled every tick — this is a legitimate, intentional exception to the polling pattern, not a violation of it.
- Calls Ollama's local REST API directly (`reqwest`, `http://localhost:11434`) — no separate Python/FastAPI service to run or supervise. One fewer moving part than MIKO's design.
- Default model: `qwen2.5:7b` (MIKO's own pick, reasonable default for a 7B-class local model — revisit only if response quality or latency is actually a problem in practice, don't pre-optimize).

**Tool set (v1 — deliberately small)**
Only tools that operate on things SamOS already has:
| Tool | Does |
|---|---|
| `get_system_state` | Reads current `State` (CPU/RAM/battery/etc.) — read-only |
| `launch_app` | Reuses the existing launcher's `.desktop` resolution — needs confirmation |
| `switch_workspace` | Via `hyprctl` — needs confirmation |
| `set_theme` | Once 4a exists — needs confirmation |
| `add_reminder` / `list_reminders` | SQLite-backed, simplest possible schema (`id, text, due_at`) |

Explicitly **not** in v1: email, calendar, PDF/codebase RAG, semantic file search, clipboard tools. These are real MIKO ideas worth having eventually, but each is its own scoped phase later, added only once the tool-call loop above is proven working with the small set.

**Memory (v1)**
- SQLite table for reminders/operational data (already needed for `add_reminder` above — don't stand up a separate memory subsystem, this table is the memory subsystem for v1).
- SQLite table for conversation summaries: after each exchange, ask the model for a one-line summary, store it. No vector DB, no embeddings, no ChromaDB in v1 — if semantic search over past conversations becomes a real need later, that's a follow-up phase with its own scoping, not a v1 assumption.

**Frontend**
- New Eww widget: text-input chat panel, HUD/hacker-mode-scoped like the visualizer. Streaming token display if Ollama's streaming response is easy to wire into Eww's update model; if not, non-streaming for v1 is fine — don't block the whole feature on getting streaming UX right first.
- Confirmation prompts for gated tools render as a simple accept/deny inline in the same panel.

**IPC note**
A chat panel polling a JSON file every 1s produces visibly laggy responses. This is the one place where pulling Phase 5.1 (JSON file → Unix socket) forward, as already flagged in v2, actually matters — do the socket migration for the AI module's communication specifically, even if the rest of the state pipeline stays on JSON polling for now. Don't do a full system-wide IPC migration just to unblock this; scope the socket to AI chat traffic only if that's faster to ship.

### Voice (v1.5 — only after the text loop above is solid)

- Push-to-talk only. **No wake-word listener in v1** — MIKO's `openWakeWord` requirement is a real feature but an always-on audio listener is a meaningfully bigger trust/privacy surface and a separate integration; add it later if push-to-talk proves annoying in practice.
- `whisper.cpp` for STT, `piper` for TTS — both are fine choices from MIKO's stack, keep them.
- PipeWire audio routing (same audio system the visualizer already uses — no new audio dependency).

### Explicitly deferred, not forgotten

Everything in this list is a legitimate MIKO idea, just not v1 scope: local RAG / knowledge engine over files and code, email drafting and inbox search, calendar integration and conflict detection, recurring workflow automation, the observability/focus-tracking layer (opt-in only, if ever), wake-word activation.

---

## Why this split matters for opencode specifically

If handed the full MIKO doc as-is, the natural failure mode is building the Tauri dashboard, ChromaDB, wake-word listener, and five tool categories simultaneously — which is exactly the kind of multi-week unreviewable batch that AGENTS.md's "one feature at a time" rule exists to prevent. This doc's v1 scope is intentionally small enough to build, compile, and test in the same incremental style as every module so far (CPU, Memory, Battery... AI tool call, confirm, execute). Expand from here only after this loop is demonstrably working.
