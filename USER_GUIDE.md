# SamOS + MIKO user guide

SamOS is now a laptop-first desktop layer. The HUD, music card, MIKO card, controls, and bottom spectrum run on `eDP-1`, below normal application windows. The external display is intentionally left clear.

## Start, stop, and restart

The installed user services are:

- `samosd.service` — collects metrics, writes `~/.local/state/samos/state.json`, and serves local state IPC.
- `samos-ollama.service` — runs Ollama on `127.0.0.1:11434` for MIKO.
- `samos-desktop.service` — starts the Eww surfaces on the laptop display.

Useful commands:

```bash
systemctl --user status samosd samos-ollama samos-desktop
systemctl --user restart samosd
systemctl --user restart samos-ollama
systemctl --user restart samos-desktop
systemctl --user stop samos-desktop
journalctl --user -u samosd -f
journalctl --user -u samos-ollama -f
```

After source changes, build and restart the actual daemon:

```bash
cargo build --release -p samosd -p samosctl
systemctl --user restart samosd
```

To stop the whole SamOS layer:

```bash
systemctl --user stop samos-desktop samos-ollama samosd
```

The daemon now handles SIGTERM/SIGINT and closes its listeners, joins assistant workers, stops/reaps its Cava child, and shuts down plugins. During restart, an active chat can return `SamOS is shutting down` or a closed-socket error; retry once the service is ready. Pending approvals are lost on restart. Cancellation does not undo an action that already completed.

Assistant subprocesses are cancelled as owned process groups. Optional module startup/update failures are logged instead of aborting every module; check `journalctl --user -u samosd` for `[module:...]` diagnostics. This does not guarantee bounded shutdown for a stuck third-party plugin or legacy automation action still executing inside a metrics update; automation hardening is the next phase.

## What the desktop shows

Top left shows the hostname, live CPU, memory, battery, temperature, and workspaces. Click a workspace number to switch it. Top right shows the clock and Wi-Fi, Bluetooth, and power-profile controls. Bottom left shows the current player, artist, playback state, previous/play-next controls. Bottom right is MIKO. The bottom line is the live audio spectrum; it follows PipeWire/Pulse audio through Cava and becomes quiet when no audio is available.

Normal applications appear above these surfaces. The panels do not reserve screen space and do not cover application windows. If a panel is misplaced after changing monitor topology, restart `samos-desktop`.

## MIKO text chat

Click `Ask miko… ↗`. A small input dialog opens. Type naturally and press Enter. MIKO can answer conversation questions, read live metrics, save/list reminders, and suggest actions.

Actions that affect the desktop always return an exact proposal first. The card shows the tool name and arguments. Choose Confirm or Deny. A confirmation expires after five minutes and is tied to that conversation, so an old or copied ID cannot approve another action.

Saving a reminder also requires confirmation. Review its exact text and due timestamp: the model can misunderstand your request. Listing reminders is read-only. Reminders currently store information; automatic delivery and calendar scheduling are subsequent phases.

The terminal client is also available:

```bash
python3 scripts/miko.py chat "What is using my RAM?"
python3 scripts/miko.py chat "Launch Firefox"
python3 scripts/miko.py confirm <id>
python3 scripts/miko.py deny <id>
python3 scripts/miko.py voice
```

The model is configurable with `SAMOS_MODEL`; the installed default is `qwen2.5:1.5b`, selected to fit this laptop's memory. To use another locally installed model, set it in the systemd override and restart:

```bash
systemctl --user edit samosd
# [Service]
# Environment=SAMOS_MODEL=qwen2.5:7b
systemctl --user daemon-reload
systemctl --user restart samosd samos-ollama
```

The model must be pulled first with `ollama pull <model>`. A larger model may exhaust memory on this machine.

## Conversation history

Completed exchanges are saved locally in `~/.local/state/samos/ai.db`, including tool-call results. They reload after a daemon restart. This no longer depends on a second model request for a summary. Existing summaries remain as fallback for conversations without saved exchanges.

History keeps at most 128 conversations. Each conversation normally retains up to 32 messages and 16 KiB, trimming only complete older user turns. A single larger turn is retained intact up to 64 KiB; an oversized exchange produces a storage error and leaves previously saved history intact. System prompts, current metrics in the system prompt, and pending approvals are not saved. Tool results may include metrics or other data read during the exchange.

```bash
python3 scripts/miko.py --conversation terminal history
python3 scripts/miko.py --conversation terminal forget --yes
python3 scripts/miko.py --conversation miko history
```

`forget --yes` explicitly confirms deletion of the named conversation's history and legacy summary. It does not remove reminders or the profile, and it refuses while an action is pending. This is logical deletion, not secure erasure of database pages/backups. Review and deny a pending proposal first. Pending approvals are deliberately lost on daemon restart; ask for the action again if needed.

Terminal voice defaults to the durable `miko` thread; terminal text defaults to `terminal`. Use `--conversation miko` for confirmation/denial or text follow-ups to a terminal voice exchange. Explicit conversation IDs are preserved, including the desktop client's existing `desktop` thread. History inspection reads saved completed turns only.

## Profile configuration

An optional `~/.config/samos/profile.toml` shapes conversational tone. Start from `config/profile.example.toml`, or create:

```toml
name = "Sam"
tone = "Be warm and direct. Keep answers short unless I ask for detail."

[shortcuts]
github = "https://github.com/notifications"
```

