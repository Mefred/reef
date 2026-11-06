# Reef Implementation Plan

A modern network device configuration and management application for Cisco IOS XE devices.

---

## 1. Project Goals, Scope, and Non-Goals

### Goals
- Build a desktop GUI application for configuring and managing Cisco IOS XE network devices.
- Support SSH, Telnet, and serial console connectivity.
- Auto-identify device models, IOS XE versions, hostnames, hardware, interfaces, and capabilities.
- Provide structured configuration management with validation, change planning, diffs, backups, and rollback via IOS XE `configure replace`.
- Include an integrated CLI terminal panel synchronized with device state.
- Initial platform support: Cisco IOS XE exclusively (Catalyst 9000, ISR 4000/1100, ASR 1000).
- Keep the configuration engine independent of GUI and transport layer.

### Scope (Initial Release)
- Device discovery and inventory (model, version, hostname, serial, interfaces).
- Interface management: physical/logical, IPv4/IPv6, VLANs/switchports, L2/L3.
- Static routing, ACLs, DHCP, NAT, SSH/device access, system settings.
- Configuration backup, preview diff, confirmation, and rollback.
- Integrated CLI terminal panel.
- Cross-platform desktop (Windows, macOS, Linux).

### Non-Goals (Explicitly Out of Scope)
- Classic IOS, IOS XR, NX-OS, or non-Cisco vendor support (future extensibility only, no abstraction built now).
- NETCONF/RESTCONF/YANG, SNMP, or API-based management (CLI-only for initial release).
- Multi-device orchestration, templates, or fleet-wide change deployment.
- Wireless/controller management, SD-WAN, or cloud integration.
- User management/role-based access within Reef itself (relies on device authentication).

---

## 2. Recommended Technology Stack

| Layer | Technology | Justification |
|-------|-----------|---------------|
| Application language | **Rust** | Memory safety, strong type system, excellent async ecosystem, cross-platform, serial/SSH/Telnet libraries available. |
| GUI framework | **Slint** | Declarative UI, Rust-first, cross-platform (desktop), lightweight, no heavy web engine dependency. |
| SSH | `ssh2-rs` / `tokio-ssh` | Mature Rust SSH implementations. |
| Telnet / Serial | `tokio-serial`, custom Telnet over tokio | Async serial and raw TCP Telnet support. |
| Async runtime | `tokio` | Standard async runtime for Rust networking/serial. |
| Config data model | `serde` + derive | Structured serialization/deserialization, validation. |
| Persistent storage | `sled` or SQLite via `rusqlite` | Device inventory, saved configs, connection profiles, session history. |
| Logging | `tracing` + `tracing-subscriber` | Structured, async-friendly logging with diagnostics. |
| Testing | `tokio-test`, `mockall`, `serial-test` | Unit/integration testing, mocked transports. |
| Packaging | Standalone Rust installer | Cross-platform desktop distribution (evaluate later). |

### Constraint Notes
- **Slint** is chosen for its Rust-native, declarative model and small footprint; it keeps GUI decoupled from backend logic.
- All configuration logic lives in pure Rust crates with no Slint dependency, enabling headless/testing use.
- Connection transports (SSH/Telnet/Serial) are abstracted behind a common `DeviceSession` trait; the GUI knows nothing about transport specifics.

---

## 3. Research Findings Summary

### Verified (from Cisco documentation)
- IOS XE has two config files: `running-config` (active RAM) and `startup-config` (NVRAM). Changes to running-config are immediate; `copy running-config startup-config` persists.
- Privilege levels: user EXEC (level 1, `>`) and privileged EXEC (level 15, `#`). `enable` escalates.
- Configuration modes: global config (`config t`), interface config (`interface ...`), line config (`line console 0` / `line vty ...`).
- Paging: `--- More ---` prompts appear on long output; must be handled (send space or disable with `terminal length 0`).
- `configure replace` + `archive config` provide rollback/checkpoint capability on IOS XE.
- Interface naming is platform/family-dependent: `GigabitEthernet1/0/1`, `TenGigabitEthernet1/1/1`, switch-number/slot/port on stacked switches.
- Console/SSH/Telnet handled via `line console 0`, `line vty`, `transport input` controls.
- ROMMON variables control boot behavior (`MANUAL_BOOT`, `ENABLE_BREAK`, `SWITCH_IGNORE_STARTUP_CFG`) — differs from classic `config-register`.

### Assumptions / Proposed Design Decisions (to be validated)
- `terminal length 0` reliably disables paging across IOS XE releases.
- `show version`, `show inventory`, `show ip interface brief`, `show ipv6 interface brief`, `show running-config` output is parseable enough for structured state.
- `configure replace` is available on all target IOS XE platforms/releases (may require archive config pre-configuration).

---

---

## 4. High-Level Architecture

Reef follows a layered, transport-independent architecture. The GUI never talks directly to a device; it talks to the Configuration Engine, which talks to a Session Manager, which talks to a Transport (SSH/Telnet/Serial).

```
┌─────────────────────────────────────────────────────────────┐
│                      GUI (Slint)                            │
│  Device List  │  Device Detail  │  Config Editor  │  CLI    │
└──────────────┴──────────────────┴──────────────────┴───────┘
                           │
                    ┌──────▼───────┐
                    │ Application  │  Orchestrates UI ↔ Engine
                    │   Controller │
                    └──────┬───────┘
                           │
                    ┌──────▼───────┐
                    │ Config Engine│  Models, validation, planning, diffs
                    │  (pure Rust) │  No GUI or transport dependency
                    └──────┬───────┘
                           │
                    ┌──────▼───────┐
                    │ Session Mgr  │  Device sessions, transport pooling
                    └──────┬───────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
        ┌─────▼─────┐ ┌────▼─────┐ ┌────▼─────┐
        │   SSH     │ │  Telnet  │ │  Serial  │
        │ Transport │ │Transport │ │ Transport│
        └───────────┘ └──────────┘ └──────────┘
```

### Component Responsibilities

**GUI Layer (Slint)**
- Device discovery/inventory list view.
- Per-device dashboard: identification, interface table, system settings.
- Forms for interface/IP/VLAN/routing/ACL/DHCP/NAT/SSH configuration.
- Configuration preview/diff viewer.
- Integrated terminal panel for CLI.
- Backup/restore and rollback UI.
- Purely presentational; emits intents (commands/requests) to the controller.

