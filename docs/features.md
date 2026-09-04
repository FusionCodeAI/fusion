# Core Features

## 1. Minimalist Inline UI & Streaming Renderer

- **Lightweight Inline View**: Built on Ratatui and Crossterm without hijacking your entire terminal buffer. Your shell scrollback and commands remain clean and visible.
- **Fluid Streaming Markdown**: Instant syntax highlighting, tables, callouts, and code blocks rendered in real time as tokens arrive.
- **Animated Spinners & Tool Status**: Visual status indicators when files are being read, edited, grepped, or checked by advisors.
- **Multiline Input**: Press `Ctrl+J`, `Shift+Enter`, or terminate a line with `\` to compose multiline queries.
- **Theming & Keymap Customization**: Runtime theme engine and configurable keybindings (`/config`, `keymap`).
- **Progress Trees & Agent Tree View**: Live hierarchical visualization of concurrent subagent and advisor activity.

## 2. Multi-Provider & Dynamic Model Catalog

Seamlessly toggle between top-tier frontier models and local private LLMs. Includes smart shorthand resolution and a dynamically synchronized model catalog that fetches available models from all configured providers concurrently:

| Provider | Transport | API Key | Example |
| **Fusion** | Cloud streaming (native) | `FUSION_API_KEY` | `fusion -p fusion -m fusion-chat` |
| **DeepSeek** | Cloud streaming | `DEEPSEEK_API_KEY` | `fusion -m deepseek-reasoner` |
| **Anthropic** | Cloud streaming | `ANTHROPIC_API_KEY` | `fusion -m claude-3-7-sonnet` |
| **OpenAI** | Cloud streaming | `OPENAI_API_KEY` | `fusion -m gpt-4o` |
| **xAI** | Cloud streaming | `XAI_API_KEY` | `fusion -m grok-2-latest` |
| **OpenRouter** | Unified gateway (200+ models) | `OPENROUTER_API_KEY` | `fusion -p openrouter -m any/model` |

Smart shorthands: `/model v3` (DeepSeek V3), `/model r1` (DeepSeek R1), `/model sonnet` (Claude 3.5 Sonnet), `/model 4o` (GPT-4o), `/model grok` (Grok 2).

## 3. Agent Engine Resilience

- **Recovery Engine**: Automatic error diagnosis and correction attempts for transient failures; resumable sessions via `/recover [status|resume|diff|discard]`.
- **Rate Throttling**: Token-bucket turn rate limits with wait-duration feedback and banner visualization when provider limits are hit.
- **Retry Policies**: Configurable per-provider retry with retryable-status detection and exponential backoff.
- **Automatic Offline Transition**: Detects connectivity loss and switches directly to local Ollama execution.
- **Context Compaction**: Budget-aware history compaction with aggressive/conservative strategies and thinking/tool prune policies; manual `/compact`.
- **Session Pruner**: Preserve recent turns, initial goals, or tool results while aggressively pruning stale context.
- **Undo / Redo & Checkpoints**: Every file mutation snapshots original content and permissions for instant restore; `/rewind [N]` rewinds sessions turn-by-turn.
- **Heartbeat Monitoring**: Phase-transition records and threshold-based liveness metrics for long-running turns.

## 4. Token, Cost & Tracing Subsystem

- **Token Accounting**: Real-time input/output/cache token analytics with per-provider pricing.
- **Cost Breakdown**: Formatted USD costs with cache-savings percentages and budget warnings.
- **Pricing Sync**: Dynamically fetches and caches current model pricing from providers.
- **OpenTelemetry (OTLP) Tracing**: Standard-compatible trace/span IDs, span kinds, and OTLP span conversion; `/trace [path]` exports trace files with secret-redaction audits.
- **Secret Scanning**: Automatic credential and secret redaction before traces and exports leave the process.

## 5. Productivity & Session Management

- **Persistent Sessions**: Save, load, search, and manage conversations across restarts; JSONL export.
- **Bookmarks & Tags**: Named conversation checkpoints (`/bookmark`) and conversation filtering (`/tag`).
- **Fork & Rewind**: Branch sessions at any turn for alternative approaches; turn-level preview diffs.
- **Snippets & Prompt Library**: Reusable code snippets and saved prompt templates with search.
- **Skills Registry**: Loadable, testable, tag-filtered skill modules (`/skills`).
- **Commit Generator**: Conventional-commit message generation from unified git diffs.
- **Export**: Markdown, HTML, and JSONL conversation export with print-ready CSS.
- **Voice & Notifications**: Pure-Rust voice activity detection, speech-to-text input, text-to-speech feedback, and cross-platform desktop notifications (`notify-send`, `osascript`, Windows toasts).

## 6. Benchmarking

- **Interactive Benchmarks**: `/benchmark [provider]` (aliases `/bench`, `/latency`, `/speed`) measures provider latency and throughput with high-precision timing and token-budget protection.
- **Comparison Harness**: Head-to-head provider/model comparison benchmarks under `benches/` with criterion-based Rust micro-benchmarks.
