#!/data/data/com.termux/files/usr/bin/bash
# Fusion — Termux bootstrap (NO npm / Node required)
# Fusion is a single Rust binary. Prefer the prebuilt installer.
set -euo pipefail

echo "==> Fusion for Termux (single binary — no npm)"
echo

# Writable temp (Android /tmp is not usable)
export PREFIX="${PREFIX:-/data/data/com.termux/files/usr}"
export TMPDIR="${TMPDIR:-$PREFIX/tmp}"
export TMP="$TMPDIR"
export TEMP="$TMPDIR"
mkdir -p "$TMPDIR" "$HOME/.fusion/tmp" 2>/dev/null || true

pkg update -y || true
# Runtime helpers only — NOT node/npm
pkg install -y curl git ripgrep ca-certificates termux-api 2>/dev/null || true

INSTALL_DIR="${PREFIX}/bin"
mkdir -p "$INSTALL_DIR"

echo
echo "Recommended: install prebuilt binary (fast, no compile, no npm):"
echo "  curl -fsSL https://fusioncode.app/install | bash"
echo
echo "Or build from source (needs rust; slow on phones):"
echo "  pkg install -y rust"
echo "  git clone https://github.com/theaungmyatmoe/fusion ~/.fusion-src"
echo "  cd ~/.fusion-src && cargo build --release"
echo "  cp target/release/fusion \"\$PREFIX/bin/\""
echo
echo "Run:"
echo "  fusion login             # sign in with browser or API key"
echo "  fusion                   # start interactive agent"
echo
echo "Temp dir: \$TMPDIR=$TMPDIR  (never use /tmp on Termux)"
echo "Config:   ~/.config/fusion/fusion.toml"
echo
echo "Inside Fusion: /providers  →  Cloudflare  →  token then account ID"
echo "Use /help for commands. Fusion does not need npm."