**Application Controller**
- Translates GUI intents into engine operations.
- Coordinates device connection lifecycle (connect/disconnect/reconnect).
- Maps device capability/features to available UI screens.
- Synchronizes CLI terminal output with engine state.

**Configuration Engine (pure Rust crate)**
- Structured data models for device config (interfaces, IP, VLANs, routes, ACLs, etc.).
- Parser: raw IOS XE CLI output → structured model.
- Generator: structured model → IOS XE CLI commands.
- Validator: checks for conflicts, invalid references, disruptive changes.
- Planner: produces a change set (add/modify/delete) and command sequence.
- Diff: compares current vs. desired state.
- Verifier: post-apply check that running-config matches intended state.
- Zero dependency on Slint or transport code.

**Session Manager**
- Manages active device sessions keyed by device ID.
- Acquires/releases transport connections.
- Handles authentication, privilege escalation (`enable`), prompt detection, paging control.
- Connection recovery / reconnection with backoff.
- Exposes `send_command`, `send_config`, `read_prompt` primitives.

**Transport Layer (SSH / Telnet / Serial)**
- Async connection, read/write, close.
- Each transport implements a common `DeviceTransport` trait.
- SSH: host key verification, password/keyboard-interactive auth.
- Telnet: raw TCP with NVT, IAC handling.
- Serial: baud, parity, stop bits, flow control, DTR/RTS.

---

## 5. Repository and Workspace Structure

```
reef/
├── Cargo.toml                 # Workspace root
├── rust-toolchain.toml
├── .cargo/config.toml
├── README.md
├── PLAN.md
├── docs/                      # Architecture decision records, research notes
│   └── architecture.md
├── crates/
│   ├── reef-app/              # Binary: Slint GUI + controller main entry
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── app.rs
│   │   │   ├── ui/            # Slint .slint files + generated bindings
│   │   │   ├── controller.rs
│   │   │   └── windows/
│   │   ├── ui/                # Slint UI source
│   │   └── Cargo.toml
│   ├── reef-engine/           # Library: config models, parsing, planning (pure Rust)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── model/         # Structured config data models
│   │   │   ├── parser/        # IOS XE CLI output → model
│   │   │   ├── generator/     # Model → IOS XE commands
│   │   │   ├── validator.rs
│   │   │   ├── planner.rs
│   │   │   ├── diff.rs
│   │   │   └── verifier.rs
│   │   └── Cargo.toml
│   ├── reef-session/          # Library: session & transport management
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── session.rs
│   │   │   ├── manager.rs
│   │   │   ├── transport.rs   # DeviceTransport trait
│   │   │   ├── ssh.rs
│   │   │   ├── telnet.rs
│   │   │   └── serial.rs
│   │   └── Cargo.toml
│   ├── reef-storage/          # Library: SQLite/sled persistence
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── db.rs
│   │   │   ├── models.rs      # Device profiles, saved configs
│   │   │   └── migrations.rs
│   │   └── Cargo.toml
│   └── reef-logging/          # Library: tracing setup
│       └── Cargo.toml
├── tests/
│   ├── integration/           # End-to-end against mock/real devices
│   └── mock_device/           # Mock IOS XE device simulator (for CI)
├── scripts/
│   └── build_release.sh
└── target/
```

### Crate dependency direction
```
reef-app → reef-controller (in app) → reef-engine, reef-session, reef-storage
reef-session → transports (ssh/telnet/serial)
reef-engine → none (pure, no GUI, no transport)
```

This ensures the configuration engine can be unit-tested in isolation without any device or GUI.

---

---

## 6. Device Connection and Session Architecture

### 6.1 Transport Abstraction

All transports implement a common async trait so the session layer is transport-agnostic:

```rust
pub trait DeviceTransport: Send + Sync {
    async fn connect(&mut self) -> Result<()>;
    async fn disconnect(&mut self) -> Result<()>;
    async fn send(&mut self, data: &str) -> Result<()>;
    async fn send_raw(&mut self, bytes: &[u8]) -> Result<()>;
    async fn read_until(&mut self, pattern: &str, timeout: Duration) -> Result<String>;
    async fn read_available(&mut self, timeout: Duration) -> Result<String>;
    fn is_connected(&self) -> bool;
    fn transport_type(&self) -> TransportType;
}
```

### 6.2 Transport Implementations

**SSH** (`ssh2-rs` wrapped in tokio, or `tokio-ssh`):
- Password and keyboard-interactive authentication.
- Host key verification (accept-known, warn-on-change, reject-unknown) with persistent known_hosts.
- Channel-based exec/shell; shell mode preferred for interactive IOS XE.
- Keepalives to detect dead sessions.

**Telnet** (custom over tokio TcpStream):
- NVT (Network Virtual Terminal) with IAC escape handling.
- No encryption — used only when SSH is unavailable; UI must mark sessions as insecure.
- Suboption negotiation (echo, suppress go-ahead, terminal type).

**Serial** (`tokio-serial`):
- Configurable baud rate (default 9600), data bits, parity, stop bits, flow control.
- DTR/RTS control for hardware handshake where supported.
- Essential for out-of-band management when network is down.

### 6.3 Session Lifecycle

```
Disconnected → Connecting → Authenticated → Privileged → Ready
                                ↓ (on failure)          ↓ (on drop)
                           Error state              Disconnected
```

States:
1. **Disconnected** — no transport established.
2. **Connecting** — transport handshake in progress.
3. **Authenticated** — logged in at user EXEC (`>`).
4. **Privileged** — `enable` completed, at privileged EXEC (`#`).
5. **Ready** — paging disabled (`terminal length 0`), ready for config operations.

Transitions include retry with exponential backoff on transient failure. A session auto-demotes to Authenticated if privilege escalation fails (read-only mode).

### 6.4 Session Manager

- Owns a map of `DeviceId → Arc<Mutex<Session>>`.
- `acquire(device_id)` returns a session, connecting if needed.
- `release(device_id)` returns session to pool or closes.
- Handles reconnection with backoff when a transport reports disconnect.
- Tracks session health (last activity, error count).
- Limits concurrent sessions per device to avoid exhausting VTY lines.

