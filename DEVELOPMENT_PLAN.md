# SamOS + MIKO — forward implementation plan

Status: implementation started. This is the forward plan for the product visions supplied in conversation. It supersedes speculative claims in earlier roadmaps, not the repository's architecture rules. Completion below means code, tests, installed runtime verification, and operator documentation all agree.

## 1. Product contract and boundaries

- Desktop: translucent corner panels and real audio spectrum on the laptop's `eDP-1` only, below ordinary windows, without reserving application space. External displays must remain free of SamOS panels, including after reconnects.
- MIKO: one local conversation experience for casual discussion, system questions and proposed actions. Spoken and typed requests use the same tool executor and confirmation policy.
- Trust: reading and suggesting are automatic; launching, opening, scheduling, deleting, changing playback/settings and creating automation require approval of the exact action. A model-generated “yes” never approves a tool.
- Explicit HUD button clicks authorize the named immediate action. A Voice button authorizes a bounded recording, not arbitrary tool actions inferred from that recording.
- Approved automation authorizes a precisely previewed recurring action under a precisely previewed condition. Changing either invalidates approval. Existing hand-authored rules need migration, not silent expansion of permissions.
- Personal data stays local. Online lookup sends only a deliberate query and necessary location parameters, never the conversation, files, schedule or profile as a bundle.
- Preserve Modules → Module Manager → Shared State → Exporter → state.json → Eww. Do not replace JSON export, remove passthrough wrappers, or introduce a dashboard framework.
- Wake-word detection is a separately supervised local worker feeding existing voice operations. New state is exported through the existing pipeline; no module knows about Eww.
- No new Rust dependency until a concrete capability needs it. Validate external library/service interfaces against upstream documentation when implementing that phase.

## 2. Verified baseline and corrections to earlier reports

Inspected: `modules/ai.rs`, `automation.rs`, `state.rs`, daemon main, desktop bridge and yuck, installer and existing tests.

| Area | Current code | Consequence |
| --- | --- | --- |
| Text assistant | Local Ollama, nine registered tools, per-conversation pending approvals | Build on this executor; do not introduce a second tool path |
| Model | Defaults to qwen2.5:1.5b, supports SAMOS_MODEL | Keep the smaller model for laptop memory; measure quality/latency rather than promising it |
| Reminders | SQLite create/list; no announcement worker | Creation currently bypasses approval; delivery still needs implementation |
| Calendar | No calendar tools in active AI registry | Build calendar persistence first; conflict detection is not merely a patch to existing scheduling |
| Voice | Bounded record/transcribe and Piper playback | No arbitration, wake-word worker, automatic spoken conversation or exported speaking flag yet |
| Continuity | In-memory messages plus best-effort model summary per conversation | Summary timeout can lose continuity; no durable canonical voice thread yet |
| Profile | Fixed system prompt | Add validated profile before learned preferences |
| Automation | Prefix/substring checks and shell execution; checks whole-state changes | Not safe to expose to MIKO yet; repair validation/execution and triggering first |
| Daemon | Infinite main loop, unreachable shutdown, failure propagation | Background workers require a verified graceful lifecycle before deployment |
| State | `state.rs` exists | The note in AGENTS.md saying it was deleted is stale; no recreation required |
| Desktop | Repo-managed assets installed explicitly; state and chat polling | New AI state must synchronize Rust, Python defaults, and yuck initial values |
| Runtime evidence | Services were running, model exported as idle | This is not proof of complete end-to-end tool, voice, or product-vision acceptance |
| Repository | Existing source edits plus tracked build artifacts | Preserve edits; avoid mixing build-output cleanup into functional changes |

## 3. Delivery discipline

Every numbered increment below is independently reviewable. Do not start another source feature on a failed build.

1. Record baseline and changed files; preserve unrelated work.
2. Implement one feature, including its confirmation classification and failure handling.
3. Add focused tests for behavior, side effects and failures; use temporary databases/files and mock service replies.
4. Format changed Rust files; run focused tests and `cargo build --offline --release -p samosd` (online dependencies only if needed and approved).
5. Restart the actual `samosd.service`, verify process/service state and a fresh state.json; do not mistake a build for activation.
6. Check the feature at runtime. Read-only requests can run live. Use isolated fixtures for writes, and deny real desktop action proposals unless the user specifically requests execution.
7. If state changes, compare Rust serialization, Python DEFAULT_STATE and yuck `:initial`; verify old JSON/missing fields degrade safely.
8. Install frontend changes explicitly with backups, independent of Cargo. Inspect actual Eww rendering and logs when UI changes.
9. Record observed results and outstanding limits here; update USER_GUIDE.md with commands and recovery steps.

