# Architecture & Design Specification: Background Auto-Updater

- **Date:** 2026-09-05
- **Status:** Approved
- **Component:** `src/agent/updater.rs`, `src/ui/repl.rs`, `src/config.rs`

---

## 1. Overview & Goals

Fusion is a standalone, single-crate, pure-Rust CLI designed for extreme speed and minimal dependencies. Users primarily install and run it via `curl -fsSL https://fusioncode.app/install | bash` to `~/.local/bin/fusion`.

This specification defines an automatic background update mechanism that:
1. Periodically checks for new releases on GitHub without delaying CLI startup or interactive prompt turns.
2. Downloads and verifies official release binaries in the background using pure-Rust HTTP (`reqwest` with `rustls`) and SHA-256 verification.
3. Stages the update in `~/.fusion/updates/pending_fusion`.
4. Shows a subtle, non-intrusive notification in the turn summary when a new version is ready.
5. Atomically replaces the current running binary upon shutdown or on the next launch, so restarts seamlessly launch the latest version.

---

## 2. Platform Target Matrix & Release Asset Mapping

The updater determines the current platform and architecture at runtime and requests the corresponding release archive from `FusionCodeAI/fusion`:

| Platform | Target Triple | Archive Format | Binary Inside |
|:---|:---|:---|:---|
| **macOS Apple Silicon** | `aarch64-apple-darwin` | `.tar.gz` | `fusion` |
| **Linux AArch64** | `aarch64-unknown-linux-musl` | `.tar.gz` | `fusion` |
| **Android / Termux** | `aarch64-linux-android` | `.tar.gz` | `fusion` |
| **Windows x64** | `x86_64-pc-windows-msvc` | `.zip` | `fusion.exe` |

---

## 3. Storage & Directory Layout

All updater-related artifacts are isolated within `~/.fusion/updates/`:
```text
~/.fusion/
  updates/
    state.json           # Last check timestamp, version info, staged status
    pending_fusion       # Verified, executable binary ready to swap (Unix)
    pending_fusion.exe   # Verified executable ready to swap (Windows)
```

### `state.json` Schema
```json
{
  "last_check_ms": 1757053800000,
  "latest_version": "2.0.0-alpha.2",
  "staged_version": "2.0.0-alpha.2",
  "staged_binary_path": "/Users/user/.fusion/updates/pending_fusion",
  "status": "ready_to_apply"
}
```

---

## 4. Lifecycle & Flow Details

### Phase A: Startup Hook (Check for Staged Binary)
1. At process launch in `main.rs`, before starting the REPL, check if `~/.fusion/updates/pending_fusion` exists and `state.json` marks it `ready_to_apply`.
2. If `current_exe()` is writable:
   - On **Unix**: Rename `current_exe()` to `current_exe().old` (or unlink) and atomically move `pending_fusion` into `current_exe()`. Set execute permissions (`0755`). Remove `current_exe().old`.
   - On **Windows**: Rename `fusion.exe` to `fusion.exe.old`, move `pending_fusion.exe` to `fusion.exe`. Attempt removal of existing `.old` files from prior runs.
3. Clean `state.json` status to `"idle"`.

### Phase B: Asynchronous Background Check
1. Spawn a detached background Tokio task after the REPL starts.
2. Check `state.json`. If elapsed time since `last_check_ms` is under 6 hours (configurable via `check_interval_hours`), skip network query.
3. Fetch `https://api.github.com/repos/FusionCodeAI/fusion/releases/latest` (with `User-Agent: fusion/<version>`).
4. Parse tag (e.g. `v2.0.0-alpha.3`). Compare semver with `env!("CARGO_PKG_VERSION")`.
5. If a newer release is found:
   - Identify asset matching the platform's target triple (e.g. `fusion-v2.0.0-alpha.3-aarch64-apple-darwin.tar.gz`).
   - Download the archive to a temp file in `~/.fusion/updates/`.
   - Download the corresponding `.sha256` asset (or verify against `SHA256SUMS.txt`).
   - Compute the SHA-256 of the downloaded archive. If mismatch, abort silently and clean temp files.
   - Extract the `fusion` binary to `~/.fusion/updates/pending_fusion`.
   - Set permissions to `0755` on Unix.
   - Write updated `state.json` with `status = "ready_to_apply"` and `staged_version = "vX.Y.Z"`.
   - Send notification event to the active REPL state so the turn summary can display:
     `Update v2.0.0-alpha.3 ready (applied on next restart)`

### Phase C: Shutdown / Exit Swap
When the user exits via `/quit`, `Ctrl+D`, or `Ctrl+C`:
1. If an update is staged, attempt the atomic swap immediately so subsequent terminal invocations use the new binary without waiting for the startup hook.

---

## 5. Error Handling & Safety Invariants

1. **Zero disruption to active sessions:** Any network failure, DNS error, rate limit, parse error, or permission error in the background task must fail silently without logging panics or interrupting LLM turn streaming.
2. **Checksum integrity:** No binary is ever executed or moved into place without passing SHA-256 checksum verification.
3. **Permission protection:** If the binary location is read-only (e.g. system `/usr/local/bin` without root privileges or read-only volume), Fusion records this and notifies the user with instructions rather than crashing on swap.
4. **Offline mode:** If offline or on metered networks, background tasks yield immediately.

---

## 6. Verification & Test Plan

1. **Unit tests in `updater.rs`:**
   - Version comparison logic (semantic version parsing and prerelease ordering).
   - Target triple to release asset filename resolution across all 4 OS/arch pairs.
   - SHA-256 verification validation (valid vs corrupted bytes).
   - State serialization and deserialization (`state.json`).
2. **Integration tests:**
   - Atomic replacement simulation: create a temporary dummy executable, stage a mock binary, trigger apply, verify the new binary replaced the old one.