### 6.5 IOS XE Interaction Primitives

The session exposes higher-level operations built on the transport:

| Operation | Behavior |
|-----------|----------|
| `enable(password)` | Sends `enable`, handles password prompt, verifies `#` prompt. |
| `config_terminal()` | Sends `configure terminal`, verifies `(config)#` prompt. |
| `exit_config()` | Sends `end`, returns to privileged EXEC. |
| `send_command(cmd)` | Executes in privileged EXEC, captures output until prompt returns. Handles paging by sending space/`q` if `--- More ---` appears. |
| `send_config(cmd)` | Executes in global config mode, captures output. |
| `send_config_batch(cmds)` | Enters config mode once, sends all commands, exits; returns combined output. |
| `disable_paging()` | Sends `terminal length 0` once at session start. |
| `read_prompt()` | Detects current prompt pattern (`>`, `#`, `(config)#`, `(config-if)#`, Password:). |

### 6.6 Prompt and Paging Handling

- Prompt regexes detect user EXEC (`>`), privileged EXEC (`#`), and sub-modes.
- Paging: before long `show` commands, paging is disabled globally via `terminal length 0`. If `--- More ---` still appears, the reader auto-responds with space.
- Error detection: scan output for `% Invalid input`, `% Incomplete command`, `% Ambiguous command`, `% Error`, and surface as structured errors.

### 6.7 Connection Recovery

- Transport-level read/write timeouts trigger reconnect.
- On reconnect, re-authenticate and re-escalate privilege; re-disable paging.
- In-flight config operations are aborted and reported; the engine verifies state on next operation (idempotent diffs avoid re-applying already-applied changes).

### 6.8 Security Notes

- Credentials stored encrypted at rest (keyring integration) or in-memory only per session.
- SSH host keys verified; never auto-accept unknown hosts in production mode.
- Telnet sessions labeled insecure in UI; discouraged for production.
- Serial access requires physical presence; no network exposure.

---

---

## 7. IOS XE Identification and Compatibility Strategy

### 7.1 Identification Commands

The session collects device identity during initial connection using these privileged EXEC commands:

| Command | Purpose | Parsed Output |
|---------|---------|---------------|
| `show version` | IOS XE version, uptime, hardware, boot reason | version string, ROM version, uptime, config register |
| `show inventory` | Chassis, module, SFP serials and PIDs | model, serial, part number |
| `show hostname` (or parse from prompt) | Device hostname | hostname string |
| `show ip interface brief` | L3 interface summary | interface name, IP, status, protocol |
| `show ipv6 interface brief` | IPv6 interface summary | interface name, IPv6, status |
| `show vlan brief` | VLAN database | VLAN ID, name, status, ports |
| `show ip route summary` | Routing table overview | route counts by protocol |
| `show running-config` | Full config for baseline | parsed into structured model |

### 7.2 Device Identification Model

```rust
pub struct DeviceIdentity {
    pub hostname: String,
    pub iosxe_version: String,        // e.g., "17.12.0a"
    pub platform: String,             // e.g., "C9500-16X", "ISR4331"
    pub chassis_serial: String,
    pub processor: String,
    pub interfaces: Vec<InterfaceRef>,
    pub capabilities: DeviceCapabilities,
}

pub struct DeviceCapabilities {
    pub supports_l2_switchports: bool,
    pub supports_ipv6: bool,
    pub supports_static_routing: bool,
    pub supports_dynamic_routing: bool, // OSPF, BGP, EIGRP
    pub supports_nat: bool,
    pub supports_dhcp: bool,
    pub supports_acl: bool,
    pub max_vlans: Option<u16>,
    pub interface_naming: InterfaceNaming,
}
```

### 7.3 Interface Naming Variability

Naming differs by platform/family. The engine must not hardcode assumptions:

- **ISR 4000/1100**: `GigabitEthernet0/0/0`, `GigabitEthernet0/0/1`
- **Catalyst 9000 standalone**: `GigabitEthernet1/0/1`, `TenGigabitEthernet1/1/1`
- **Catalyst 9000 stacked**: `GigabitEthernet1/0/1` (switch/slot/port)
- **ASR 1000**: `GigabitEthernet0/0/0`, `TenGigabitEthernet0/0/0`
- Logical: `Vlan10`, `Loopback0`, `Tunnel0`, `Port-channel1`

The engine parses actual interface names from `show ip interface brief` / `show interfaces status` and treats them as opaque identifiers. The GUI presents them as-is.

### 7.4 Release/Platform Differences Handling

| Area | Variability | Handling Approach |
|------|-------------|-------------------|
| Command availability | Some commands only in newer XE releases | Feature-gate UI by detected capabilities; fallback to CLI panel |
| Interface types | Fixed vs modular, SFP vs RJ45 | Discovered from inventory; UI shows only present interfaces |
| VLAN support | L2 switches vs L3-only routers | `supports_l2_switchports` gates VLAN UI |
| NAT | Not on all platforms | `supports_nat` gates NAT screen |
| Configuration archive | `configure replace` may need pre-config | Detect availability; disable rollback UI if unsupported |
| Privilege levels | Custom levels possible | Default to level 15; detect if `enable` unavailable |

### 7.5 Compatibility Matrix (Initial Release)

| Product Family | Example Models | IOS XE Releases | Transport |
|----------------|---------------|-----------------|-----------|
| Catalyst 9000 | C9200, C9300, C9500 | 16.12+, 17.x | SSH, Telnet, Serial |
| ISR 4000 | ISR4321, ISR4331, ISR4431 | 16.x, 17.x | SSH, Telnet, Serial |
| ISR 1100 | ISR1100 series | 17.x | SSH, Telnet, Serial |
| ASR 1000 | ASR1001-X, ASR1002-X | 16.x, 17.x | SSH, Telnet, Serial |

### 7.6 Extensibility Without Overengineering

- The `DeviceCapabilities` struct is the single point for feature gating.
- Adding a new platform later means updating detection logic and capability flags — no GUI changes needed if UI is driven by capabilities.
- No abstract multi-vendor OS layer is built; the codebase assumes IOS XE semantics throughout. Future vendors would be separate crates, not forced into a common abstraction prematurely.

### 7.7 Assumptions to Validate