## 4. Phase A — trustworthy conversational foundation

### A1. Close reminder confirmation gap (first implementation)

Files: `crates/samos-core/src/modules/ai.rs`, guide.

- Add `add_reminder` to the central confirmation policy and make its schema description honest.
- Prove proposing a reminder creates no database row; denial, expiration and cross-conversation approval attempts create no row.
- Prove approval writes the reviewed text/time once; replay cannot write a duplicate.
- Preserve the existing tool-call queue order and per-call approval behavior.
- Live smoke: request a reminder proposal, inspect exact arguments, deny it, and verify no reminder was saved.
- Exit: regression tests and release build pass; live daemon emits a ToolCall before any reminder write.

### A2. Profile, tone and user-maintained shortcuts

Files: new `ai_profile.rs` helper, `lib.rs`, `modules/ai.rs`, example profile and guide. No new metric module, dependency, state field or installer overwrite.

- Optional `~/.config/samos/profile.toml`: `name`, `tone`, `[shortcuts]`.
- Missing file gives defaults. Bound file size, field lengths and shortcut count to keep prompt context small.
- Validate complete profile before applying it. Reject unknown fields, malformed URLs, non-HTTP(S) schemes and URLs with credentials.
- Reload at each new user turn; invalid reload keeps last valid profile and logs the error. Removing the file restores defaults.
- Include preferences as explicitly labelled data, below fixed tool/trust rules. Preferences cannot change the executor's approval policy.
- Shortcuts enter context but opening them waits for the browser tool phase; never invent an opening success.
- Exit: missing/valid/malformed/oversized/deleted profile tests; prompt reload integration tests; default live chat succeeds without requiring a profile.

### A3. Durable conversation continuity

- Add versioned SQLite migration for bounded message history, retaining old summaries.
- Canonical default voice conversation `miko`; keep explicit desktop/terminal conversation IDs and return IDs in every response.
- Persist completed exchanges without a second blocking inference on the response path. Summaries compress older history but are not the only durable record.
- Preserve assistant tool-call/result relationships during truncation; persist no executable pending approvals across restarts in this first version.
- Add inspect/reset memory commands with confirmation for deletion. Document retention and how to remove local history.
- Tests: restart, repeated voice turns, isolation, incomplete tool exchange, rollback from failed write, bounded retention.
- Exit: a follow-up after daemon restart correctly uses a prior fact from the same conversation.

### A4. Voice presence and audio arbitration

- Single audio coordinator for capture/playback. Explicit ownership, finite waiting and cancellation; never hold the general conversation or metrics lock while waiting for audio.
- Export `ai.listening`, `ai.transcribing`, `ai.speaking` with serde defaults. Frontend derives chat presence from exported state, not guessed timers.
- RAII guards clear flags on failure and cancellation. The speaking flag covers actual playback, not model inference.
- Voice response policy: explicit voice interaction can request spoken answers; text chat does not unexpectedly speak. Pending tool approval is displayed/read back, never auto-approved.
- Tests: simultaneous capture/speech, failed Piper/aplay, recorder timeout, canceled request; all flags return to idle.
- Exit: push-to-talk → transcript → answer → optional playback works with visible accurate HUD states and continuous metrics.

## 5. Phase B — runtime and automation prerequisites

### B1. Graceful supervision

- Handle SIGTERM/Ctrl-C in the daemon loop; stop accepting new requests, cancel/join bounded workers and clean only owned sockets/children.
- Isolate optional plugin/module failures while surfacing degraded status; avoid an unavailable optional sensor taking down MIKO.
- Populate system metadata before rules evaluate it. Keep business logic out of ModuleManager.
- Exercise two consecutive restarts, stop during inference and audio, and optional dependency failure.
- Exit: no duplicate listeners, orphaned Cava/recorders or stale busy flags; service stops within its timeout.

