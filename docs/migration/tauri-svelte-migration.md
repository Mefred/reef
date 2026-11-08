# Reef desktop migration: Slint to Tauri 2

## Decision

Replace the Slint desktop frontend with a Tauri 2 desktop application using Svelte 5, TypeScript, and Vite. Preserve the existing Rust workspace crates and their device/configuration responsibilities. The migration targets Linux, Windows, and macOS.

This is a complete UI rewrite, not a rewrite of the configuration engine or transport implementations.

## Repository baseline

The existing workspace contains:

- `reef-engine`: device models, configuration parsing, planning, generation, and verification.
- `reef-session`: transport/session abstractions and SSH, Telnet, and serial support.
- `reef-storage`: persisted profiles and configuration data.
- `reef-logging`: application logging.
- `reef-app`: current Slint desktop application.

The current application wires Slint callbacks directly to a `ScreenController`, which owns a session manager through `Rc<RefCell<SessionManager>>`. The controller also contains Slint-specific callbacks. Existing UI callbacks use `Runtime::block_on`, which can block the UI thread.

## Target architecture

```text
Svelte 5 + TypeScript + Vite
          |
          | typed Tauri commands, events, and channels
          v
Tauri host (reef-app)
  - command handlers
  - application state
  - application services
  - event publication
          |
          v
Rust domain crates
  - reef-engine
  - reef-session
  - reef-storage
  - reef-logging
          |
          v
Network devices (SSH / Telnet / serial)
```

The frontend must remain a presentation layer. It must not implement device protocols, parse CLI output, generate device configuration, or hold long-lived device credentials.

## Critical session ownership constraint

The existing `SessionManager` stores sessions using `Rc<RefCell<Box<dyn DynSession>>>` and its transport/session trait objects are designed for a single-threaded owner. Do not put this manager directly into Tauri managed state, do not assume replacing `Rc` with `Arc` is sufficient, and do not use unsafe Send/Sync implementations.

Preferred migration path: introduce a session actor that owns the non-Send manager on a dedicated thread with a Tokio current-thread runtime and `LocalSet`. Expose only a thread-safe actor handle to Tauri state, using message channels and oneshot responses. Keep operations asynchronous from the command handler's perspective. Evaluate whether transport internals can eventually become Send before considering removal of the actor.

## Application boundaries

- Keep `reef-engine` transport- and UI-independent.
- Remove Slint callbacks and UI property mutation from the controller; extract a UI-independent device workspace/service.
- Define serializable request/response DTOs at the application boundary.
- Keep command handlers thin: validate, delegate, map errors, return results.
- Use Tauri events for low-volume lifecycle/progress notifications and channels for ordered/high-volume terminal streams.
- Assign operation IDs; support cancellation only where it can be made safe.
- Never block the Tauri command executor with synchronous network work.

## Frontend

Use Svelte 5, TypeScript, Vite, pnpm, Tailwind CSS, shadcn-svelte, and Lucide icons. Add xterm.js for the interactive terminal when terminal integration begins. Keep dependencies purposeful and pin the package manager/lockfile.

Design direction: dark neutral foundation, Reef mint accent (`#36D8A2`), responsive desktop layout, connection-first launch, one primary application window, persistent navigation, device identity/status in the top bar, and explicit loading/empty/error/disconnected states.

The first-run flow must prioritize connecting to or creating a device profile. After a successful connection, reveal the device workspace. Do not show device-specific controls as if they were usable while disconnected.

## Security and persistence

- Store profile metadata in `reef-storage`; never store passwords or enable secrets in plaintext profile files.
- Integrate with OS credential facilities through a Rust abstraction (Secret Service/keyring on Linux, Windows Credential Manager, macOS Keychain).
- Do not log secrets or include them in events.
- Keep SSH host-key verification enabled and make trust decisions explicit.
- Use restrictive Tauri capabilities; add permissions only when a feature requires them.
- Keep arbitrary shell execution out of the frontend API.
- Use atomic writes for configuration and backup artifacts.

## Migration phases

1. Audit all existing Slint screens, callbacks, controller methods, storage behavior, and tests.
2. Decouple controller/domain behavior from Slint; introduce typed DTOs and the session actor.
3. Establish Tauri 2 host, Svelte/Vite frontend, capabilities, and cross-platform build configuration.
4. Build the shared design system and responsive application shell.
5. Implement connection manager, profile persistence, credential integration, and connection lifecycle.
6. Restore device dashboard, interfaces, VLANs, routing, ACL, DHCP, NAT, and system views supported by the backend.
7. Restore configuration inspection, planning, diff, apply, verification, rollback, backups, and history.
8. Integrate interactive terminal and background operation UX.
9. Complete automated tests, platform packaging, docs, and remove all Slint dependencies/assets.

