Reef Diagnostics System
========================
Portable Windows x64 build

OVERVIEW
--------
Reef records structured diagnostics for troubleshooting classroom and lab
sessions. Diagnostics are written to:

  Logs/Diagnostics/   - run diagnostics (JSONL + human-readable, per run)
  Logs/Sessions/<date>/  - terminal session recordings (ANSI-stripped,
                         redacted, append-only, part-rotated, 512 MiB cap)
  Logs/Exports/       - diagnostics export ZIP files
  Logs/TestNotes/     - test notes captured during classroom runs

RUN IDS & TIMESTAMPS
--------------------
Every Reef run gets a unique run ID. Every log event carries a millisecond
timestamp (ISO-8601 UTC). Events are tagged TRACE, DEBUG, INFO, WARN, or
ERROR. Startup and shutdown events bracket each run, and environment
information is captured at startup.

DIAGNOSTICS STREAMS
-------------------
Each run produces two files in Logs/Diagnostics/:
  reef-diagnostics-<date>-<time>-<runid>.log   - human-readable text
  reef-diagnostics-<date>-<time>-<runid>.jsonl - one JSON object per line

Streams rotate when they reach the size cap, producing part-NNN files, with
a hard cap of 512 MiB per stream to bound disk use on USB sticks.

SESSION RECORDING
-----------------
Terminal I/O is captured at the backend. Recording defaults ON in production
builds. Recordings are written under Logs/Sessions/<YYYY-MM-DD>/ as
ANSI-stripped, secret-redacted, append-only part files. Data is preserved
across unexpected session ends (flush on close, part rotation keeps recent
data).

REDACTION
---------
Known secret keywords (password, passwd, secret, key-string, passphrase,
private-key, api-key, apikey, authorization, token, community) are redacted
in both diagnostics logs and session recordings. Reef never logs passwords,
private keys, tokens, or community strings in plain text.

DIAGNOSTICS PANEL (in-app)
--------------------------
From the Diagnostics panel you can:
  - view run status and environment information
  - list and preview log files
  - copy a log file's contents
  - open the Logs folder in Explorer
  - clear the visible terminal (does NOT delete stored logs)
  - export a diagnostics ZIP to Logs/Exports/
  - start/stop session recording and view recording status
  - send Ctrl+C (interrupt) to a connected device

EXPORT ZIP CONTENTS
-------------------
The diagnostics ZIP includes:
  environment.json
  diagnostics-status.json
  README.txt
  all of Logs/ except Logs/Exports (to avoid ZIP nesting)
  Config/reef-config.json

SAFETY GUARD AND DIAGNOSTICS
----------------------------
Attempts to write running-config to startup-config (write memory, write,
copy running-config startup-config, copy run start, erase/delete of
startup-config/NVRAM) are blocked and logged as diagnostics events without
secrets. Normal running-config edits and show commands are unaffected. There
is no setting to re-enable startup-config writes.

LIMITATIONS
-----------
- The safety guard covers Reef-managed command paths only. Arbitrary keystrokes
  typed directly into the terminal cannot be fully guarded.
- Frontend terminal is request/response (no streaming PTY); returned output
  is bounded at 1 MiB per command, while backend recording captures full output.
- No WebView2 Runtime required; the UI is rendered natively by wgpui.
- SSH host-key verification cannot be disabled.
