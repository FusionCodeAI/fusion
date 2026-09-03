# Slash Commands

Interactive command reference (browsable in-app via `/help` and `/palette`):

| Command | Syntax | Description |
| :--- | :--- | :--- |
| `/help` | `/help [command]` | Browse command help and shortcuts (aliases `/h`, `/?`). |
| `/palette` | `/palette [filter]` | Searchable command palette (aliases `/commands`, `/pal`). |
| `/clear` | `/clear` | Reset conversation history (aliases `/cls`, `/c`). |
| `/file` | `/file [query]` | Fuzzy file picker (aliases `/f`, `/find`). |
| `/status` | `/status` | Session tokens, context usage, environment state. |
| `/quit` | `/quit` | Exit Fusion (aliases `/exit`, `/q`). |
| `/bookmark` | `/bookmark [name\|list\|recall\|checkpoint\|restore\|fork\|pin\|del]` | Named conversation checkpoints (aliases `/bm`, `/mark`). |
| `/tag` | `/tag <add\|list\|filter\|remove\|clear\|stats>` | Tag and filter conversations (aliases `/tags`). |
| `/session` | `/session <list\|search\|new\|load\|save\|delete\|info\|clear>` | Persistent session management. |
| `/fork` | `/fork [title] [turn]` | Branch the session at any turn. |
| `/rewind` | `/rewind [N]` | Rewind the session N turns. |
| `/compact` | `/compact` | Trigger context compaction manually. |
| `/export` | `/export [md\|html] [path]` | Export conversation to Markdown, HTML, or JSONL. |
| `/prompt` | `/prompt <list\|save\|load\|show\|delete\|search>` | Saved prompt library. |
| `/snippet` | `/snippet <save\|insert\|recall\|show\|list\|search\|delete\|clear\|export\|import>` | Reusable code snippets. |
| `/recover` | `/recover [status\|resume\|diff\|discard]` | Inspect and resume interrupted work. |
| `/model` | `/model [name]` | Inspect or switch model on the fly. |
| `/provider` | `/provider [name]` | Switch providers. |
| `/advisors` | `/advisors <on\|off\|toggle\|status>` | Manage the advisory committee. |
| `/stats` | `/stats` | Token and cost statistics card. |
| `/benchmark` | `/benchmark [provider] [options]` | Provider latency/throughput benchmark (aliases `/bench`, `/latency`, `/speed`). |
| `/config` | `/config <show\|path\|save\|set>` | Inspect and edit runtime configuration. |
| `/tools` | `/tools` | List registered tools and capabilities. |
| `/trace` | `/trace [path]` | Export an OpenTelemetry (OTLP) trace file. |
| `/preset` | `/preset [coding-fast\|deep-reasoning\|cheap\|offline-ollama\|termux-mobile]` | Apply a curated configuration preset. |
| `/skills` | `/skills <list\|info\|reload\|enable\|disable\|test>` | Manage the skills registry. |