### B2. Rules execution hardening

- Replace shell execution with typed action dispatch and literal argument arrays; forbid shell interpretation and broad command prefixes.
- Validate actual field paths, types, operators, unique rule IDs and allowed action values against the supported rule model.
- Trigger on each rule's false→true transition, with explicit initial-match/rearm policy. Bound worker queue and process deadlines; rule actions must not block metric collection.
- Validate whole ruleset on reload; retain last valid rules on malformed updates and clear rules on deliberate removal.
- Factor a pure condition evaluator reusable by contextual reminders; do not make modules depend on one another.
- Tests: separators/substitution, wrong field/type, duplicate IDs, repeated true values, false→true reset, reload, queue saturation and timeout.
- Exit: no `sh -c` path accepts rule/model text; one sustained condition produces one event under the documented policy.

## 6. Phase C — files, process awareness and browser

### C1. Process query (read-only)

- Add `system_query` with memory/CPU sort and bounded top-N via existing sysinfo; sample CPU over an actual interval and label units.
- Return process name, PID and usage; omit full command lines/environment, which can expose secrets.
- Execute outside metric polling; unavailable process reads return partial labelled results.
- Exit: tests of limits/sorting/units; live answer identifies actual high-memory processes without needing a task manager.

### C2. File search and open

- `find_files` read-only: configured roots under home, bounded traversal/time/result count, file type/name and modification-time filters.
- Verify installed fd syntax at implementation; literal argv, explicit option terminator, canonical roots and symlink boundary checks. Modification time is not last-read time; say so.
- File-content searching is separate opt-in scope, not an implicit recursive upload/index.
- `open_file` confirmed: canonical existing regular file, reject executable/desktop entries and unsafe URI schemes; return a preview before launching a handler.
- Tests: spaces, leading dashes, traversal, symlinks, hidden files, nonexistent paths, timeout and bounded output.
- Exit: locate a fixture PDF and propose opening the exact result, with no launch on denial.

### C3. Browser and shortcuts

- `open_url` confirmed, required browser enum `firefox|vivaldi`, validated HTTP(S), literal argv. Ask for browser if absent; no silent default.
- Resolve user shortcut by exact key; no guessed account/inbox. Preview full normalized URL and chosen browser.
- Verify launch binary availability and failure without claiming the page loaded; only browser handoff is observable initially.
- Exit: tests for malformed URL/credentials/schemes/browser enum and confirmation; deny live opening; separately execute only user-requested navigation.

## 7. Phase D — local schedule and contextual initiative

### D1. Calendar persistence and read-only listing

- Choose/test an iCalendar parser with explicit need before adding a crate; use `.ics` vdir at `~/.local/share/calendars/personal/`.
- Initial writes are single non-recurring events. Read imported all-day/timezone events correctly or return an explicit unsupported warning; never silently miss conflicts from unsupported recurrence.
- Stable UIDs, escaped/folded text, safe filenames, atomic per-event write and bounded directory scan. No attendee invitations/network sync implied.
- Tests: roundtrip, Unicode/newlines, local/UTC boundaries, DST ambiguity and malformed/imported events.

### D2. Confirmed scheduling/cancellation and conflict preview

- `list_schedule` read-only; `schedule_meeting` and `cancel_meeting` confirmed. Dates ambiguous to a human need a follow-up, not model guessing.
- Preview start/end/timezone, title and overlap summaries before approval. Recheck conflicts immediately before commit; changed preview requires a new approval.
- Define overlap with half-open intervals [start,end), so adjacent events do not conflict. Cancellation targets stable UID and previewed event version.
- Tests: adjacent/overlapping/containing events, zero-length interval, changed calendar between approval and write, stale cancellation and repeat approval.
- Exit: fixture schedule/list/cancel roundtrip and conflict re-approval verified without altering personal appointments.

### D3. Reminder delivery and conditions

