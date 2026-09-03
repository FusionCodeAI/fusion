<div align="center">

# Fusion

**Fast, ultra-lightweight, cross-platform AI coding assistant written in 100% pure Rust.**
*Runs natively on macOS, Linux, Windows, Android/Termux, and in the Browser via WebAssembly.*

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[**Quick Install**](#quick-install) • [**Quickstart**](#quickstart) • [**Configuration**](#configuration) • [**Documentation**](#documentation) • [**Development**](#development)

</div>

## Quick Install

Choose your preferred installation method:

### 1. One-Line Installer (macOS & Linux)

```bash
curl -fsSL https://fusion.sh/install.sh | bash
```

### 2. Android / Termux Bootstrap (Mobile Coding)

```bash
pkg update && pkg install -y curl
curl -fsSL https://fusion.sh/install-termux.sh | bash
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
git clone https://github.com/theaungmyatmoe/fusion.git
cd fusion
cargo build --release
# Executable located at ./target/release/fusion
```

### 6. Browser WebAssembly (Zero Install)

Experience Fusion immediately in your browser with zero installation via WebAssembly — featuring the interactive tabbed model selector, client-side virtual filesystem, and direct browser LLM streaming.

## Quickstart

### 1. Configure Provider API Keys

Set any of the supported provider environment variables:

```bash
export FUSION_API_KEY="..."      # Fusion API (Recommended)
export DEEPSEEK_API_KEY="sk-..."
export ANTHROPIC_API_KEY="sk-ant-..."
export OPENAI_API_KEY="sk-proj-..."
export XAI_API_KEY="xai-..."
export OPENROUTER_API_KEY="sk-or-..."
```

> **Fusion API**: The project's own OpenAI-compatible provider at `http://api.fusioncode.app/v1`. Set `FUSION_API_KEY` and run `fusion -p fusion`.

### 2. Launch Interactive Inline REPL

```bash
fusion
```

```text
  Fusion v0.3.0 (Pure-Rust AI Coding Assistant)
  Provider: deepseek  Model: deepseek-chat  Advisors: on
  Type your prompt, /help for commands, or Ctrl+D / /exit to quit.

> Explain the concurrency architecture in src/agent/loop_runner.rs
```

### 3. Non-Interactive / Scripting Mode

```bash
fusion "Find all public functions in src/tools and output a table"
fusion -p deepseek -m deepseek-reasoner "Analyze deadlock risks in async locks"
git diff | fusion "Generate a conventional git commit message for these changes"
```

### 4. TypeScript SDK (Browser / Node.js)

```bash
npm install @fusion/sdk
```

```typescript
import { FusionAgent, VirtualFs } from "@fusion/sdk";

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
| [Features](docs/features.md) | Inline UI, providers, resilience, tokens, sessions, benchmarking |
| [Agents](docs/agents.md) | Multi-agent mesh & advisory committee |
| [ACP](docs/acp.md) | Agent Client Protocol for Zed/Neovim/JetBrains |
| [WASM & SDK](docs/wasm-sdk.md) | Browser playground & TypeScript SDK |
| [Tools](docs/tools.md) | Sandboxed tool registry |
| [Architecture](docs/architecture.md) | System design, Pure Rust manifesto, source layout |
| [Commands](docs/commands.md) | Slash command reference |
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

## License

Licensed under [MIT](LICENSE).
