# WebAssembly & TypeScript SDK

## Browser Playground

Compile and run the entire Fusion agent engine inside any WebAssembly-compatible browser:

- **Tabbed Model Picker**: Instant visual switching across providers and models.
- **Virtual Memory FS**: Sandboxed in-browser code exploration and editing with `read`, `write`, `edit`, `grep`, `glob`, and simulated `bash` tools.
- **xterm.js Terminal**: WebSocket ACP bridge to a full in-browser terminal experience.

## TypeScript SDK (`@fusion/sdk`)

The [official SDK](../sdk/README.md) wraps the pure-Rust WASM core for Browser and Node.js (>= 18):

- **In-Memory Virtual File System (VFS)** with session checkpoint serialization.
- **Streaming Token & Event Pipeline**: real-time deltas for thinking traces, text, tool executions, advisor critiques, and token analytics.
- **Multi-Agent Advisors**: Architect, Security, and Performance critiques before and after tool calls.
- **Xterm.js Terminal Adapter** with ANSI coloring, auto-wrapping, history navigation, and slash commands.
- **Universal Model Support**: OpenRouter, Anthropic, OpenAI, DeepSeek, and local Ollama.