- Versioned migration adds reminder kind/condition/delivery state; retain existing timestamp reminders.
- One bounded worker polls due times and validated structured conditions using the B2 evaluator; no LLM call on each tick.
- Define startup behavior, due tolerance, timezone rendering and trigger hysteresis. Conditions default to a one-shot reminder; missing/stale sensor values cannot trigger.
- Persist delivery claims to avoid repeats across restart. On playback failure keep a visible notification and bounded retry state rather than silently dropping or endlessly repeating it.
- “Before I leave” requires an observable trigger chosen with the user; no unsupported location inference.
- Exit: fake-clock and restart tests, condition transition tests, denied creation and reliable visible delivery.

### D4. Limited proactive watches and morning brief

- Few fixed, opt-in watches: battery below chosen threshold and meeting in ten minutes. Deduplicate with durable keys and battery hysteresis.
- Quiet hours, movie mode and audio ownership control whether alerts are visual, deferred or spoken; no automatic system changes.
- Optional morning brief after 06:00 local time on the first interaction; once-per-local-date marker survives restart. Includes today's schedule and overdue reminders only.
- Exit: tests across midnight, restart, charging/discharging, sleep/resume and meeting cancellation; no repeated alert storm.

## 8. Phase E — wake-word conversation

Depends on A3/A4/B1. A detector alone is not a complete voice loop.

- Evaluate openWakeWord locally against installed Python/audio versions and CPU budget. Verify upstream model/training/licensing instructions before downloading assets.
- Separate opt-in worker/service with an explicit mute control, model path, threshold, debounce and visible status. Fail closed when custom MIKO model is absent.
- Capture stays local and transient; detector yields microphone ownership before invoking the same recording/transcription path as push-to-talk.
- Ignore triggers during playback, apply cooldown, bound request queue, and return to listening after failures. Never continuously buffer/store room audio for memory.
- Voice approvals need a separate short approval state bound to the exact pending ID and expiry. “Yes” from normal conversation, recordings from another session, or model output must not approve an action.
- Tests: simulated detections, debounce, stale approval, playback feedback, mute/restart, missing mic/model and CPU load. Measure real false accepts/rejects with user samples.
- Exit: spoken wake → answer → listening completes repeatedly with no button; explicit mute immediately closes capture. Custom model quality remains a measured user acceptance gate.

## 9. Phase F — media and live information

### F1. Local player control

- Inspect each player's actual MPRIS capabilities; do not assume Spotify MPRIS is read-only.
- `now_playing` read-only. All play/pause/skip/seek/volume/open-URI actions confirmed to follow the stated trust rule consistently.
- Select player explicitly when ambiguous; never send commands to every player. Treat metadata as untrusted text.
- Exit: capability-aware tests, no-player state, deny action, and actual state change when explicitly approved.

### F2. Spotify catalog playback

- Implement search/queue by supported local capability first; use Spotify Web API only for capabilities unavailable locally.
- If API required: verify current scope/account requirements, use PKCE with state validation, loopback redirect, scoped tokens stored with restrictive permissions, token refresh and disconnect/revoke flow.
- Catalog search read-only; exact selected track/device/playback action confirmed. Handle expired auth, unavailable devices and rate limits honestly.
- User supplies account authorization; do not create credentials or subscribe on their behalf. Until then label backend unconfigured.

### F3. Web lookup and weather

- Backend interface with finite time/result/response-size limits. Configure SearXNG or Brave credentials/endpoint; no hardcoded public instance or paid service activation.
- Tool request contains query only; retrieved snippets are untrusted evidence, never system instructions or authority for other tools.
- Return source title, URL, timestamp and concise excerpts. User answer distinguishes retrieved facts from inference and cites sources; no fake live answer on network failure.
- Weather: explicit location/geocoding ambiguity handling, timezone, units, observation/forecast timestamp; dedicated provider only after verifying upstream API and usage terms.
- Test malformed responses, redirects, excessive payload, timeouts, rate limits, source injection and offline mode.
- Exit: cited current lookup plus offline failure demonstration; no leakage of local context into outbound query bodies.

## 10. Phase G — personal voice and learned style

