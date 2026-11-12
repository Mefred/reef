# Reef desktop application

Reef is a native desktop application for inspecting and managing Cisco IOS XE
devices. The shell is built with [wgpui](https://crates.io/crates/wgpui) 0.4
(GPUI-style Rust UI on winit + wgpu) and renders directly with the operating
system's GPU surface. There is no webview, JavaScript bridge, Node.js, or
installer.

Device models, parsing, planning, transports, and persistence remain in the
existing Rust workspace crates. The UI talks to the thread-affine
`SessionActor`, portable `Storage`, and `AppDiagnostics` directly.

## Requirements

- Rust 1.95 or newer (the workspace pins the `stable` channel).
- Linux desktop development packages for winit/wgpu, for example on Debian or
  Ubuntu:

  ```sh
  sudo apt-get install -y \
    build-essential pkg-config libssl-dev \
    libxkbcommon-dev libwayland-dev libx11-xcb-dev libxcb1-dev \
    libfontconfig1-dev libfreetype6-dev libasound2-dev
  ```

- Windows x64 cross-builds use MinGW-w64:

  ```sh
  rustup target add x86_64-pc-windows-gnu
  sudo apt-get install -y gcc-mingw-w64-x86-64
  ```

## Run

```sh
cargo run -p reef-app
```

## Verify

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace
```

## Build the portable Windows executable

### Linux cross-build (MinGW)

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt-get install -y gcc-mingw-w64-x86-64
```

### Windows native build

```sh
rustup target add x86_64-pc-windows-gnu
```

### Enabling legacy SSH key-exchange algorithms

libssh2 compiles out the legacy SHA1-based key-exchange algorithms
(`diffie-hellman-group14-sha1`, `diffie-hellman-group-exchange-sha1`) by
default — they are behind `#ifdef LIBSSH2_KEX_SHA1_ENABLE`. Many Cisco IOS
devices require these, so you must enable them when building for Windows:

**Option 1: `.cargo/config.toml` (recommended, persists across builds)**

Add to `.cargo/config.toml` (already configured in this repository):

```toml
[env]
CFLAGS_X86_64_PC_WINDOWS_GNU = "-DLIBSSH2_KEX_SHA1_ENABLE"
```

**Option 2: Environment variable**

```sh
CFLAGS_X86_64_PC_WINDOWS_GNU="-DLIBSSH2_KEX_SHA1_ENABLE" \
  cargo build --release --target x86_64-pc-windows-gnu -p reef-app
```

Then build:

```sh
cargo build --release --target x86_64-pc-windows-gnu -p reef-app
```

The binary is written to:

```text
target/x86_64-pc-windows-gnu/release/reef.exe
```

Copy that executable, `portable.marker`, and the documentation files into the
same directory on a USB drive. Reef resolves `Config/`, `Data/`, and `Logs/`
relative to the executable, so the drive letter can change between machines.
No administrator rights or runtime installation are required.

The Windows binary imports only system DLLs; no MinGW runtime DLLs need to be
copied alongside it.

## Features

- SSH, Telnet, and serial connection profiles.
- Connect/disconnect, device identification, privilege escalation, and command
  execution.
- Explicit first-use SSH host-key trust.
- Device overview and live interface, VLAN, and static-route inspection.
- Sanitized running-configuration inspection for system settings, ACLs, DHCP,
  supported NAT syntax, SSH settings, and line access policy.
- A bounded request/response terminal with backend session recording.
- Portable diagnostics status, log browsing, test notes, and ZIP export.
- A read-only configuration preview for hostname, domain name, additive DNS
  servers, and add-only VLAN creation.

## Deliberate non-goals

The migration does not add new configuration-write functionality. Configuration
preview remains review-only; it does not apply commands to devices, and
startup-configuration writes are permanently disabled. Native OS credential
vault integration, broader configuration editing, safe apply/verification/
rollback, terminal streaming, and additional IOS parser coverage are separate
product improvements.
