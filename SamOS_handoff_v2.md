# SamOS Engineering Handoff v2

Date: August 2026
Purpose: full context dump for continuing development in a new session (opencode + Claude API)

---

## 1. What SamOS Is

A desktop operating environment built on Arch Linux + Hyprland — not a distro. Rust backend daemon collects system state, exports it as JSON, and a set of Eww widgets (HUD, launcher, notification center) render it live. Long-term target: Jarvis/Iron-Man-HUD-style desktop with AI integration, plugin system, theme engine.

Core rule that has held throughout: **strict separation between data collection (Rust daemon) → shared state → export → frontend (Eww)**. Modules never know about Eww. Eww never contains business logic. This has not been violated and should not be.

---

## 2. Architecture (current, stable — do not redesign)

```
CPU Module ─┐
Memory Module ─┤
Battery Module ─┤
Disk Module ──┼─→ Module Manager ─→ Shared State ─→ Exporter ─→ state.json ─→ Eww widgets
Network Module ┤
Temperature Module ┘

Eww widgets:
  samos_hud_window        (live system stats, polls state.json every 1s)
  samos_launcher_window   (app launcher, own data source: .desktop files)
  samos_notif_window      (notification center, own data source: mako)
```

Three independent data paths feed the frontend:
1. `samosd` → `state.json` → HUD (polled every 1s via `jq`)
2. `.desktop` file scan → launcher (triggered on keystroke, debounced 200ms)
3. `mako` (external D-Bus notification daemon) → notification panel (polled every 2s via `makoctl history`)

---

## 3. Tech Stack

- OS: Arch Linux
- Compositor: Hyprland
- Widgets: Eww (yuck + scss)
- Backend: Rust — `tokio` async, `serde`/`serde_json` for state, `sysinfo` for metrics
- Notification daemon: `mako` (not custom-built — deliberate scope decision, see Section 7)
- Process supervision: systemd user service (`samosd.service`)

Not yet in use: `zbus`/D-Bus (planned replacement for JSON file export), Waybar, mpvpaper.

---

## 4. Repository Structure

```
~/Projects/SamOS/
  crates/
    samos-core/
      src/
        modules/
          cpu.rs
          memory.rs
          battery.rs
          disk.rs
          network.rs
          temperature.rs
          mod.rs
        state.rs
        config.rs        (exists, not yet wired into daemon — Config struct: theme/monitor/refresh_ms)
        lib.rs
    samosd/
      src/
        module_manager.rs
        export.rs
        modules/          (thin re-export wrappers around samos-core modules — known tech debt)
          cpu.rs
          memory.rs
          battery.rs
          disk.rs
          network.rs
          temperature.rs
          mod.rs
        main.rs
```

Eww config lives outside the repo, in `~/.config/eww/`:
```
~/.config/eww/
  eww.yuck                    (entry point — includes below files)
  eww.scss                    (entry point — imports below files)
  samos_hud.yuck / .scss
  samos_launcher.yuck / .scss
  samos_notifications.yuck / .scss
  scripts/
    launcher.sh                (scans .desktop files, filters, pushes to samos_launcher_results)
    launch.sh                  (launches selected app, closes launcher)
    notif_poll.sh               (polls makoctl history → JSON)
    notif_dismiss.sh            (dismiss single notification by id)
    notif_dismiss_all.sh        (dismiss all)
```

Mako config: `~/.config/mako/config` — includes `output=eDP-1` to pin popups to laptop screen (was defaulting to external monitor, still being debugged — see Section 8 open issues).

systemd unit: `~/.config/systemd/user/samosd.service` — runs `samosd` in release mode, `Restart=on-failure`, enabled via `systemctl --user enable --now samosd.service`.

---

## 5. Shared State Schema (`samos-core/src/state.rs`)

```rust
pub struct State {
    pub system: SystemInfo,       // hostname, time, uptime (uptime still hardcoded to 0)
    pub cpu: Cpu,                 // usage: f32
    pub memory: Memory,           // used_percent, used_mb, total_mb
    pub battery: Battery,         // percent, status (reads /sys/class/power_supply/BAT0 — hardcoded path)
    pub disk: Disk,               // used_percent, used_gb, total_gb (root mount only)
    pub network: Network,         // rx_kb, tx_kb (delta since last 1s refresh — reads as 0 when idle, this is expected)
    pub temperature: Temperature, // celsius (first sensor component found — not selective yet)
}
```