- Profile-selected installed Piper voice first; validate asset availability before selection and retain working fallback. Provide sample playback as an explicit action.
- Custom voice is a separate data/training deliverable: authorized recordings, documented dataset/license, reproducible training, held-out intelligibility tests and comparison listening sessions. Never claim personalization from renaming a stock voice.
- Learned style: small local table of explicit, bounded preferences with provenance and last-used date; no inferred sensitive attributes. Keep this separate from tasks, permissions, shortcuts and factual memory.
- `remember_about_me` may write assistant preferences automatically under the user's stated exception; it cannot change tools, approval policy, account URLs or system settings.
- Provide inspect/edit/forget controls and reset; deletions confirmed. Conflicting preferences prefer latest explicit user instruction.
- Exit: adaptation persists and can be inspected/removed; profile/learned text cannot disable approval checks.

## 11. Phase H — complete SamOS moods and MIKO-created automation

- Themes specify palette, motion, density, visibility and quietness policies, not arbitrary scripts. Validate hud/hacker/elegant/motivation/love/movie independently.
- Ensure laptop-only placement across scale changes, lid close, disconnect/reconnect and suspend/resume. Eliminate duplicate SamOS surfaces while preserving unrelated panels.
- Integrate launcher/notification/control visual design with bounded data and honest errors. Add keyboard access and reduced-motion/readability checks.
- Only after B2, expose create/list/disable/delete rule tools. Create/modify/disable/delete require exact approval, showing trigger, actions, repeat policy and effect.
- Test denied rule creation, subsequent rule edits requiring renewed approval, and exactly-once transition behavior; no natural-language-to-shell execution.
- Exit: theme screenshot matrix and actual app-over-HUD layering checks; approved fixture rule acts once, denied rule never exists.

## 12. Acceptance matrix and release gate

| Scenario | Evidence required |
| --- | --- |
| Text conversation and system question | Real socket responses; metrics timestamp current; no fabricated action |
| Mutating tool proposal/deny/approve | Exact preview; zero effect until approval; one effect after approval; replay fails |
| Cross-session and restart | Durable conversation follows up; pending approvals invalidated safely |
| Voice and wake word | Audio ownership, accurate HUD flags, repeated loop, mute and failure recovery |
| Scheduling/reminders | Timezone/conflict tests; restart-safe delivery; bounded alert frequency |
| Media/web | Targeted player/device; cited live answer; unconfigured/offline state explained |
| Privacy | No local context in web requests; token/audio/history permissions checked |
| Desktop | eDP-1 only; below apps; no duplicates; real audio spectrum; theme matrix |
| Service lifecycle | Restart/stop without orphan children; metrics remain fresh during inference |
| Install/recovery | Existing config preserved; per-file backups; restore verified in temporary home |

Release checks: focused tests per increment; full workspace tests at integration boundaries; release build; service restart; frontend parse/log/render check when changed; operator guide matches actual tools. Record latency and memory on this laptop under realistic load instead of claiming an unmeasured performance target. Any unsupported capability stays explicitly marked unavailable.

## 13. External inputs and stopping conditions

- Search: chosen backend URL/key required for live search activation; implementation/tests can use fixtures before then.
- Spotify API: user account OAuth required only if local capabilities are insufficient.
- Wake word: a usable custom model and real-room measurements required for claiming reliable “Miko” activation.
- Voice: user-selected voice and consented recordings required for personal training; do not fabricate these choices.
- Calendar: choose/configure a viewer only when desired; storage/tools do not require a viewer.
- No architecture redesign is scheduled. If implementation needs one, present the concrete reason before proceeding.

## 14. Implementation ledger

| Increment | Status | Evidence |
| --- | --- | --- |
| Baseline/forward plan | Complete | Active code inspected; missing capabilities and inaccurate prior claims identified |
| A1 reminder confirmation | Implemented and runtime-verified | Approval/denial/expiry/wrong-ID/replay regression passed; release build and service restart; live `development-a1` returned `ToolCall`, denial returned `Done`, matching SQLite row count remained zero |
| A2 profile and shortcuts | Implemented and runtime-verified with defaults | Bounded parsing, atomic reload, invalid fallback, deletion and prompt integration tests passed; rebuilt/restarted service; live `development-a2` returned a normal greeting. Custom profile reload verified with isolated fixture files; personal profile was not changed |
| A3 durable continuity | Implemented and restart-verified | SQLite migration and bounded completed exchanges; no blocking summary inference; history inspection and explicit deletion; terminal voice defaults to miko; 16 core tests passed and release build activated. Live development-a3 recalled “Silver Orchard” after actual service restart; synthetic test history removed afterward |
| A4 voice presence/audio arbitration | Implemented; playback runtime-verified | Single bounded audio owner, phase guards, four backward-compatible state flags, HUD badge and opt-in terminal spoken reply. 18 core tests and 2 frontend compatibility tests passed; daemon/HUD restarted and live synthesis→speaking→idle observed |
| B1 graceful supervision | Implemented and runtime-verified | SIGTERM/SIGINT cleanup, stoppable listeners, joined request workers, cancellable subprocess groups, owned socket cleanup, Cava termination before reader join, optional module error isolation. 20 core tests + 1 daemon test pass; repeated live restarts and restart during inference verified |
| B2 onward | Planned | Safe automation is next; execute one feature/build/runtime cycle at a time |

