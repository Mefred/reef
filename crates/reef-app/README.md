# Reef desktop application

The desktop host is built with Tauri 2. The frontend lives in `frontend/` and uses Svelte 5, TypeScript, and Vite.

## Prerequisites

- Rust toolchain matching the workspace's minimum supported version.
- Node.js and pnpm (the package manager version is pinned in `package.json`).
- Tauri's platform-specific prerequisites:
  - Linux: the WebKitGTK, GTK, app indicator, and build tool development packages required by Tauri 2.
  - Windows: Microsoft C++ Build Tools and WebView2.
  - macOS: Xcode Command Line Tools.

Consult the Tauri 2 prerequisites for the exact packages for the target distribution.

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

## Frontend checks

```sh
pnpm --dir frontend check
pnpm --dir frontend build
```

## Desktop package

```sh
pnpm tauri:build
```

The current migration increment provides the desktop shell and saved-profile read path. Device connections and the remaining device-management workflows are not yet wired into the new frontend.

## Migration status

The Tauri/Svelte frontend is being introduced in stages. The Windows icon source is committed as `icons/icon.ico`, and the 512×512 RGBA `icons/icon.png` is the source used by the Tauri icon generator. The current migration includes saved connection profiles, SSH/Telnet/serial session commands, device identification, and an initial xterm.js command console. Interface/VLAN/routing/configuration views and terminal streaming are not yet at Slint feature parity.

The `pnpm tauri:build` script generates the platform-specific icon variants from `icons/icon.png` before packaging.
