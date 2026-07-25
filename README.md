# Fusion

**Fusion** is a terminal-first AI coding agent — **made by Fusion AI**.

| Desktop | Mobile (Termux) |
|:---:|:---:|
| ![Fusion TUI](docs/screenshot.png) | ![Fusion on Termux](docs/screenshot_mobile.png) |


## Install

```bash
curl -fsSL https://fusioncode.app/install | bash
```


## Usage

```bash
fusion login                    # sign in
fusion                          # interactive TUI
fusion --minimal                # scrollback-friendly mode
fusion -p "fix the bug"         # headless one-shot
fusion --always-approve -p "…"  # auto-approve all tools
```

> For configuration, build instructions, architecture details, and the full CLI reference see **[docs/DETAILS.md](docs/DETAILS.md)**.


## License

MIT OR Apache-2.0