Keep the application buildable at each phase. Do not delete the old implementation until the replacement has equivalent tested behavior; however, the final PR must remove Slint entirely.

## Acceptance criteria

- No Slint runtime, build dependency, build script, generated UI, or source assets remain.
- Existing Rust engine and transport behavior is preserved.
- No blocking network operation runs on the UI thread.
- Session lifecycle is safe under Tauri's threading model.
- All existing supported device-management workflows are available in the new UI.
- Connection profiles and OS-backed credential handling work.
- Configuration preview/apply/verify/rollback are validated against a real device or realistic integration environment.
- Terminal streaming is ordered, bounded, and responsive.
- Linux, Windows, and macOS build/package workflows pass.
- Rust formatting, Clippy, unit/integration tests, frontend checks, and production builds pass.
- Clean installation and upgrade behavior are documented and tested.

## Current implementation status

Implemented in the migration branch:

- Tauri 2 host and Svelte 5/Vite frontend shell.
- Tauri-managed persistent profile storage and asynchronous list/create/delete commands.
- Profile IDs are constrained to safe filename characters before storage paths are built.
- Connection form supports SSH, Telnet, and serial profile types.
- Transient password and enable-secret fields are passed only to the connection command and are not part of the profile DTO.
- A dedicated thread-affine session actor owns `SessionManager` on a current-thread Tokio runtime with a `LocalSet`.
- Tauri managed state contains only the actor's message-channel handle, not the non-Send manager.
- Connect, disconnect, command-send, connection-status, identify, privilege-escalation, and device-workspace inspection actor operations are exposed through typed Tauri commands.
- Initial live Interfaces, VLANs, and static Routing views query IOS CLI output through the actor and parse it with the existing transport-independent `reef-engine` parsers. Per-command errors are isolated so one unsupported command does not suppress the other sections.
- A read-only Running Configuration view retrieves `show running-config`, parses it through `parse_running_config`, and returns a deliberately sanitized DTO containing hostname/domain, interfaces, VLANs, static routes, ACL names and parsed rules, DHCP pool networks/routers/DNS servers, and parser warnings. Secret-bearing line passwords and other sensitive configuration fields are not serialized to the frontend. Parser diagnostics expose only line numbers and warning reasons, never the unrecognized raw configuration text.
- The existing `reef-engine` configuration planner, validator, command generator, diff renderer, and verifier remain available as pure Rust domain logic. The planner now sorts map-backed interface, VLAN, ACL, and DHCP changes so previews and generated command order are deterministic; focused ordering tests cover interfaces, VLANs, ACLs, and DHCP pools. It also emits one system-level modification operation when any hostname, domain, or DNS setting changes, preventing the generator from repeating the full system command block for each changed field.
- The session layer now records authenticated/privileged state from the detected IOS XE prompt, so an already-privileged session can be used without an unnecessary enable exchange.
- Unknown SSH host keys are rejected by default; the UI requires an explicit trust action before accepting and recording an unknown key.
- An initial xterm.js terminal sends commands through the actor and displays complete command responses. This is request/response execution, not a continuous interactive byte stream.
- Cross-platform GitHub Actions checks are defined for Linux, Windows, and macOS.

Still pending:

- Full Slint screen/controller audit and extraction of all domain workflows.
- Credential vault integration using each platform's native secure storage.
- Device identification and parsed device inventory/dashboard.
- ACL, DHCP, NAT, system, and full interface/VLAN/route configuration editing. The current operational and running-configuration views are read-only.
- Expand the typed, read-only planner preview beyond its initial system-settings scope. The current Tauri command reads the running configuration in the actor, applies only explicitly supplied hostname/domain/additive-DNS draft fields to a clone, validates and renders a diff, and returns generated commands for review. Input validation has unit coverage. The verifier is not yet integrated into the preview workflow.
- Safe apply orchestration, verification, rollback, backups, and history.
- Interactive terminal streaming, bounded output, cancellation, and operation progress.
- Green CI on the latest branch head and platform packaging validation. The source icon is now a 512×512 RGBA PNG. Cross-platform runs on an earlier head have passed Windows workspace checks, tests, and icon generation; current-head Linux, Windows, and macOS checks/builds remain in progress.

The current increment is a functional connection/profile foundation, not feature parity. Do not report the migration complete until the acceptance criteria and corresponding integration tests pass.

