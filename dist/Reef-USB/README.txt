Reef - Portable USB Diagnostics & Terminal Tool
================================================
Version: portable Windows x64 build
Built: 2026-11-08
Target: x86_64-pc-windows-gnu (MinGW-w64)

WHAT THIS IS
------------
Reef is a portable diagnostics and terminal application for network devices
(SSH, Telnet, serial). It runs entirely from a USB stick or any removable
drive and does not require installation, administrator rights, or developer
tools on the host machine.

All user data (logs, config, session recordings) is stored next to the
Reef.exe executable, under Logs/, Config/, and Data/. The drive letter may
change between machines; Reef always resolves its storage relative to the
location of its own executable, so nothing is tied to a fixed drive letter or
to %APPDATA% / the user home directory.

PORTABLE MODE
-------------
Reef runs in portable mode when either:
  * a file named `portable.marker` exists next to Reef.exe, OR
  * the environment variable REEF_PORTABLE_ROOT is set to a directory.

When neither is present, Reef still stores data next to its executable
(derived from current_exe().parent()) and never falls back to %APPDATA% or
the user home directory.

CONTENTS
--------
Reef.exe             - main application executable (native wgpui UI)
portable.marker      - empty marker file that opts into portable mode
Logs/                - runtime logs (Diagnostics, Sessions, Exports, TestNotes)
Config/              - application configuration (reef-config.json)
Data/                - application data directory
README.txt           - this file
TEST-CHECKLIST.txt   - classroom testing checklist
TEST-REPORT.txt      - build and test results
DIAGNOSTICS-README.txt - diagnostics system overview

SYSTEM REQUIREMENTS (Windows host)
----------------------------------
- Windows 10 or Windows 11, x64
- No WebView2 Runtime required. The UI is rendered natively by wgpui and
  imports only standard Windows system DLLs.
- No administrator rights, no installation, no Rust/Cargo/Node toolchain.

GETTING STARTED
---------------
1. Copy the entire `dist/Reef-USB/` folder onto a USB stick.
2. Plug the stick into a Windows PC. The drive letter can be anything.
3. Run `Reef.exe`.
4. Use the Diagnostics panel to view status, logs, and environment, and to
   export a diagnostics ZIP for support.

NO STARTUP-CONFIG WRITES (SAFETY GUARD)
---------------------------------------
Reed is configured to NEVER write, save, or copy the running-config to the
startup-config. The following are blocked across all Reef-managed command
paths and logged without secrets:
  write memory
  write
  copy running-config startup-config
  copy run start
  erase/delete of startup-config or NVRAM

Normal running-config edits (interface, route, etc.) and `show` commands
continue to work. There is no setting to re-enable startup-config writes.

Note: this guard covers Reef-managed command paths only. Direct, arbitrary
terminal access (raw keystrokes typed by the user) cannot be fully guarded;
treat the terminal as an unrestricted session.

DIAGNOSTICS
-----------
Reef records structured diagnostics to Logs/Diagnostics/ (JSONL +
human-readable) with run IDs, millisecond timestamps, and TRACE/DEBUG/INFO/
WARN/ERROR levels. Session recordings are written to Logs/Sessions/<date>/.
See DIAGNOSTICS-README.txt for details.

Logs may be exported to a ZIP from the Diagnostics panel. Exports are written
to Logs/Exports/.

SECRETS
-------
Reef never logs passwords, private keys, tokens, or community strings. Values
matching known secret keywords are redacted in logs. SSH host-key verification
is always enabled and cannot be disabled.

SUPPORT
-------
Export a diagnostics ZIP (Diagnostics panel -> Export) and share it with
support. Do not include the ZIP in public forums without reviewing it first
for sensitive device information.
