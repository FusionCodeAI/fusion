<div align="center">

<img src="assets/logo.svg" width="80" height="80" alt="Fusion Logo">

# Fusion

**Fast, ultra-lightweight, cross-platform AI coding assistant written in 100% pure Rust.**  
*Runs natively on macOS, Linux, Windows, Android/Termux, and in the Browser via WebAssembly.*

[![Paper](https://img.shields.io/badge/DOI-10.5281%2Fzenodo.22289167-blue.svg)](https://doi.org/10.5281/zenodo.22289167)
[![License: MIT](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)
[![CI](https://github.com/FusionCodeAI/fusion/actions/workflows/ci.yml/badge.svg)](https://github.com/FusionCodeAI/fusion/actions/workflows/ci.yml)
</div>

## Quick Install

Choose your preferred installation method:

### 1. One-Line Installer (macOS & Linux)

```bash
curl -fsSL https://fusioncode.app/install | bash
```

### 2. Android / Termux Bootstrap (Mobile Coding)

```bash
pkg update && pkg install -y curl
curl -fsSL https://raw.githubusercontent.com/FusionCodeAI/fusion/main/scripts/termux-bootstrap.sh | bash
```

*Or build directly inside Termux:*

```bash
pkg install -y rust git
cargo install --locked fusion
```

### 3. macOS & Linux via Homebrew

```bash
brew install theaungmyatmoe/tap/fusion
```

### 4. Windows via Scoop or Winget

**Via Scoop:**

```powershell
scoop bucket add fusion https://github.com/theaungmyatmoe/scoop-bucket
scoop install fusion
```

**Via Winget:**

```powershell
winget install theaungmyatmoe.fusion
```

### 5. From Source via Cargo

```bash
cargo install --locked fusion
```

*Or clone and build locally:*

```bash
git clone https://github.com/FusionCodeAI/fusion.git
cd fusion
cargo build --release
# Executable located at ./target/release/fusion
```

### 6. Browser WebAssembly (Zero Install)

Experience Fusion immediately in your browser with zero installation via WebAssembly — featuring the interactive tabbed model selector, client-side virtual filesystem, and direct browser LLM streaming.

## Quickstart

### 1. Authenticate & Configure Providers

**Direct CLI Login (Recommended):**

Authenticate directly with Fusion Code AI from your terminal:

```bash
fusion login
```

This initiates browser authorization and saves your credentials to `~/.config/fusion/config.json`. You can also supply an API key directly on the command line:

```bash
fusion login --key <YOUR_API_KEY>
```

*(Inside an active interactive REPL, `/login` is also available.)*

**Or via Environment Variables:**

Set any of the supported provider environment variables:

```bash
export FUSION_API_KEY="..."      # Fusion API (Recommended)
export DEEPSEEK_API_KEY="sk-..."
export ANTHROPIC_API_KEY="sk-ant-..."
export OPENAI_API_KEY="sk-proj-..."
export XAI_API_KEY="xai-..."
export OPENROUTER_API_KEY="sk-or-..."
```

> **Fusion API**: The project's own OpenAI-compatible provider at `http://api.fusioncode.app/v1`. Set `FUSION_API_KEY` or run `fusion login`, then launch with `fusion -p fusion`.

### 2. Launch Interactive Inline REPL

```bash
fusion
```

```text
  Fusion v2.0.0-alpha.1 (Pure-Rust AI Coding Assistant)
  Provider: deepseek  Model: deepseek-chat  Advisors: on
  Type your prompt, /help for commands, or Ctrl+D / /exit to quit.

> Explain the concurrency architecture in src/agent/loop_runner.rs
```

> **Interactive Highlights**:
> - **Live `@file` Autocomplete**: Type `@` (e.g. `@main`, `@auth`) to trigger instant, fuzzy file path autocompletion directly in the prompt bar.
> - **Working Tree Rewind**: Use `/rewind` or `/undo` to inspect checkpoints and roll back file modifications and conversation turns with a single command.

### 3. Non-Interactive / Scripting Mode

```bash
fusion "Find all public functions in src/tools and output a table"
fusion -p deepseek -m deepseek-reasoner "Analyze deadlock risks in async locks"
git diff | fusion "Generate a conventional git commit message for these changes"
```

### 4. TypeScript SDK (Browser / Node.js)

```bash
npm install @fusioncode/sdk
```

```typescript
import { FusionAgent, VirtualFs } from "@fusioncode/sdk";

const agent = new FusionAgent({
  provider: "openrouter",
  apiKey: process.env.OPENROUTER_API_KEY,
  fs: new VirtualFs(),
  advisors: true,
});

for await (const event of agent.run("Refactor the auth module for caching")) {
  if (event.type === "text") process.stdout.write(event.delta);
}
```

See the [SDK README](sdk/README.md) for the full API.

### 5. IDE Integration (ACP)

```bash
fusion --acp
```

Point Zed, Neovim, or JetBrains at Fusion over the Agent Client Protocol — see [docs/acp.md](docs/acp.md).

## Native Language Server Protocol (LSP)

Fusion features a native, zero-dependency pure-Rust Language Server Protocol (LSP) client operating over asynchronous stdio JSON-RPC (`fusion::tools::lsp`). Rather than relying solely on naive text matching or heuristic regexes, the built-in `lsp` tool equips agents with compiler-grade semantic code intelligence across dozens of programming languages.

### Capabilities & Supported Actions

- **Zero-Config Auto-Discovery**: Ships with embedded configurations for standard language servers (`rust-analyzer`, `vtsls`, `typescript-language-server`, `pyright`, `basedpyright`, `pylsp`, `gopls`, `clangd`, `zls`, `biome`, `deno`, `sourcekit-lsp`, `metals`, `hls`, `ocamllsp`, and more). Fusion automatically detects project root markers (e.g. `Cargo.toml`, `package.json`, `tsconfig.json`, `go.mod`, `pyproject.toml`) and spawns the appropriate server on demand.
- **Deep Semantic Intelligence**:
  - **`definition`**: Jump directly to exact symbol definitions across local crates, packages, and dependency sources.
  - **`references`**: Find all incoming usages, call sites, and symbol references across the entire workspace.
  - **`type_definition`**: Locate underlying type declarations for variables, parameters, and expressions.
  - **`diagnostics`**: Query live compiler and linter diagnostics, errors, and warnings for a specific file or the entire workspace (`file: "*"`).
  - **`symbols`**: Extract hierarchical outlines of functions, structs, enums, traits, methods, and constants.
- **Resilient Fallback**: When an external language server binary is unavailable in the environment, Fusion transparently falls back to local Tree-sitter AST queries and symbol indexing without interrupting the agent's turn.

## Subagent Delegation & Live Scrollback Streaming

For complex, multi-stage engineering tasks, Fusion features an autonomous multi-agent mesh with parallel delegation and real-time visual streaming designed to keep your terminal history clean, informative, and fully interactive.

### Autonomous Subagent Delegation

- **Parallel Worker Mesh**: The primary coordinator dynamically fans out independent tasks using `spawn_subagent` and `spawn_subagents_batch`, scaling up to 16–32 concurrent workers.
- **Specialized Roles**:
  - **`Scout`**: Fast, read-only exploration specialist that uses `grep`, `glob`, `read`, and `lsp` to index files and map architecture without mutation risk.
  - **`Coder`**: Surgical implementation specialist for applying targeted code edits, refactors, and file creation.
  - **`Tester`**: Verification specialist for executing targeted test suites via sandboxed commands, capturing output, and isolating regressions.
  - **`Reviewer`**: In-depth audit specialist for reviewing diffs, security vulnerabilities, and design patterns.
  - **`General` / `Custom`**: Adaptable workers tailored dynamically for user-defined pipelines and prompts.
- **Isolated Workspaces**: Subagents execute with isolated workspace environments (powered by `fusion-iso`), eliminating race conditions and file mutation conflicts during concurrent edits.

### Live Terminal Scrollback Streaming

- **Muted Italic Reasoning Stream**: Model reasoning chunks (`AgentEvent::ThinkingDelta` from DeepSeek-R1, Claude 3.7 Sonnet Thinking, etc.) stream live to stdout in dimmed italic ANSI text (`\x1b[2;3m`). Once reasoning concludes, it is cleanly committed into your terminal's native scrollback history separated by a subtle divider line (`───`), leaving the active prompt ready for action.
- **Real-Time Subagent Execution Trees**: Background subagent lifecycles and tool invocations stream live into your terminal scrollback as hierarchical trees:
  ```text
  ┌─ 🤖 Scout (ArchitectureMapper): Map crate dependencies and interfaces
  │  ⠋ grep
  │  ✓ grep
  │  ⠋ read
  │  ✓ read
  └─ ✓ ArchitectureMapper finished in 3 turns
  ```
- **Non-Intrusive Inline View**: Powered by Ratatui and Crossterm, Fusion's inline rendering engine never captures or clobbers your alternate screen buffer (`insert_before`). All commands, compiler diagnostics, agent thoughts, and execution trees remain permanently preserved, searchable, and copyable in your standard terminal scrollback buffer.

## Frontier Agent Capabilities

Fusion introduces four high-leverage frontier capabilities engineered for interactive agent steering, token-efficient codebase comprehension, and zero-risk refactoring:

### 1. Interactive Architectural Decision-Making (`ask` Tool)

Rather than guessing during ambiguous architectural decisions, choosing between alternative libraries, or performing high-risk operations, autonomous agents invoke the interactive `ask` tool:

- **Structured Inquiries**: Agents present structured questions with contextual options, detailed tradeoff descriptions, multi-select support, and suggested `(Recommended)` chips.
- **Interactive TUI Prompt**: In interactive terminal sessions, choices render as an inline interactive selector, allowing developers to review tradeoffs and make informed decisions in real time.
- **Headless & Non-Interactive Fallback**: When operating in headless scripts, CI pipelines, or non-interactive environments, the tool automatically resolves to the recommended or default choice without stalling.
- **Decision Audit Trail**: The selected option (`Selected option: <label>`) is immediately recorded in the agent's turn context, ensuring subsequent code generation adheres strictly to your design decisions.

### 2. Structural Outline Reader with Line Selectors (`read` Tool)

Large files often exhaust LLM context windows or introduce unwanted noise. Fusion's enhanced file reader provides surgical, token-conserving inspection with inline path selectors:

- **Line Range Slicing (`:start-end`)**: Read targeted slices directly via path syntax (e.g. `src/server.rs:45-90`), returning numbered lines for the specified range.
- **Offset Reading (`:start`)**: Start reading from a specific line number to the end of the file (e.g. `src/lib.rs:120`).
- **Verbatim Raw Output (`:raw`)**: Bypass line-number formatting headers to stream exact file bytes (e.g. `config.toml:raw`).
- **Structural AST Outline (`:defs`)**: Powered by Tree-sitter and `fusion_ast`, elides function and struct bodies (`...`) to return a concise, high-level declaration outline with line numbers.
- **Automatic Elision on Oversized Files**: Files exceeding 500 lines read without an explicit range automatically produce a declaration outline with an elision notice (`Summary: N lines elided; re-issue with line range selector (e.g. :50-100)`), protecting context windows from accidental blowup.

### 3. Live `@file` Fuzzy Autocompletion in Prompt Bar

Referencing files in natural language is effortless with instantaneous fuzzy path completion directly in the inline REPL:

- **Trigger on `@`**: Typing `@` followed by any path fragment (e.g. `@main`, `@auth`, `@handler`) queries the workspace file index in real time.
- **Floating Dropdown UI**: A sleek, non-intrusive popup renders above the prompt bar showing matching file paths with clean file indicators (`📄 `) and highlighted match characters.
- **Seamless Navigation**: Cycle candidates with `Up` / `Down` arrows and press `Tab` or `Enter` to expand the full relative file path into the prompt buffer.
- **Powered by `fusion_walker`**: Employs gitignore-aware multi-threaded directory walking with in-memory caching for zero-latency suggestions even in massive monorepos.

### 4. Working Tree Checkpoint Restoration (`/rewind` & `/undo`)

Experiment fearlessly with complex autonomous edits knowing every turn can be undone with a single command:

- **Automatic Turn Checkpoints**: Fusion's execution engine captures snapshots of working tree diffs and conversation state before each turn.
- **One-Click Restoration**:
  - `/rewind [steps]` or `/undo [steps]`: Instantly reverts workspace files and conversation context by N turns (defaults to 1 turn).
  - Inspect checkpoints: Running `/rewind` or `/undo` displays recent checkpoints with turn numbers, relative timestamps, and modified files before reverting.
- **Verified File Restoration**: Restores modified and newly created files to their exact pre-turn state while displaying an inline summary of restored file diffs.
- **Resilient Refactoring**: Revert misguided agent refactors, failed multi-file migrations, or experimental changes cleanly without manually running complex git reset commands.

## Configuration

Fusion stores its configuration in `~/.config/fusion/config.json` (or `%APPDATA%\fusion\config.json` on Windows). Inspect with `/config`:

```json
{
  "default_provider": "deepseek",
  "default_model": "deepseek-chat",
  "advisors_enabled": true,
  "temperature": 0.0,
  "max_tokens": 8192
}
```

Full reference, presets, and environment variables: **[docs/configuration.md](docs/configuration.md)**

## Documentation

Detailed guides have been moved to `docs/`:

| Guide | Description |
| :--- | :--- |
| [Vision](docs/vision.md) | Philosophy and comparison with heavyweight alternatives |
| [Features](docs/features.md) | Inline UI, @file autocomplete, scrollback streaming, providers, checkpoint restore |
| [Agents](docs/agents.md) | Multi-agent mesh, subagent delegation & advisory committee |
| [ACP](docs/acp.md) | Agent Client Protocol for Zed/Neovim/JetBrains |
| [WASM & SDK](docs/wasm-sdk.md) | Browser playground & TypeScript SDK |
| [Tools](docs/tools.md) | Sandboxed tool registry, interactive ask tool, selectors & native LSP capabilities |
| [Architecture](docs/architecture.md) | System design, Pure Rust manifesto, source layout |
| [Commands](docs/commands.md) | CLI commands (login), /rewind checkpoint restore, and slash command reference |
| [Configuration](docs/configuration.md) | Config file, presets, env vars, keybindings |
| [Development](docs/development.md) | Build, CI/CD, Termux, contributing, roadmap |

## Development

```bash
cargo build --release
cargo test
cargo fmt --all --check
cargo clippy --all --all-targets
```

WASM build: `cargo build --features wasm --target wasm32-unknown-unknown`  
SDK: `cd sdk && npm install && npx tsc --noEmit`

See [docs/development.md](docs/development.md) for CI/CD, Termux support, and contribution guidelines.
## Research Paper & Citation

If you use Fusion in your research or systems, please cite our paper:

```bibtex
@article{moe2026fusion,
  title   = {Ubiquitous Autonomous Coding Agents for Constrained Devices via Bounded Explicit State and Zero-C Native Runtimes},
  author  = {Moe, Aung Myat and {Fusion Code AI}},
  year    = {2026},
  month   = {9},
  doi     = {10.5281/zenodo.22289167},
  url     = {https://doi.org/10.5281/zenodo.22289167}
}
```

## License

Licensed under [MIT](LICENSE) © 2026 Aung Myat Moe and Fusion Code AI.
