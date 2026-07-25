# Fusion

**Fusion** is a terminal-first AI coding agent — **made by Fusion AI**.

| Desktop | Mobile (Termux) |
|:---:|:---:|
| ![Fusion TUI](docs/screenshot.png) | ![Fusion on Termux](docs/screenshot_mobile.png) |


## Install

### One-Liner (macOS, Linux, Termux)
```bash
curl -fsSL https://fusioncode.app/install | bash
```

### npm / npx (Universal, No OS Warnings)
```bash
npx @fusioncode/cli login
# or install globally
npm i -g @fusioncode/cli
```

### JSR
```bash
npx jsr run @fusioncode/cli login
# or with Deno
deno run jsr:@fusioncode/cli login
```


## Usage

```bash
fusion login                    # sign in via browser OAuth
fusion                          # interactive TUI
fusion --minimal                # scrollback-friendly mode
fusion -p "fix the bug"         # headless one-shot
fusion --always-approve -p "…"  # auto-approve all tools
```

> For configuration, build instructions, architecture details, and the full CLI reference see **[docs/DETAILS.md](docs/DETAILS.md)**.


## License

MIT OR Apache-2.0