- `show version` output format is stable enough across 16.x/17.x to parse reliably.
- `terminal length 0` persists for the session and disables paging on all target releases.
- `configure replace` rollback is available on all initial-release platforms (may require `archive config` pre-configuration).

---

---

## 8. Configuration Data Models and Parsing Strategy

### 8.1 Design Principle

The configuration engine uses structured data models as the source of truth. The UI builds models, the engine parses device output into models, compares them, generates commands, and verifies results. The GUI never generates CLI commands directly.

### 8.2 Core Data Models

```rust
// Top-level device configuration
pub struct DeviceConfig {
    pub hostname: Option<String>,
    pub ip_domain_name: Option<String>,
    pub ip_name_server: Vec<String>,
    pub interfaces: HashMap<String, InterfaceConfig>,
    pub vlans: HashMap<u16, VlanConfig>,
    pub ip_routes: Vec<StaticRoute>,
    pub ipv6_routes: Vec<StaticRoute>,
    pub acls: HashMap<String, AclConfig>,
    pub dhcp_pools: HashMap<String, DhcpPool>,
    pub nat: Option<NatConfig>,
    pub ssh: Option<SshConfig>,
    pub line_configs: HashMap<String, LineConfig>,
    pub banner: Option<String>,
}

pub struct InterfaceConfig {
    pub name: String,                    // e.g., "GigabitEthernet1/0/1"
    pub description: Option<String>,
    pub enabled: bool,
    pub mode: InterfaceMode,             // Access, Trunk, Routed, L2
    pub access_vlan: Option<u16>,
    pub trunk_vlans: Vec<u16>,
    pub native_vlan: Option<u16>,
    pub ipv4: Option<Ipv4Address>,       // address/mask
    pub ipv6: Option<Ipv6Address>,
    pub mtu: Option<u16>,
    pub duplex: Option<Duplex>,
    pub speed: Option<Speed>,
}

pub enum InterfaceMode { Access, Trunk, Routed, L2 }

pub struct VlanConfig {
    pub id: u16,
    pub name: String,
    pub enabled: bool,
}

pub struct StaticRoute {
    pub prefix: IpNetwork,               // destination
    pub next_hop: IpAddr,
    pub admin_distance: Option<u16>,
    pub permanent: bool,
}

pub struct AclConfig {
    pub name: String,
    pub acl_type: AclType,               // Standard, Extended
    pub rules: Vec<AclRule>,
}

pub struct AclRule {
    pub action: Action,                  // Permit, Deny
    pub protocol: Option<Protocol>,
    pub source: AddressSpec,
    pub destination: AddressSpec,
    pub ports: Option<PortSpec>,
}

pub struct DhcpPool {
    pub name: String,
    pub network: IpNetwork,
    pub default_router: Option<IpAddr>,
    pub dns_servers: Vec<IpAddr>,
    pub lease_time: Option<Duration>,
}

pub struct NatConfig {
    pub inside_interfaces: Vec<String>,
    pub outside_interfaces: Vec<String>,
    pub pools: Vec<NatPool>,
    pub rules: Vec<NatRule>,
}

pub struct SshConfig {
    pub enabled: bool,
    pub version: SshVersion,             // 1, 2, or Both
    pub timeout: Option<u16>,
    pub authentication_retries: Option<u16>,
}

pub struct LineConfig {
    pub line: String,                    // "console 0", "vty 0 15"
    pub transport: Vec<Transport>,       // ssh, telnet, all
    pub password: Option<Secret>,
    pub login: bool,
}
```

### 8.3 Parsing Strategy

IOS XE CLI output is semi-structured text. The parser converts `show` and `show running-config` output into the models above.

**Parsing approach:**
- Line-by-line parsing with mode detection (global config, interface config, line config).
- Regex-based extraction for known patterns (IP addresses, VLAN IDs, interface names).
- Indentation-aware for nested config (e.g., `interface` blocks, `ip access-list` blocks).
- Unknown/unhandled lines are preserved as raw config snippets so nothing is silently lost.

**Commands parsed:**
- `show version` → DeviceIdentity
- `show inventory` → hardware/serial
- `show ip interface brief` / `show ipv6 interface brief` → interface operational state
- `show vlan brief` → VLAN table
- `show running-config` → full DeviceConfig (baseline)
- `show ip route` / `show ipv6 route` → routing table
- `show ip access-lists` → ACLs
- `show ip dhcp pool` → DHCP pools
- `show ip nat translations` / `show running-config | include nat` → NAT

### 8.4 Parser Structure

```
reef-engine/src/parser/
├── mod.rs
├── version.rs        # show version, show inventory
├── interfaces.rs     # show ip interface brief, show interfaces status
├── vlans.rs          # show vlan brief
├── running_config.rs  # show running-config (block-based)
├── routes.rs         # show ip route
├── acls.rs           # show ip access-lists
├── dhcp.rs           # show ip dhcp pool
└── nat.rs            # show ip nat
```

Each parser returns `Result<Model, ParseError>` where `ParseError` includes the raw line and position for diagnostics.

### 8.5 Serialization / Persistence

- Device configs serialized with `serde` to JSON for storage and diff comparison.
- Saved configuration snapshots stored per device with timestamps.
- Secret fields (passwords, keys) use a `Secret<T>` wrapper that is excluded from logs/diffs by default.

### 8.6 Limitations and Unknowns

- IOS XE `show running-config` output order is mostly stable but not guaranteed; parsers must be order-tolerant where possible.
- Some features (dynamic routing, advanced NAT, QoS) are intentionally out of initial scope and will parse as "unsupported" raw blocks rather than fail.
- Different releases may format `show` output slightly; parsers should degrade gracefully (return partial model + warning) rather than hard-fail.

---

---

## 9. Configuration Planning, Generation, Application, Verification Workflow

### 9.1 Overview

The engine follows a deterministic, four-stage workflow: **Plan → Preview → Apply → Verify**. No command is sent to a device without first generating a diff and obtaining confirmation (except explicit CLI panel use).

### 9.2 Stage 1: Plan (Change Detection)

Input: current device config model (from parser) + desired config model (from UI or CLI).
Output: a `ChangeSet` describing additions, modifications, and deletions.

