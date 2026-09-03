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
- **Release** (`.github/workflows/release.yml`): Version validation, cross-compiled binaries for seven targets — `x86_64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `aarch64-linux-android` (Termux), `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc` — plus packaging and artifact upload.
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

- [ ] MCP tool-server discovery and auto-connection.
- [ ] Distributed mesh coordination across multiple Fusion processes.
- [ ] Additional provider adapters (community-contributed).
- [ ] WASM playground VFS persistence via Origin Private File System.
