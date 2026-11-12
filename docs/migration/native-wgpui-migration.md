# Reef desktop migration: web shell to native wgpui

## Goal

Replace Reef's desktop shell with a native Rust UI so the portable Windows build
runs directly from removable media without a webview runtime, Node.js, an
installer, or administrator rights.

The network-management product is unchanged. The existing Rust domain crates
remain the source of truth:

- `reef-engine`: device models, parsing, planning, command generation, diffing,
  and verification.
- `reef-session`: SSH, Telnet, serial, and session lifecycle.
- `reef-storage`: profiles and persisted data.
- `reef-logging`: diagnostics and session recording.
- `reef-app`: desktop entry point and UI integration.

## Why the web shell was removed

The project previously used a Tauri 2 host with a Svelte 5/TypeScript/Vite
frontend. That stack required:

- a WebView2 runtime on every Windows host;
- a separate Node.js toolchain and frontend build;
- a JavaScript command bridge between the UI and Rust state.

For a USB-distributed lab tool those are avoidable dependencies. The Tauri host,
Svelte sources, frontend build files, capabilities, and bundle configuration
have been removed.

## Target architecture

```text
wgpui native UI (Rust)
  - sections, forms, terminal, diagnostics views
  - entities and render tree
          |
          | direct Rust calls
          v
reef-app integration layer
  - SessionActor handle
  - portable Storage
  - AppDiagnostics
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

`wgpui` 0.4 is a GPUI-style Rust UI library built on winit and wgpu. It renders
with the platform GPU surface and has no DOM, JavaScript engine, or webview.

## Migration decisions

- UI toolkit: `wgpui` 0.4.
- Text entry: a local `TextField` entity adapted from wgpui's input example,
  with masked entry for credentials and an Enter-to-submit callback.
- Colors: a fixed dark-green palette rather than system light/dark colors.
- Threading: keep `SessionManager` on its dedicated current-thread Tokio runtime
  and `LocalSet` behind the existing `SessionActor`.
- Concurrency: blocking profile-storage and diagnostics work runs through
  `cx.background_spawn`; results are applied back on the UI thread.
- Configuration: the preview remains review-only. Startup-configuration writes
  remain permanently disabled.
- Packaging: the Windows target is `x86_64-pc-windows-gnu`, producing a single
  `reef.exe` that imports only system DLLs.

## Preserved workflows

- Saved SSH, Telnet, and serial profiles with create/refresh/delete.
- Connect/disconnect, device identification, privilege escalation, and command
  execution.
- Explicit first-use SSH host-key trust.
- Overview dashboard.
- Live interface, VLAN, and static-route inspection.
- Sanitized running-configuration inspection for system settings, ACLs, DHCP,
  supported NAT syntax, SSH settings, and line access policy.
- Bounded request/response terminal with backend session recording controls.
- Portable diagnostics status, log-file browsing, test notes, and ZIP export.

## Acceptance criteria

- `reef-app` builds and runs without Tauri, Svelte, Node.js, or a webview.
- The workspace crates remain independent of the UI framework.
- The thread-affine session manager stays isolated behind `SessionActor`.
- `cargo fmt --all -- --check`, `cargo check --workspace --all-targets`, and
  `cargo test --workspace` pass.
- The Windows cross-build produces a portable `reef.exe` with no non-system DLL
  dependencies.
- The portable layout is drive-letter independent and uses the executable's
  directory for `Config/`, `Data/`, and `Logs/`.

## Verification

Run the Rust checks and tests above, then build:

```sh
cargo build --release --target x86_64-pc-windows-gnu -p reef-app
```

Manually smoke-test launch, profile CRUD, connection/disconnection, device
inspection, configuration preview, terminal command execution, diagnostics
export, and recording controls on a Windows host with a suitable device or
simulator. The portable ZIP is assembled from `dist/Reef-USB/`.

## Explicitly out of scope

These are product improvements rather than framework-replacement work:

- Native OS credential-vault integration.
- New or comprehensive interface, VLAN, route, ACL, DHCP, NAT, SSH, or system
  editors.
- Configuration apply, backup, verification, or rollback workflows.
- Continuous terminal streaming and cancellation.
- Auto-updating, code signing, and release infrastructure.
