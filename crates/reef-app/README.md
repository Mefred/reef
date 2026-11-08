# Reef desktop application

Reef is a cross-platform desktop application for inspecting and managing Cisco IOS XE devices. The desktop shell is Tauri 2 and the UI is Svelte 5, TypeScript, and Vite. Device models, parsing, planning, transports, and persistence remain in the existing Rust workspace crates.

## Development prerequisites

- Rust toolchain matching the workspace minimum supported version.
- Node.js 22 and pnpm 9.15.4.
- Tauri 2 platform prerequisites:
  - Linux: GTK 3, WebKitGTK 4.1, app-indicator, librsvg, and build tools.
  - Windows: Microsoft C++ Build Tools and WebView2.
  - macOS: Xcode Command Line Tools.

See the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for distribution-specific setup.

## Install and run

From `crates/reef-app`:

```sh
pnpm install
pnpm --dir frontend install
pnpm tauri:dev
```

## Verify and package

```sh
pnpm --dir frontend check
pnpm --dir frontend build
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
pnpm tauri:build
```

The desktop migration workflow also builds the Tauri application on Linux, Windows, and macOS.

## Migration boundary

This change replaces the Slint desktop shell with Tauri 2 and Svelte. It does not replace the Rust device model, parser, planner, transport, or storage crates, and it does not attempt to implement every configuration-management feature.

The current Tauri application includes:

- SSH, Telnet, and serial connection profiles.
- Connect/disconnect, device identification, privilege escalation, and command execution.
- Explicit first-use SSH host-key trust.
- Device overview and live interface, VLAN, and static-route inspection.
- Sanitized running-configuration inspection for system settings, ACLs, DHCP, supported NAT syntax, SSH settings, and line access policy.
- A bounded xterm.js request/response terminal.
- A read-only configuration preview for hostname, domain name, additive DNS servers, and add-only VLAN creation.

The legacy Slint source, runtime dependency, and build integration have been removed. Tauri is the only desktop application entry point.

## Deliberate non-goals

The migration does not add new configuration-write functionality. Configuration preview remains review-only; it does not apply commands to devices. Native OS credential-vault integration, broader configuration editing, safe apply/verification/rollback, terminal streaming, and additional IOS parser coverage are separate product improvements and are not prerequisites for replacing the UI framework.
