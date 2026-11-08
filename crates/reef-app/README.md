# Reef desktop application

Reef is a cross-platform desktop application for inspecting and managing Cisco IOS XE devices. The desktop host uses Tauri 2; the frontend uses Svelte 5, TypeScript, and Vite. Device models, parsing, planning, transports, and persistence remain in the Rust workspace crates.

## Prerequisites

- Rust toolchain matching the workspace minimum supported version.
- Node.js 22 and pnpm 9.15.4.
- Tauri 2 platform prerequisites:
  - Linux: GTK 3, WebKitGTK 4.1, app-indicator, librsvg, and build tools.
  - Windows: Microsoft C++ Build Tools and WebView2.
  - macOS: Xcode Command Line Tools.

Consult the Tauri 2 documentation for distribution-specific prerequisites.

## Install dependencies

From this directory:

```sh
pnpm install
pnpm --dir frontend install
```

## Development

```sh
pnpm tauri:dev
```

## Checks

```sh
pnpm --dir frontend check
pnpm --dir frontend build
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
```

The GitHub Actions workflow runs frontend checks/builds, Rust formatting, workspace checks and tests, icon generation, and Tauri desktop builds on Linux, Windows, and macOS.

## Desktop package

```sh
pnpm tauri:build
```

The build script generates platform icon variants from the committed 512×512 RGBA `icons/icon.png` source. A Windows ICO source is also committed.

## Current migration scope

The Tauri frontend currently provides:

- Saved SSH, Telnet, and serial connection profiles.
- Transient connection credentials; profile JSON does not persist passwords or enable secrets.
- A thread-affine session actor for the existing non-`Send` session manager.
- Connect/disconnect, device identification, privilege escalation, and command execution.
- Explicit trust handling for unknown SSH host keys.
- An Overview dashboard with device identity, interface state counts, and detected capabilities.
- Live interface, VLAN, and static-route inspection.
- Sanitized running-configuration inspection, including system settings, ACLs, DHCP, supported NAT syntax, SSH settings, and line access policy.
- A read-only configuration preview for supported system settings and additive VLAN creation.
- An initial xterm.js terminal that executes commands and displays complete responses.

The running-configuration view intentionally excludes passwords and other secret-bearing fields. Parser diagnostics do not expose raw unrecognized configuration lines. Configuration preview only generates a diff and commands for review; it does not apply changes to a device.

## Known limitations

This is an active migration, not yet full feature parity or a production-ready configuration manager. The following work remains:

- Native OS credential-vault integration (Keychain, Windows Credential Manager, and Linux Secret Service).
- Complete interface, VLAN, route, ACL, DHCP, NAT, SSH, and system configuration editing and validation.
- Safe apply orchestration, post-apply verification, rollback, backups, and history.
- Continuous terminal streaming, bounded output, cancellation, and operation progress.
- Broader IOS syntax coverage, including static, route-map, and VRF-specific NAT.
- Clean install/upgrade and packaged-app validation on all supported platforms.

The legacy Slint implementation is retained until replacement workflows have been reviewed and tested. Do not use this migration increment as evidence that device configuration writes, automatic rollback, or secure credential storage are available.
