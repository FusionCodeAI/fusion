# Development

## Build & Test

```bash
# Build
cargo build --release

# Run tests
cargo test

# Lint
cargo fmt --all --check
cargo clippy --all --all-targets

# Benchmarks
cargo bench

# WASM feature build
cargo build --features wasm --target wasm32-unknown-unknown

# Shell completion generation
fusion --generate-completion bash
```

### TypeScript SDK Development

```bash
cd sdk
npm install
npx tsc --noEmit    # type-check
npm run build       # compile TypeScript
```

## CI/CD & Release

- **CI** (`.github/workflows/ci.yml`): Rustfmt & Clippy lint jobs, test suite across stable Rust, WASM feature build validation, and shell-script verification (`scripts/verify.sh`) with superseded-run cancellation.
- **Release** (`.github/workflows/release.yml`): Version validation, cross-compiled release binaries for four production targets — `aarch64-apple-darwin` (macOS Apple Silicon), `aarch64-unknown-linux-musl` (Linux ARM64), `aarch64-linux-android` (Termux / Android NDK), `x86_64-pc-windows-msvc` (Windows x64) — plus packaging and GitHub release asset upload.
- **Homebrew**: Live formula in `Formula/fusion.rb` with shell-completion generation and version smoke test.
- **Packaging**: `scripts/package.sh` and `scripts/verify.sh` for release artifact packaging and post-build verification.

## Termux & Mobile Support

Fusion treats Android/Termux as a first-class tier:
- **Pure-Rust TLS**: Employs `rustls` with native root certificates, completely eliminating Android's notorious OpenSSL build and dynamic linking failures.
- **Low-Memory Footprint**: Strict streaming buffers and zero-copy JSON parsers ensure Fusion operates comfortably on devices with limited RAM.
- **Touch & Terminal Friendliness**: The inline terminal interface automatically respects narrow viewport widths, preventing layout wrapping and garbled escape sequences.
- **Mobile Preset**: `/preset termux-mobile` applies a curated low-memory configuration.
- **Precompiled Bootstrap**: `scripts/termux-bootstrap.sh` installs a standalone ARM64 binary without a Rust toolchain.

## Contributing

Contributions are welcome. Please follow this guide to keep the project fast, pure-Rust, and portable:

### Ground Rules

1. **Zero `unwrap()` in production paths.** Use `anyhow` for application-level errors and `thiserror` for library-level typed errors. Panics are reserved for tests and genuinely impossible states.
2. **No new C/C++ dependencies.** Every dependency must compile with pure-Rust TLS (`rustls`) on macOS, Linux (glibc & musl), Windows, Android/Termux, and `wasm32-unknown-unknown`. New system-library requirements are rejected.
3. **Cross-platform safety.** Never leak OS-specific paths, shell constructs, or signals into shared code. Use the existing platform abstraction modules (`src/ui/termux.rs`, `src/tools/system.rs`).
4. **Respect performance budgets.** Sub-15ms startup, <15MB binary, <25MB RSS. Avoid eager allocations, blocking calls in async contexts, and unnecessary abstraction layers.
5. **Error handling & cancellation safety.** Propagate errors instead of suppressing them; ensure async tasks handle cancellation and signal interrupts correctly.

### Workflow

```bash
# 1. Fork and create a feature branch
git checkout -b feat/my-feature

# 2. Make changes, then verify locally
cargo fmt --all
cargo clippy --all --all-targets
cargo test

# 3. Open a pull request against main
```

- Branch naming: `feat/**` for features, `fix/**` for fixes (CI runs on both).
- PRs must pass Rustfmt, Clippy, and the full test suite before review.
- For SDK changes, also run `npx tsc --noEmit` inside `sdk/`.
- Update `README.md` and `sdk/README.md` when adding user-visible features.

### Reporting Issues

Open a GitHub issue with:
- Platform (macOS / Linux / Windows / Termux / WASM) and terminal.
- Provider and model in use.
- Minimal reproduction steps and, for crashes, `RUST_BACKTRACE=1` output.

## Roadmap

### Phase 1: Core Engine & Minimal UI (Completed)

- [x] **Pure-Rust Single-Crate Engine**: Sub-15ms startup, zero C/C++ dependencies, zero-copy JSON streaming parser.
- [x] **fx-Style Minimal Terminal UI**:
  - Clean startup screen clearing and cursor positioning.
  - Vertical rail prompt symbol (`┃`) with compact 2-space indentation.
  - Real-time thinking and streaming status with animated Braille spinner (`⠋⠙⠹...`), duration clock, and input/output token counters.
  - 1-blank-line padding above and below turn summaries (`  {time} (↑{in} ↓{out})`).
- [x] **Interactive Skills Picker Panel**:
  - Opened via `/skills` or `/skill` + Enter.
  - Source filter tabs: `[All]`, `[Fusion]`, `[Claude]`, `[Global]`, `[Other]`.
  - Dynamic discovery from project-local (`.fusion/skills/`, `.claude/skills/`) and global (`~/.fusion/skills/`, `~/.claude/skills/`) paths.
  - Full skill name display without truncation, type-to-filter search, and keyboard navigation (`↑↓` navigate, `Tab` cycle source, `Enter` select, `Esc` dismiss).
  - Persistent active-skill chip on the input line (`┃ <skill> · <source>`) allowing continuous message typing.
- [x] **TypeScript & WebAssembly SDK (`@fusioncode/sdk`)**:
  - Official package published to npm at `2.0.0-alpha.1`.
  - Dual transports: browser in-memory WASM (`WasmTransport`) and stdio JSON-RPC (`StdioTransport`).
  - Sandboxed in-memory Virtual File System (VFS) with session checkpoints.
  - Turnkey `xterm.js` adapter with themes, spinners, and slash commands.
- [x] **Universal Release & Distribution**:
  - Automated GitHub Releases for ARM64 and Windows with SHA-256 checksums.
  - One-line installer (`curl -fsSL https://fusioncode.app/install | bash`) resolving latest v2 releases.
  - Termux native bootstrap (`scripts/termux-bootstrap.sh`).
  - Homebrew formula (`Formula/fusion.rb`).

### Phase 2: Extensibility & Protocols (In Progress)

- [x] **MCP Client & Tool Bridge**: Native Model Context Protocol client (`src/tools/mcp.rs`, `src/tools/mcp_bridge.rs`) for external tool invocation.
- [x] **Agent Client Protocol (ACP)**: Full JSON-RPC 2.0 ACP server for Zed, Neovim, and JetBrains editor integration (`src/acp/`).
- [ ] **TUI MCP Discovery**: Auto-discovery and dynamic connection to local stdio and remote SSE Model Context Protocol servers directly from slash commands.
- [ ] **WASM OPFS Persistence**: WebAssembly playground Virtual File System persistence backed by Origin Private File System (OPFS).
- [ ] **Distributed Multi-Process Mesh**: Peer-to-peer session and task sharing across multiple local Fusion processes.

### Phase 3: Advanced Autonomous Capabilities (Planned)

- [ ] **Vector-Guided Context Compaction**: Semantic pruning and progressive distillation of long conversation histories.
- [ ] **Autonomous TDD Refactoring Loops**: Automated test-driven fix/verify cycles with compiler feedback loops.
- [ ] **Embedded Local Model Runtime**: Zero-dependency local model execution using pure-Rust inference kernels (Candle).
- [ ] **Voice-Driven Interactive Turns**: Streaming voice input and speech synthesis via Web Audio and lightweight local engines.