Validation for A1/A2: `cargo test --offline -p samos-core` passed 13 tests; `cargo build --offline --release -p samosd` succeeded; daemon restarted after each increment. During final inference, exported metrics were 0.11 seconds old. No Rust/Python/yuck state shape changes, frontend deployment, new dependencies, or personal reminder writes were needed.

Known observations: the live small model shortened the test reminder's requested text to “SamOS”; the proposal exposed this before denial. Confirmation enforces reviewed arguments, not model understanding. Existing plugin warnings and unreachable daemon shutdown remain visible and are assigned to B1. Earlier source/build-artifact changes remain outside this increment's scope.

A3 details: versioned, additive `ai_migrations`/`conversation_history` tables leave reminders and legacy summaries intact. Retention is 128 conversations, normally 32 messages/16 KiB each, pruning entire older turns; a single larger turn is bounded at 64 KiB. Completed tool exchanges survive intact; pending approvals are never persisted. Tests cover restart isolation, stale approval rejection, confirmed deletion, failed-write preservation, legacy fallback and retention. Terminal `history` and `forget --yes` operate on explicit conversation IDs. Desktop continues using its explicit desktop conversation. State schema and frontend assets did not change. Automatic summary compression is deferred; deterministic bounded history now provides continuity without a second inference or unreliable summary dependency.

A4 validation: confirmed a short `tts_speak` test through the real socket and observed `ai.synthesizing`, then `ai.speaking`, then all flags false in exported state. Installed frontend with backup `20260919-200817-093459`; Eww opened all five surfaces on eDP-1, with no SamOS surfaces on DP-1. Inspected laptop screenshot `/tmp/samos-a4-laptop.png`; idle badge and corner layout render correctly. Python tests compare yuck/Python defaults and verify legacy/stale state handling. Rust tests cover backward-compatible deserialization, exclusive ownership, failure cleanup and phase export. Microphone capture and a complete `voice --speak` exchange were not live-tested in this increment; synthetic state/error tests cover their wiring. Contention uses a two-second bounded wait with retry feedback, not an unbounded queue. Cancellation and legacy automation audio coordination remain B1/B2 work; no wake-word worker was started.

B1 validation: `cargo test --offline -p samos-core -p samosd` passed 20 + 1 tests, including stalled AI client shutdown, active state-socket owner protection, idle socket shutdown and cancellation of a 30-second child. Socket tests required execution outside the sandbox because local Unix bind was blocked there. Release build succeeded without the old unreachable-shutdown warning. After activation, live restarts completed in 0.57 s and 0.73 s with no surviving old descendant PIDs. Restart during a synthetic assistant request completed in 0.87 s, delivered a shutdown error to the client and returned to idle with all audio flags false. Journal confirmed plugin shutdown and `SamOS daemon shutdown complete` for each new-build stop. Initial activation stopped the previous binary, which timed out on its old Cava child; the subsequent new-build checks did not time out.

B1 scope limits: signal handling occurs between metrics updates; synchronous legacy automation or a hung FFI plugin can still delay reaching cleanup. These are not claimed solved by request cancellation. Actual audio interruption was covered through the cancellable child/guard tests rather than a live microphone capture. Cava's child is now owned and reaped; automatic restart after unexpected Cava failure is not added here. Module errors are reported in logs; no new health-state schema was introduced. B2 must remove legacy shell/blocking automation paths before exposing rules to MIKO.