Exported every 1s to `~/.local/state/samos/state.json` via full truncate-and-rewrite (not append). This is why Eww uses `defpoll` with `jq`, not `deflisten`/`tail -f` — the file gets fully rewritten each cycle, not appended to.

---

## 6. Completed So Far

**Backend (Rust)**
- Module trait system (`init`/`update`/`shutdown`), Module Manager, Shared State — stable, unchanged since original design
- 6 working modules: CPU, Memory, Battery, Disk, Network, Temperature
- JSON export to `~/.local/state/samos/state.json`
- Daemon compiles and runs via `cargo run -p samosd`, now supervised by systemd user service (survives terminal close, restarts on crash)

**Frontend (Eww)**
- HUD widget: live hostname, time, CPU%, RAM%, battery%/status, disk%, net rx/tx, temp — polling `state.json` every 1s
- App launcher: scans `.desktop` files from `/usr/share/applications` and `~/.local/share/applications`, live filter on keystroke, click-to-launch, bound to `SUPER+SPACE` in Hyprland
- Notification center: built on `mako` as the actual D-Bus notification daemon (not custom-built), panel polls `makoctl history` every 2s, supports dismiss-one and dismiss-all

**Verified working end-to-end**: daemon → state.json → HUD updates live; launcher opens/filters/launches; notifications appear both as mako popups and in the Eww panel.

---

## 7. Deliberate Scope Decisions (so the next session doesn't relitigate these)

- **mako, not a custom notification daemon.** Implementing `org.freedesktop.Notifications` over D-Bus from scratch is a much larger scope than "one feature at a time" — mako gives a working notification pipeline immediately, Eww panel just visualizes its history.
- **JSON file export, not sockets/D-Bus yet.** Doc's own Section 8/20 marks this as temporary/technical debt. Intentional — not a mistake, don't "fix" it without a reason tied to an actual feature need.
- **`config.rs` (theme/monitor/refresh_ms) exists in samos-core but is not wired into `samosd` yet.** Not a bug — just not yet consumed. Next natural step if picking up the theme engine work.
- **samosd wrapper modules that just `pub use samos_core::modules::x::*`** are known, tracked tech debt from the original doc (Section 20) — not something introduced this session, don't be surprised by it.

---

## 8. Open / Unresolved Issues

- **Mako output pinning still being debugged.** Added `output=eDP-1` to `~/.config/mako/config` to force popups onto the laptop screen (they were defaulting to the external `DP-1` monitor) — last test still showed the popup on the external monitor. Needs verification: check `pgrep -a mako` for duplicate/stale processes, confirm `mako --version` supports the `output` config key, and check `~/.config/hypr/hyprland.conf` for a conflicting `exec-once = mako` line without `-c ~/.config/mako/config`.
- `uptime` is still hardcoded to `0` in state.json (never wired to real uptime).
- Network module reads instantaneous deltas — will show `0` until there's real traffic between two 1s polls. Not a bug, but worth a smoothing/rolling-average pass eventually.
- No tests, no CI, no plugin loader, no config system consumption yet (per original doc Section 20 — still true).

---

## 9. Roadmap From Here

**Phase 3 (in progress)**: Launcher ✅, Notifications ✅ (pending mako monitor fix) → Control Center, Workspace manager still open.

**Phase 4**: AI assistant, Automation, Plugin system, Theme engine (this is where `config.rs`'s `theme` field would finally get consumed), Voice.

**Phase 5**: Production optimization, migrate JSON export to Unix sockets/D-Bus, packaging, installer, docs, testing, release.

---

## 10. Ground Rules Carried Forward

- No architecture redesign without measurable benefit — the module → state → export → HUD pipeline is considered stable.
- One feature at a time, compile and run before moving on.
- Modules never touch Eww. Eww never contains business logic.
- Keep commits small (per original doc's git workflow section — not enforced in this session's command dumps, worth tightening going forward).
