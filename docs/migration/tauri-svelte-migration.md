# Reef desktop migration: Slint to Tauri 2

## Goal

Replace Reef's Slint desktop application with a Tauri 2 desktop application using Svelte 5, TypeScript, and Vite.

This is a desktop-framework migration, not a rewrite of Reef's network-management product. Keep the existing Rust domain, parser, planner, transport, storage, and logging crates as the source of truth. Do not expand the PR into new configuration-management features merely because the new frontend makes them possible.

## Existing architecture

The Rust workspace contains:

- `reef-engine`: device models, parsing, planning, command generation, diffing, and verification.
- `reef-session`: SSH, Telnet, serial, and session lifecycle.
- `reef-storage`: profiles and persisted configuration data.
- `reef-logging`: application logging.
- `reef-app`: desktop entry point and UI integration.

The old Slint application used a `SessionManager` backed by `Rc<RefCell<...>>`. That manager is not suitable for direct storage in Tauri's thread-safe managed state. The Tauri host therefore uses a dedicated session actor that owns the manager on a current-thread Tokio runtime and `LocalSet`; Tauri commands communicate with it through a thread-safe handle. Do not add unsafe `Send`/`Sync` implementations or move the manager into Tauri state.

## Target architecture

```text
Svelte 5 + TypeScript + Vite
          |
          | Tauri commands / events
          v
Tauri 2 desktop host
  - command handlers
  - persistent application state
  - session actor handle
          |
          v
Existing Rust workspace crates
  - reef-engine
  - reef-session
  - reef-storage
  - reef-logging
          |
          v
Network devices (SSH / Telnet / serial)
```

The frontend is a presentation layer. Device protocols, CLI parsing, configuration planning, and persistence stay in Rust.

## Migration decisions

- Frontend: Svelte 5, TypeScript, and Vite.
- Desktop host: Tauri 2.
- Platforms: Linux, Windows, and macOS desktop.
- Preserve the existing Rust workspace and domain boundaries.
- Use typed Tauri commands for request/response operations.
- Keep the non-`Send` session manager isolated behind the session actor.
- Keep credentials out of saved profile JSON and logs.
- Keep Tauri capabilities minimal; the current frontend only needs the core application capability.
- Keep the current configuration preview review-only. Do not introduce device configuration writes as part of the framework migration.

## Existing UI scope

The previous Slint window had a working connection flow and interface editor. Several other navigation sections were placeholders ("coming soon") or had no implemented content. The migration should not treat those placeholders as missing features that must be built in this PR.

The Tauri frontend currently provides:

- Saved SSH, Telnet, and serial profiles.
- Connect/disconnect, device identification, privilege escalation, and command execution.
- Explicit trust handling for unknown SSH host keys.
- An overview dashboard.
- Live interface, VLAN, and static-route inspection.
- Sanitized running-configuration inspection for system settings, ACLs, DHCP, supported NAT syntax, SSH settings, and line access policy.
- A bounded xterm.js request/response terminal.
- A read-only configuration preview for hostname, domain name, additive DNS servers, and add-only VLAN creation.

The old Slint interface editor is the one notable implemented UI workflow not carried into the Tauri frontend. This migration intentionally does not port its desired-state editing and device-apply workflow: doing so would expand a framework replacement into configuration-management work, which is outside the agreed scope. The Tauri Interfaces view provides live inspection, and the limited configuration preview remains review-only. Track interface editing and safe apply/verification as a separate follow-up rather than treating it as a blocker for replacing Slint.

## Migration acceptance criteria

- Tauri 2 is the only desktop host and application entry point.
- Svelte is the only desktop UI.
- Slint runtime/build dependencies, macros, generated UI integration, and source files are removed.
- The application builds and launches in development mode.
- The production desktop build and bundle configuration are valid.
- The existing Rust workspace crates remain independent of the UI framework.
- The session manager is safely isolated from Tauri's thread model.
- Existing Tauri frontend workflows work through registered Rust commands.
- Linux, Windows, and macOS CI builds pass.
- README and development instructions describe only the Tauri application.

## Explicitly out of scope

These are product/security improvements, not requirements for changing the UI framework:

- Native OS credential vault integration (Keychain, Credential Manager, Secret Service).
- New or comprehensive interface, VLAN, route, ACL, DHCP, NAT, SSH, or system editors.
- New configuration apply, backup, verification, or rollback workflows.
- Continuous terminal streaming, cancellation, and operation progress.
- Expanding IOS parser coverage beyond what the existing application needs.
- Mobile targets, auto-updating, code signing, and release distribution infrastructure.

Do not hold the framework migration open for these unrelated features. Track them separately after the Tauri replacement is complete.

## Verification

Run the frontend type check and production build, Rust formatting and workspace checks/tests, and Tauri desktop builds on Linux, Windows, and macOS. A green compile alone is not sufficient: manually smoke-test launch, profile CRUD, connection/disconnection, device inspection, configuration preview, and terminal command execution against a suitable test device or simulator before release.
