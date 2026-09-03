# Configuration

Fusion stores its configuration in `~/.config/fusion/config.json` (or `%APPDATA%\fusion\config.json` on Windows). You can inspect and modify it directly using `/config`:

```json
{
  "default_provider": "deepseek",
  "default_model": "deepseek-chat",
  "advisors_enabled": true,
  "temperature": 0.0,
  "max_tokens": 8192,
  "system_prompt": null,
  "sessions_dir": "~/.local/share/fusion/sessions",
  "providers": {
    "deepseek": {
      "api_key": null,
      "base_url": "https://api.deepseek.com/v1"
    },
    "anthropic": {
      "api_key": null,
      "base_url": "https://api.anthropic.com/v1"
    },
    "openai": {
      "api_key": null,
      "base_url": "https://api.openai.com/v1"
    },
    "xai": {
      "api_key": null,
      "base_url": "https://api.x.ai/v1"
    },
    "ollama": {
      "api_key": null,
      "base_url": "http://localhost:11434"
    },
    "openrouter": {
      "api_key": null,
      "base_url": "https://openrouter.ai/api/v1"
    }
  }
}
```

## Configuration Reference

| Key | Type | Description |
| :--- | :--- | :--- |
| `default_provider` | `string` | Provider used when `-p` is omitted (`deepseek`, `anthropic`, `openai`, `xai`, `ollama`, `openrouter`). |
| `default_model` | `string` | Model used when `-m` is omitted. |
| `advisors_enabled` | `bool` | Enable the concurrent advisory committee. |
| `temperature` | `number` | Sampling temperature (`0.0` for deterministic output). |
| `max_tokens` | `number` | Maximum tokens per response. |
| `system_prompt` | `string \| null` | Custom system prompt override. |
| `sessions_dir` | `path` | Persistent conversation storage directory. |
| `providers.<name>.api_key` | `string \| null` | Per-provider credential (env variables take precedence). |
| `providers.<name>.base_url` | `url` | Per-provider endpoint override (self-hosted gateways, proxies). |

## Configuration Presets

Curated presets via `/preset`:

| Preset | Target Workflow |
| :--- | :--- |
| `coding-fast` | High-throughput daily coding with low latency. |
| `deep-reasoning` | Complex reasoning models for hard analysis tasks. |
| `cheap` | Cost-optimized model and token budgets. |
| `offline-ollama` | Fully local, private Ollama execution. |
| `termux-mobile` | Memory-conscious mobile configuration for Termux. |

## Environment Variables

| Variable | Description | Default |
| :--- | :--- | :--- |
| `FUSION_CONFIG` | Custom path to configuration JSON file | `~/.config/fusion/config.json` |
| `DEEPSEEK_API_KEY` | DeepSeek API authorization key | — |
| `ANTHROPIC_API_KEY` | Anthropic Claude API authorization key | — |
| `OPENAI_API_KEY` | OpenAI API authorization key | — |
| `XAI_API_KEY` | xAI Grok API authorization key | — |
| `OPENROUTER_API_KEY` | OpenRouter API gateway key | — |
| `OLLAMA_HOST` | Custom Ollama host URL | `http://localhost:11434` |
| `RUST_LOG` | Tracing log level filter (`debug`, `info`, `warn`) | `error` |

## Shortcuts & Keybindings

| Keybinding | Action |
| :--- | :--- |
| `Enter` | Submit prompt to assistant |
| `Ctrl+J` / `Shift+Enter` | Insert newline for multiline composition |
| `\` + `Enter` | Continue prompt on next line |
| `Up` / `Down` | Browse prompt input history |
| `Ctrl+C` | Interrupt active streaming response or subagent run |
| `Ctrl+D` | Exit Fusion when prompt is empty |
| `Ctrl+L` | Clear screen buffer |