```rust
pub enum ChangeOp { Add, Modify, Remove, NoChange }

pub struct ChangeSet {
    pub interface_changes: Vec<(String, ChangeOp)>,
    pub vlan_changes: Vec<(u16, ChangeOp)>,
    pub route_changes: Vec<ChangeOp>,
    pub acl_changes: Vec<(String, ChangeOp)>,
    pub ... // per-section changes
    pub is_disruptive: bool,      // e.g., removing an IP from mgmt interface
    pub affects_connectivity: bool,
}
```

- Engine compares field-by-field, not line-by-line.
- Interface IP changes on the management interface flagged as `affects_connectivity`.
- Change set is serializable for preview UI.

### 9.3 Stage 2: Preview (Diff)

The engine renders a human-readable diff:

```
Interface GigabitEthernet1/0/1:
  [MODIFY] ip address 10.0.0.1/24 -> 10.0.0.2/24
  [ADD]    description Uplink-to-Core
VLAN 10:
  [ADD]    name Sales
```

- Diff is shown in GUI before any apply.
- Disruptive changes require explicit confirmation checkbox.
- Preview can be exported as text.

### 9.4 Stage 3: Generate (Command Generation)

Pure function: `ChangeSet → Vec<Command>`. Commands are ordered and grouped by config mode.

```rust
pub struct CommandBatch {
    pub enter_config_mode: bool,       // configure terminal
    pub commands: Vec<String>,
    pub exit_config_mode: bool,        // end
    pub save_to_startup: bool,         // copy run start
}
```

Generation rules (examples):
- Interface IP change: `interface GigabitEthernet1/0/1` → `ip address 10.0.0.2 255.255.255.0`
- VLAN add: `vlan 10` → `name Sales`
- Route add: `ip route 10.1.0.0 255.255.255.0 10.0.0.1`
- Each command is idempotent where possible (IOS XE accepts re-entering same config).

Commands are NOT sent yet. They are presented as the "command plan."

### 9.5 Stage 4: Apply

The session sends commands via `send_config_batch`:
1. `configure terminal`
2. Each generated command, in order
3. `end`
4. Optionally `copy running-config startup-config`

Error handling:
- If any command returns `% Invalid input`, `% Ambiguous command`, or `% Incomplete command`, apply halts.
- The session captures output; engine parses errors and reports which command failed.
- Partial application is possible; engine verifies state afterward to detect drift.

### 9.6 Stage 5: Verify

After apply, engine re-reads `show running-config` (or targeted `show` commands) and re-parses into a model. Compares against desired model:

- **Verified** — model matches desired state.
- **Drift** — model differs; report specific mismatches.
- **Unverifiable** — parse failed; require manual CLI check.

Verification result is stored with the change record for audit.

### 9.7 Rollback Strategy

IOS XE native rollback via `configure replace`:

1. Before apply, engine saves current running-config to a local snapshot file.
2. If apply fails verification or user requests rollback:
   - Engine saves current running-config to device flash via `archive config` (if archive configured).
   - Or pushes the saved snapshot back via `configure replace <url>`.
   - `configure replace` triggers a rollback of the running config to the saved state.
3. Rollback UI lists available checkpoints (`show archive`).

Limitation: `configure replace` requires the configuration archive feature enabled on the device; if unavailable, rollback falls back to "manual reverse commands" (engine generates inverse config commands).

### 9.8 Change History / Audit

Each change recorded locally:
- Device ID, timestamp, user, change set summary, command plan, apply result, verification result, rollback status.
- Stored in reef-storage (SQLite).
- GUI provides history list per device with diff view.

### 9.9 CLI Panel Integration

- Integrated terminal sends raw commands directly to session.
- After any CLI command, engine marks device state as "potentially dirty" and offers a re-sync (re-parse running-config).
- CLI and GUI changes share the same session; no command buffering conflicts (mutex on session).

### 9.10 Assumptions to Validate

- `configure replace` is available and functional on target platforms (may need `archive config` set up first).
- Re-parsing `show running-config` after apply reliably reflects applied changes.
- IOS XE accepts re-entering identical configuration commands without error (idempotency).

---

---

## 10. GUI Structure and Major Application Screens

### 10.1 UI Framework: Slint

Slint is used for the desktop GUI. Key properties:
- Declarative `.slint` markup language with Rust bindings.
- Cross-platform desktop (Windows, macOS, Linux).
- Lightweight — no Chromium/web engine dependency.
- Callback-driven: UI emits events; Rust controller handles business logic.
- The GUI is purely presentational. It never talks to devices or generates CLI commands directly.

### 10.2 Application Layout

```
┌─────────────────────────────────────────────────────────────────┐
│ Reef  [Devices] [Connect] [Settings]                              │
├──────────────┬───────────────────────────────────────────────────┤
│ Sidebar      │ Main Content Area                                  │
│ - Device list│                                                   │
│   (tree)     │ [Active Device: hostname]                          │
│ - Connect new│ [Tabs: Overview | Interfaces | IP | VLAN | ...]    │
│ - History    │                                                   │
│ - CLI panel  │ [Configuration Preview / Diff]                     │
│              │ [Apply] [Rollback] [Cancel]                        │
└──────────────┴───────────────────────────────────────────────────┘
```

### 10.3 Major Screens

**A. Device Inventory (Sidebar + Landing)**
- List of discovered/saved devices with status (connected, disconnected, error).
- "Add Device" dialog: hostname/IP, transport type (SSH/Telnet/Serial), port, credentials.
- Connection status indicator per device.
- Recent history list with timestamps.

**B. Device Overview Dashboard**
- Device identity: hostname, model, IOS XE version, serial, uptime.
- Interface summary table (name, type, status, IP, protocol).
- Quick actions: connect, disconnect, backup config, run diagnostics.
- System settings: hostname, domain, DNS, NTP, banner.

**C. Interfaces Screen**
- Table of all physical and logical interfaces discovered from device.
- Per-interface edit: description, enable/shutdown, speed, duplex, MTU.
- Mode selector: Access, Trunk, Routed, L2.
- Access VLAN assignment; trunk allowed VLANs; native VLAN.
- Add/remove logical interfaces (Loopback, VLAN SVI, Port-channel).
- Changes reflected in preview before apply.