New chat/voice turns reload the file without restarting the daemon. Missing files use defaults; malformed edits keep the last valid profile for the current daemon process and log a diagnostic under `[ai-profile]`. Deleting the file restores defaults. Unknown fields are rejected. Limits: 8 KiB file, 80 bytes for name, 600 bytes for tone, eight shortcuts with 40-byte keys and 512-byte HTTP(S) URLs. URLs cannot contain credentials, spaces or control characters.

Shortcuts currently supply conversational context; browser opening is not implemented yet. Tone and profile text cannot turn off tool confirmation. No profile is installed or overwritten automatically.

## Voice operation

Click Mic in the MIKO card or run `python3 scripts/miko.py voice`. Recording is limited to 1–30 seconds and uses the local Whisper model. TTS uses the local Piper voice and plays through `aplay`.

For a five-second terminal recording followed by a spoken answer, use:

```bash
python3 scripts/miko.py voice --speak
```

This explicitly enables reply playback for that voice turn, including its continuation after a tool confirmation. Use `--conversation miko` when confirming a terminal voice proposal. Other tool actions still need their own approval. Plain `voice`, the HUD Mic button, and text chat do not automatically speak answers. If reply playback fails, the terminal response retains the answer and includes `speech_error`.

The MIKO badge follows live daemon state: LISTENING during capture, TRANSCRIBING during Whisper, PREPARING VOICE during Piper, and SPEAKING during playback. These fields update with the normal metrics interval, so very short phases may not be visible. Recording, transcription, and speech share one assistant audio owner. A competing request waits at most two seconds before returning an audio-busy error; retry after the current operation ends. This coordinates MIKO's audio operations, not unrelated applications or the legacy automation speech path.

Failures release ownership and clear the flags. Stale exported state resets the frontend's audio flags rather than displaying a stuck microphone/speaker indicator. Service shutdown now cancels assistant inference and audio subprocesses and joins request workers; wake-word listening is not enabled yet.

If speech fails, check the dependencies and runtime paths:

```bash
ls ~/.local/bin/whisper-cli ~/.local/bin/piper
ls ~/.local/share/whisper.cpp/models/ggml-base.bin
ls ~/.local/share/piper/voices/en_US-lessac-medium.onnx
ldd ~/.local/bin/whisper-cli | grep 'not found' || true
```

The installer includes a self-contained Whisper executable and backs up the previous one.

## Music controls

The player card uses `playerctl` and MPRIS. Start music in Spotify, Firefox, VLC, or another MPRIS player. The now-playing title/artist and spectrum update automatically. Previous, play/pause, and next use literal `playerctl` arguments.

## Themes and configuration

Configuration lives at `~/.config/samos/config.toml`:

```toml
theme = "hud"
monitor = "eDP-1"
refresh_ms = 1000
```

Theme files live in `~/.config/samos/themes/`. Use the CLI:

```bash
samosctl theme-list
samosctl theme-get
samosctl theme-set hacker
```

Theme changes preserve `monitor`, `refresh_ms`, and unknown user settings. The daemon reloads valid settings during its loop; no manual daemon restart is required.

## Automation

Rules live in `~/.config/samos/rules.json` and currently load on startup. The current implementation can repeat actions while conditions remain true and still has shell-based command/speech paths. MIKO does not create rules. Validation, literal argument execution, hot reload and per-rule transition handling are planned in phase B2 of [DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md); earlier claims that these safeguards were already complete were incorrect.

## Safe installation and restore

Install from the repository after building:

```bash
cargo build --release -p samosd -p samosctl
./install/install.sh --whisper-binary /tmp/samos-whisper-build/bin/whisper-cli
```

Installation copies only SamOS-managed files. It does not mirror or delete your existing `~/.config/eww` or `~/.config/samos` trees. Every overwritten file is saved under `~/.local/state/samos/backups/<timestamp>/` with a manifest.

Restore a backup:

```bash
python3 install/restore.py ~/.local/state/samos/backups/<timestamp>
systemctl --user daemon-reload
systemctl --user restart samosd samos-desktop samos-ollama
```

## Troubleshooting

If MIKO says Ollama is unavailable, run `systemctl --user status samos-ollama` and `ollama list`. If the model was killed by the OOM killer, use the smaller installed model and check `free -h`; avoid running a 7B model while swap is exhausted.

If the widgets are not visible, run `systemctl --user restart samos-desktop`, then check `eww --config ~/.config/eww-samos active-windows` and `hyprctl -j layers`. The expected namespace is `samos-desktop` and the expected monitor is `eDP-1`.

If the bottom spectrum is flat, check `playerctl status`, `cava -v`, and that the audio source is PipeWire/Pulse. The rest of the desktop remains useful when no audio player is running.

If the daemon fails, inspect `journalctl --user -u samosd -n 100 --no-pager`. It keeps the last valid configuration, isolates broken optional plugins/modules where possible, and removes its sockets during a clean stop.

## Verification commands

```bash
cargo test --workspace
python3 -m unittest discover -s tests -v
cargo fmt --all -- --check
python3 scripts/miko.py chat "Report my current CPU usage in one sentence."
```

The backend tests cover configuration preservation, MIKO tool parsing and confirmation order, live-state reads, voice-tool registration, shell-fragment rejection, automation transition triggers, rule validation, and safe rule reload.
