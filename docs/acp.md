# Agent Client Protocol (ACP) Support

Fusion features a built-in JSON-RPC 2.0 stdio server implementing the standard **Agent Client Protocol (ACP)**. This allows modern editors and IDEs (such as Zed, Neovim, JetBrains, and VS Code) to use Fusion directly as their native AI assistant engine:

```bash
# Start Fusion in ACP server mode over standard I/O
fusion --acp
```

The ACP engine provides granular session update events, token-by-token streaming, tool status tracking, advisor feedback lifecycles, and bidirectional notification bridging. It runs over any reader/writer pair — stdio, WebSocket, or in-process streams for testing.

## Example: Configuring in Zed Editor

Add Fusion as a custom ACP agent in your `~/.config/zed/settings.json`:

```json
{
  "assistant": {
    "version": "2",
    "provider": {
      "name": "custom",
      "command": "fusion",
      "args": ["--acp"]
    }
  }
}
```

## Example: Configuring in Neovim

```lua
require("fusion-acp").setup({
  cmd = { "fusion", "--acp" },
  autostart = true,
})
```