**D. IP Addressing Screen (IPv4 & IPv6)**
- Per-interface IP address management: add/remove/change addresses.
- Secondary addresses support.
- Static route table: destination, next-hop, admin distance.
- Route add/edit/delete.
- IPv6 addressing where device capability supports it.

**E. VLANs Screen**
- VLAN list: ID, name, status, member ports.
- Create/edit/delete VLANs.
- Assign interfaces to VLANs (access mode) or trunk membership.
- VLAN database operations.

**F. Routing Screen**
- Static routes (IPv4/IPv6): prefix, next-hop, distance, permanent.
- Default route management.
- Dynamic routing summary (read-only where supported; detailed config via CLI panel).
- Route validation: unreachable next-hop warnings.

**G. ACLs Screen**
- ACL list: name, type (standard/extended), rule count.
- Rule editor: action, protocol, source, destination, ports.
- Apply ACL to interface (in/out).
- Preview of rendered `ip access-list` commands.

**H. DHCP Screen**
- DHCP pools: name, network, default router, DNS, lease time.
- Excluded addresses.
- Enable/disable DHCP service on device.

**I. NAT Screen**
- Inside/outside interface designation.
- NAT pools and rules.
- Source NAT / NAT overload configuration.
- NAT translations summary (read-only).

**J. SSH & Device Access Screen**
- SSH enable/disable, version (1/2/both), timeout, retries.
- Line configuration: console 0, vty ranges.
- Transport input: ssh, telnet, all, none.
- Login authentication method.

**K. Configuration Preview / Diff**
- Shows computed diff between current and desired config.
- Lists commands to be executed.
- Disruptive change warnings (e.g., management interface IP change).
- Confirmation checkbox required before Apply for destructive changes.

**L. Backup / Restore / Rollback**
- Backup running-config to local file.
- List of saved backups with timestamps.
- Restore from backup (generates `configure replace` plan).
- View archive checkpoints on device (`show archive`).
- Rollback confirmation.

**M. Integrated CLI Panel**
- Docked terminal at bottom or separate window.
- Sends raw commands to active device session.
- Output displayed monospaced; scrollback.
- "Sync" button: re-parse device state after CLI changes.
- Warning banner: CLI changes bypass GUI validation.

### 10.4 UI State Flow

1. User selects device → controller connects via Session Manager.
2. Engine fetches and parses current config → GUI populates screens.
3. User edits fields → controller builds desired model in memory.
4. User clicks "Preview" → engine computes diff → GUI shows preview dialog.
5. User confirms → controller calls engine to generate commands → session applies.
6. Engine verifies → GUI shows success/drift.
7. Any CLI use marks state dirty → GUI offers re-sync.

### 10.5 Design Constraints

- No screen assumes a specific Cisco model; all data driven by discovered capabilities.
- Transport type (SSH/Telnet/Serial) is abstracted; UI shows connection status only.
- Configuration engine has zero Slint dependency; GUI depends on engine, not reverse.
- Long-running operations (connect, apply) show progress indicators and do not block UI thread.

---

---

## 11. Security and Configuration Safety Considerations

### 11.1 Credential Handling

- **Device credentials** (username, password, enable secret, SSH keys) are held in memory only during an active session; never written to disk in plaintext.
- Optional integration with OS credential stores (macOS Keychain, Windows Credential Manager, Linux Secret Service) for persisting credentials encrypted at rest.
- A "remember password" toggle is opt-in per device; if declined, credentials are prompted each connection and dropped on disconnect.
- Secret fields in config models (`Secret<T>` wrapper) are masked in logs, diffs, and UI (shown as `****`).
- Config files saved locally by Reef may contain device secrets; these files are encrypted or access-restricted where possible, and the user is warned before saving.

### 11.2 Transport Security

- **SSH** is the preferred transport. Host keys are verified against a `known_hosts` store:
  - Unknown host: connection refused unless user explicitly accepts (with fingerprint shown).
  - Changed host key: big warning; connection blocked until user confirms.
- **Telnet** is unencrypted. UI marks Telnet sessions with an "insecure" label and shows a warning banner when connected. Telnet is not disabled entirely because legacy devices may require it.
- **Serial console** requires physical access; no network exposure. UI indicates local serial sessions.
- SSH keepalives enabled to detect dead sessions and prevent stale privileged sessions.

### 11.3 Privilege Escalation

- Reef connects at user EXEC level, then escalates with `enable` if an enable secret is provided.
- If `enable` fails, the session degrades to read-only mode; UI indicates limited access.
- No hardcoded/default credentials anywhere in the codebase.
- `enable` password treated as a secret, separate from login password.

### 11.4 Configuration Safety

**Backups**
- Before any apply operation, Reef automatically saves the current `running-config` to a local backup file (timestamped, per device).
- Backups stored in reef-storage; user can browse, download, or restore.
- Backup is taken even if the user does not explicitly request it (default on), but can be disabled in settings.

**Change Preview & Diff**
- Every GUI-initiated change produces a diff preview before execution.
- User reviews planned commands and must click Apply.
- No silent configuration changes.

**Confirmation for Disruptive Changes**
- The engine flags changes as `affects_connectivity` when they touch the management interface (the interface Reef is connected through).
- Examples: changing IP/subnet on mgmt interface, shutting mgmt interface, changing VLAN of mgmt interface, changing SSH/transport settings that could lock out access.
- These require an explicit "I understand this may disconnect me" checkbox before Apply.
- Non-disruptive changes (e.g., adding a VLAN, adding a loopback IP) apply without extra confirmation.

**Rollback**
- Primary rollback: IOS XE `configure replace` using a pre-change snapshot saved to device flash (`archive config`).
- If archive not configured: Reef keeps a local snapshot and can push it back via `configure replace <local_url>` if the device supports file copy from the management station, or falls back to generating reverse-config commands.
- Rollback UI lists available checkpoints and allows one-click restore with confirmation.
- After rollback, engine re-verifies state.

**Connectivity Loss Protection**
- When applying a change to the management interface, Reef offers a "confirm connectivity" timer: after apply, the user must re-confirm reachability within a window (e.g., 60 seconds); if no confirmation, Reef attempts automatic rollback to the pre-change snapshot.
- This is a best-effort safety net; if the session drops, Reef attempts reconnect and rollback.

### 11.5 Audit and History

