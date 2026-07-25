# Fusion

**Fusion** is a terminal-first AI coding agent — **made by Fusion AI**.

| Desktop | Mobile (Termux) |
|:---:|:---:|
| ![Fusion TUI](docs/screenshot.png) | ![Fusion on Termux](docs/screenshot_mobile.png) |


## Install

Install `fusion` globally:

```bash
# npm (Recommended — macOS, Windows, Linux)
npm i -g @fusioncode/cli

# Homebrew (macOS & Linux)
brew tap theaungmyatmoe/fusion https://github.com/theaungmyatmoe/fusion
brew install fusion

# Standalone Script (macOS, Linux, Termux)
curl -fsSL https://fusioncode.app/install | bash

# JSR / Deno
deno install -g -n fusion jsr:@fusioncode/cli
```


## Usage

```bash
fusion login                    # sign in via browser OAuth
fusion                          # interactive TUI
fusion -p "fix the bug"         # single-turn prompt
fusion --always-approve         # auto-approve tool execution
```

### Sign In Flow

```bash
$ fusion login

  Initializing Fusion login session…

  Opening browser to sign in to Fusion…
  If browser does not open automatically, visit:
    https://fusioncode.app/cli-auth?token=...

  Waiting for authorization…

  ✓ Logged in as user@example.com
    API key saved to: ~/.fusion/fusion.toml

  You can now run `fusion` to start the AI agent.
```

> For configuration, build instructions, architecture details, and the full CLI reference see **[docs/DETAILS.md](docs/DETAILS.md)**.


## License

MIT OR Apache-2.0
