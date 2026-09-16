# SamOS desktop completion plan

## Target experience

Separate desktop surfaces behind application windows, with no reserved screen area. A restrained graphite/ice-blue palette, translucent rounded cards, readable typography without private-use icon glyphs, and clear empty/error states. Top left: identity, live system metrics, workspaces. Top right: clock and quick controls. Bottom left: now-playing card. Bottom right: MIKO chat and explicit tool confirmation. Along the bottom edge: a real audio-reactive spectrum. Applications cover the desktop naturally. Chat takes keyboard focus only when clicked.

Display policy: use the user's selected displays; default to all connected displays. Resolve monitor names at startup rather than hardcoding numeric monitor zero. Do not replace wallpaper, application layout, or existing compositor keybindings. Music visualization follows actual playback, with a quiet idle state instead of invented activity.

## Constraints

Preserve Modules → Module Manager → State → JSON export → Eww. Keep daemon wrappers and JSON export. Frontend assets are maintained separately from the Rust build and installed explicitly with backups. No new Rust dependencies without demonstrated need. One feature per implementation/compile/runtime-check cycle. Preserve pre-existing working-tree changes.

## Implementation order and acceptance checks

1. **Restore a safe foundation.** Fix GPU plugin compilation and bounded sampling; replace destructive installer behavior with explicit, backed-up file installation. Verify workspace builds. Audit items 1–2, 39, 43.
2. **Desktop frontend.** Create self-contained Eww configuration, safe frontend bridge, corner surfaces, bottom spectrum, media controls, display-aware startup, and service integration. Validate Eww parsing and GTK styling; inspect a real screenshot and Hyprland layer placement. Existing external configuration must be backed up before deployment. Keep this installation separate from Cargo. Items 32–34, 43.
3. **Control/config correctness.** Preserve unknown config keys and settings when switching themes; validate identifiers; remove shell interpolation; report subprocess errors; reload daemon config without restarting for theme changes. Verify with temporary config and mock commands. Items 6, 18, 20–22.
4. **MIKO text loop.** Native Ollama schema, live metrics read through the existing state file, bounded requests, complete conversation/tool history, per-conversation confirmations with exact arguments and expiry, persisted summaries, safe process execution. Connect UI chat, error, busy, confirm and deny states. Use a mock Ollama server for deterministic tests, then check local Ollama availability. Items 3, 7–15, 19.
5. **Voice.** Wire the existing STT/TTS operations into the active AI tool registry, resolve home paths, stream text through stdin, bound recording duration, check exit status, and clean temporary files. Expose deliberate push-to-talk and speech actions; do not introduce wake-word monitoring. Items 5, 14–17.
6. **Automation.** Validate complete field/operator/action combinations; execute restricted commands without a shell; trigger on each rule's transition; reload valid rules safely while retaining last good state on errors; reject duplicate IDs. Populate system metadata before updates. Regression-test repeats, typos, reloads, and failed actions. Items 4–5, 23–28.
7. **Runtime reliability and metrics.** Graceful service stop and bounded child processes, listener ownership, plugin error isolation, accurate rates/percentages, real optional-state handling. Verify service restarts and live JSON. Items 28–40.
8. **Release checks and documentation.** Workspace tests, formatting, relevant lint checks, installer backup/restore test, frontend syntax/runtime logs, actual desktop screenshot, monitor coverage, AI protocol tests. Update audit dispositions honestly and provide exact remaining environmental limits. Add focused CI; stop adding build output to future changes. Items 41–44.

## Definition of done

- Safe install/restore and a successful workspace build.
- Desktop panels render at the requested corners and remain below normal apps.
- Real audio drives the bottom visualization smoothly without increasing metric polling to frame rate.
- Controls accurately report failures; theme changes preserve settings.
- Text chat and confirmations work through the real IPC protocol, with no arbitrary shell execution from tool arguments.
- Voice tools are wired, with dependency failures surfaced clearly.
- Automation does not repeat unchanged conditions or execute shell fragments.
- Tests cover the substantive backend fixes, and service/frontend validation results are recorded.

## Progress

- Planning: complete; implementation started.
- Initial audit: `REPO_AUDIT.md` (44 findings; preserve as a baseline and annotate dispositions at completion).