- Every apply, rollback, and backup is logged locally with: device ID, timestamp, user, change summary, command plan, result (success/failure/drift), and rollback status.
- History view per device in GUI; logs written via `tracing` to file for diagnostics.
- Logs exclude secret values (masked).

### 11.6 CLI Panel Safeguards

- CLI commands bypass the validation/planning engine.
- A warning banner appears when the CLI panel is active: "Manual CLI changes bypass validation. Use Sync to refresh state."
- After CLI use, the device is marked "dirty"; GUI offers a re-sync before any GUI-initiated apply to avoid overwriting manual changes unknowingly.

### 11.7 Threat Model Summary

| Threat | Mitigation |
|--------|-----------|
| Credential theft from disk | In-memory only; optional OS keychain; opt-in remember |
| MITM on SSH | Host key verification with known_hosts |
| Accidental lockout | Diff preview, disruptive-change confirmation, auto-rollback timer |
| Unintended config overwrite | Preview, CLI dirty tracking, re-sync prompt |
| Malicious config file | Backups stored locally; user-controlled restore |
| Telnet eavesdropping | Insecure warning banner; SSH preferred |

### 11.8 Assumptions

- `configure replace` rollback is available on target devices; if not, rollback falls back to reverse commands (less reliable).
- The user is responsible for backing up critical configs outside Reef as well; Reef backups are convenience, not sole disaster recovery.

---

---

## 12. Testing Strategy

### 12.1 Testing Pyramid

```
        /\
       /  \   E2E (real hardware / high-fidelity mock)
      /____\
     /      \  Integration (mocked transports, engine + session)
    /________\
   /          \ Unit (pure engine parsers, models, planners)
  /____________\
```

### 12.2 Unit Tests (reef-engine, reef-session, reef-storage)

**Configuration engine (pure Rust, no device needed):**
- Parser unit tests: feed captured `show version`, `show ip interface brief`, `show running-config` text → assert structured model output.
- Parser edge cases: empty output, truncated output, unknown lines, unusual interface names.
- Model comparison: diff between two configs → correct change set.
- Command generator: given change set → assert exact IOS XE command strings.
- Validator: catch invalid combos (e.g., trunk native VLAN mismatch, IP on L2 interface).
- Diff renderer: assert human-readable diff text.

**Session layer:**
- `DeviceTransport` mock implementations for testing session logic without real transports.
- Prompt detection regex tests against sample prompts (`>`, `#`, `(config-if)#`).
- Paging handling: simulate `--- More ---` in stream, verify auto-response.
- Reconnect with backoff logic.

**Storage:**
- CRUD on device profiles, saved configs, history.
- Migration tests between schema versions.

### 12.3 Mock Device Simulator

A mock IOS XE device (TCP/SSH/Telnet/Serial) for CI and local testing:
- Listens on a port, speaks a simplified IOS XE CLI.
- Pre-defined device identity (fixed `show version` output).
- Maintains an in-memory running-config; accepts `configure terminal`, commands, `end`.
- Supports paging simulation (`terminal length`, `--- More ---`).
- Supports `show running-config`, `show ip interface brief`, `show vlan brief`, `show ip route`, etc.
- Can inject errors (`% Invalid input`, `% Ambiguous command`).
- Configurable disconnect/reconnect behavior.
- Implemented as a separate crate or test binary (`tests/mock_device/`).

**Why a mock:** Enables full GUI + engine + session integration tests without physical hardware. Runs in CI on every commit.

### 12.4 Integration Tests

- **reef-session + mock device:** connect, authenticate, enable, send config batch, verify running-config updated.
- **reef-engine + mock device:** end-to-end plan→preview→apply→verify against mock.
- **GUI + mock device:** Slint UI tests (slint testing utilities) driving a mock device through flows: add device, connect, edit interface, preview diff, apply, verify.
- Transport switching: same session logic over SSH mock vs Telnet mock vs serial mock (pty-based).

### 12.5 End-to-End (Real Hardware)

- Validation against real Cisco IOS XE devices in a lab environment (Catalyst 9000, ISR 4000).
- Test matrix: SSH connect, Telnet connect, serial connect; config apply; rollback; paging on long output; reconnect after network disconnect.
- Manual exploratory testing; not automated in CI (requires hardware).

### 12.6 Test Data (Fixtures)

Capture real `show` command output from representative IOS XE devices and store as text fixtures under `tests/fixtures/`:
- `show_version_c9300.txt`, `show_version_isr4331.txt`
- `show_ip_interface_brief_c9300.txt`
- `show_running_config_sample.txt`
- `show_vlan_brief.txt`

Parser unit tests run against these fixtures to guard against regressions when IOS XE output format changes.

### 12.7 CI Pipeline

- `cargo test --workspace` on every PR (unit + integration against mock).
- `cargo clippy` and `cargo fmt --check`.
- Mock device test binary included in CI.
- No real-hardware tests in CI (hardware not available in CI environment).

### 12.8 Coverage Targets

- `reef-engine`: ≥ 90% (pure logic, highly testable).
- `reef-session`: ≥ 80% (transport-agnostic logic).
- `reef-app` GUI: lower coverage target; focus on controller logic, not Slint rendering.

### 12.9 Assumptions

- Mock device behavior approximates real IOS XE closely enough to catch logic errors; real hardware validation still required before release.
- Slint provides widget testing utilities sufficient for GUI integration tests.

---

---

## 13. Phased Implementation Roadmap

### Phase 0: Foundation (Weeks 1–2)
**Goal:** Workspace, core crates, transport scaffolding, no GUI.

Milestones:
- Rust workspace with `reef-engine`, `reef-session`, `reef-storage`, `reef-app` crates.
- `DeviceTransport` trait defined; stub SSH/Telnet/Serial implementations.
- Basic session manager (connect/disconnect, send command).
- Logging (`tracing`) and config (serde) set up.
- `cargo test --workspace` passes (empty/stub tests).

Acceptance criteria:
- Workspace compiles; all crates build.
- Can instantiate a (stub) session and call `send`/`read`.

### Phase 1: Connection & Identification (Weeks 3–5)
**Goal:** Connect to real/mock IOS XE over SSH, identify device.

Milestones:
- Working SSH transport (password auth, host key check).
- Telnet and Serial transports implemented.
- Prompt detection, paging control (`terminal length 0`), privilege escalation (`enable`).
- Parsers for `show version`, `show inventory`, `show hostname`.
- Device identity model populated from live/mock device.

Acceptance criteria:
- Connect over SSH to mock device; retrieve and display hostname, version, model, serial.
- Reconnect with backoff works.
- Errors (bad password, unreachable host) surfaced cleanly.

### Phase 2: Configuration Engine Core (Weeks 6–8)
**Goal:** Parse, model, diff, generate commands — pure Rust, testable without devices.

Milestones:
- Structured config models (interfaces, IP, VLANs, routes, ACLs, etc.).
- Parser for `show running-config` and key `show` commands.
- Change set computation (add/modify/remove).
- Command generator (model → IOS XE commands).
- Validator for basic conflicts.
- Unit tests against captured fixtures.

Acceptance criteria:
- Parser turns fixture `show running-config` into model.
- Diff between two models produces correct change set.
- Generator emits syntactically valid IOS XE commands.
- ≥ 90% unit test coverage on `reef-engine`.

### Phase 3: Apply & Verify (Weeks 9–10)
**Goal:** Send commands to device, verify result, basic rollback.

Milestones:
- `send_config_batch` on session (config mode, commands, exit).
- Apply workflow: plan → generate → apply → re-parse → verify.
- Drift detection after apply.
- Local backup before apply (save running-config to file).
- `configure replace` rollback support (where available).

Acceptance criteria:
- Apply a simple interface change to mock device; verification confirms.
- Failed command halts batch and reports error.
- Rollback restores pre-change config on mock.

### Phase 4: GUI Shell (Weeks 11–13)
**Goal:** Slint desktop app, device list, connect, overview.

Milestones:
- Slint main window, sidebar, device list.
- "Add Device" dialog (host, transport, credentials).
- Connection status per device.
- Overview dashboard (identity, interface summary).
- Integrate with session manager and engine.

Acceptance criteria:
- App launches, shows device list.
- User adds device, connects over SSH to mock, sees overview data.
- Disconnect works; status updates.

### Phase 5: Configuration Screens (Weeks 14–17)
**Goal:** GUI forms for interfaces, IP, VLANs, routes, ACLs, DHCP, NAT, SSH, system.

Milestones:
- Interface screen (table + edit form).
- IP addressing (IPv4/IPv6, static routes).
- VLANs screen.
- ACL screen.
- DHCP, NAT, SSH/access, system settings screens.
- Preview/diff dialog before apply.
- Apply/apply-and-save buttons with verification result display.

Acceptance criteria:
- User edits interface IP in GUI, previews diff, applies to mock device, sees success.
- Each screen gated by detected device capabilities.

### Phase 6: Safety, Backup, CLI (Weeks 18–20)
**Goal:** Backups, rollback UI, history, integrated CLI terminal.

Milestones:
- Backup/restore screen with timestamped local backups.
- Rollback UI listing checkpoints.
- Change history/audit view.
- Integrated CLI panel (docked terminal).
- Disruptive-change confirmation and connectivity-loss safety timer.

Acceptance criteria:
- Backup taken automatically before apply.
- Rollback restores config via UI.
- CLI panel sends commands to active session; output displayed.
- Dirty-state warning after CLI use.

### Phase 7: Polish, Packaging, Release (Weeks 21–24)
**Goal:** Cross-platform builds, installer, documentation, real-hardware validation.

Milestones:
- Release builds for Windows, macOS, Linux.
- Packaging (installer / app bundle / .deb or tarball).
- Real-device lab testing (Catalyst 9000, ISR 4000).
- Bug fixes from lab testing.
- User documentation (connect, configure, rollback, safety).

Acceptance criteria:
- Application installs and runs on all three platforms.
- Successful configuration changes validated on real IOS XE hardware.
- Release artifacts published.

---

## 14. Major Technical Risks, Unknowns, and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `show` command output format varies across IOS XE releases | High | Medium | Parser degrades gracefully; partial results + warnings; fixtures per release; don't hard-fail on unknown lines |
| `configure replace` / archive not available on some devices | Medium | High | Detect availability; fall back to local snapshot + reverse commands; clearly indicate rollback unsupported |
| Accidental management-interface lockout | Low | Critical | Disruptive-change confirmation; auto-rollback timer; keep serial/console as out-of-band fallback |
| Paging handling fragile on long output | Medium | Medium | `terminal length 0` upfront; auto-respond to `--- More ---`; test with long configs |
| SSH host-key / auth compatibility issues | Medium | Medium | Support password and keyboard-interactive; known_hosts with accept/warn/reject modes; clear error messages |
| GUI blocks on slow device operations | Medium | Medium | All transport calls async; Slint spawned on async runtime; progress indicators |
| Secret leakage in logs/diffs | Low | Critical | `Secret<T>` type; masking in logs/UI; audit log excludes secrets |
| Mock device doesn't match real IOS XE behavior | Medium | High | Real-device lab phase before release; fix mismatches found in lab |

### Key Unknowns (to validate in Phase 1–2)
1. Exact `show version`/`show inventory` output format across ISR 4000, Catalyst 9000, ASR 1000 — validate with real fixtures.
2. Whether `terminal length 0` persists across the session on all target releases.
3. Whether `configure replace` works without pre-configuring `archive config` on all platforms.
4. Serial port behavior and baud-rate defaults across devices.

---

## Appendix: Verified Facts vs. Assumptions

**Verified (Cisco documentation):**
- `running-config` vs `startup-config`; `copy running-config startup-config` persists.
- Privilege levels 1 (`>`) and 15 (`#`); `enable` escalates.
- Config modes: global, interface, line.
- Paging via `--- More ---`; `terminal length 0` disables.
- `configure replace` + `archive config` for rollback.
- Interface naming varies by platform (switch/slot/port).
- ROMMON variables (not `config-register`) control boot on Catalyst 9000 XE.

**Assumptions / Design Decisions (to validate):**
- `terminal length 0` reliably disables paging on all target releases.
- `configure replace` available on all initial-release platforms.
- `show running-config` parseable enough for structured state.
- Re-parsing after apply reflects applied changes.
- Idempotent: re-entering same config command is accepted.

---

*End of Implementation Plan. This document is planning and research only; no application code has been written.*