#!/bin/sh
# Fusion — one-line installer
# curl -fsSL https://fusioncode.app/install | bash
#
# Downloads the latest GitHub release binary for this platform.
# Release assets (see .github/workflows/release.yml):
#   fusion-<tag>-<triple>.tar.gz
# where <triple> is one of:
#   x86_64-unknown-linux-gnu       ← Linux x86_64
#   aarch64-linux-android          ← Termux / Android ARM64 (native NDK)
#   aarch64-apple-darwin           ← macOS Apple Silicon
#   x86_64-apple-darwin            ← macOS Intel
#   x86_64-pc-windows-msvc         ← Windows x64 (Git Bash / MSYS2)
#
# Older releases used aarch64-unknown-linux-musl for Termux; we fall back to
# that as a secondary candidate if the primary asset is not found.
set -eu

REPO="theaungmyatmoe/fusion"
BINARY="fusion"

# Colors (no-op if stdout is not a TTY)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    DIM='\033[0;90m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED='' GREEN='' DIM='' BOLD='' NC=''
fi

info() { printf '%b\n' "${DIM}$1${NC}"; }
ok()   { printf '%b\n' "${GREEN}${BOLD}$1${NC}"; }
err()  { printf '%b\n' "${RED}$1${NC}" >&2; exit 1; }

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || err "Required command not found: $1"
}

need_cmd uname
need_cmd curl
need_cmd tar
need_cmd mktemp
need_cmd grep
need_cmd head

# Detect platform
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Alpine / iSH detection
if [ -f "/etc/alpine-release" ]; then
    PLATFORM="alpine"
    INSTALL_DIR="/usr/local/bin"

    # Auto-install dependencies if missing on Alpine
    if ! command -v git >/dev/null 2>&1 \
        || ! command -v rg >/dev/null 2>&1 \
        || [ ! -f /etc/ssl/certs/ca-certificates.crt ]; then
        info "Installing missing dependencies (git, ripgrep, ca-certificates)..."
        if command -v apk >/dev/null 2>&1; then
            apk update
            apk add git ripgrep ca-certificates
        else
            err "apk package manager not found. Please install git, ripgrep, and ca-certificates manually."
        fi
    fi
# Termux detection (PREFIX is set to …/com.termux/…/usr)
elif [ -n "${PREFIX:-}" ] && printf '%s' "$PREFIX" | grep -q "com.termux"; then
    PLATFORM="termux"
    INSTALL_DIR="$PREFIX/bin"

    # Termux needs a writable temp dir; system /tmp is often unusable.
    export TMPDIR="${PREFIX}/tmp"
    mkdir -p "$TMPDIR"

    # Auto-install basic deps if missing
    if ! command -v git >/dev/null 2>&1 \
        || ! command -v rg >/dev/null 2>&1 \
        || ! command -v python3 >/dev/null 2>&1 \
        || ! command -v curl >/dev/null 2>&1 \
        || [ ! -f "$PREFIX/etc/tls/cert.pem" ]; then
        info "Installing missing dependencies (git, ripgrep, python, curl, ca-certificates)..."
        pkg update -y || true
        pkg install -y git ripgrep python curl ca-certificates
    fi

    # Ensure pip and duckduckgo-search Python packages are installed
    if command -v python3 >/dev/null 2>&1; then
        if ! command -v pip >/dev/null 2>&1 && ! command -v pip3 >/dev/null 2>&1; then
            info "Ensuring pip is installed..."
            python3 -m ensurepip --upgrade || true
        fi
        if ! python3 -c "import duckduckgo_search" >/dev/null 2>&1; then
            info "Installing duckduckgo-search Python package (for captcha-free web search)..."
            python3 -m pip install --upgrade pip || true
            python3 -m pip install --upgrade duckduckgo-search || true
        fi
    fi
elif [ "$OS" = "darwin" ]; then
    PLATFORM="macos"
    INSTALL_DIR="${HOME}/.local/bin"
elif [ "$OS" = "linux" ]; then
    PLATFORM="linux"
    INSTALL_DIR="${HOME}/.local/bin"
elif printf '%s' "$OS" | grep -q "mingw\|msys\|cygwin"; then
    # Git Bash / MSYS2 / Cygwin on Windows
    PLATFORM="windows"
    INSTALL_DIR="${HOME}/bin"
    BINARY="fusion.exe"
else
    err "Unsupported OS: $OS"
fi

# Map architecture → release triple component
case "$ARCH" in
    aarch64|arm64) TARGET_ARCH="aarch64" ;;
    x86_64|amd64)  TARGET_ARCH="x86_64" ;;
    *)             err "Unsupported architecture: $ARCH" ;;
esac

# Target triple for GitHub release assets.
# Termux uses the native aarch64-linux-android build (compiled via Android NDK).
# Alpine / plain Linux use the static musl build.
case "$PLATFORM" in
    termux)
        TARGET="${TARGET_ARCH}-linux-android"
        ;;
    alpine|linux)
        TARGET="${TARGET_ARCH}-unknown-linux-gnu"
        ;;
    macos)
        TARGET="${TARGET_ARCH}-apple-darwin"
        ;;
    windows)
        TARGET="${TARGET_ARCH}-pc-windows-msvc"
        ;;
    *)
        err "Unsupported platform: $PLATFORM"
        ;;
esac

info "Installing Fusion..."
info "  platform: $PLATFORM ($TARGET)"
info "  install:  $INSTALL_DIR/$BINARY"
echo ""

RELEASE_URL="https://api.github.com/repos/${REPO}/releases/latest"
RELEASE_JSON=$(curl -fsSL "$RELEASE_URL") || err "Failed to fetch latest release metadata from GitHub"

# Pick the best matching browser_download_url from the release JSON.
# Prefer: versioned tar.gz → unversioned tar.gz → bare binary.
# Patterns (examples for aarch64 Termux):
#   fusion-v0.2.0-aarch64-unknown-linux-musl.tar.gz   (current release.yml)
#   fusion-aarch64-unknown-linux-musl.tar.gz          (older unversioned)
#   fusion-aarch64-linux-android.tar.gz               (legacy Termux name)
# Emit download URLs whose asset name ends with the given suffix (not .sha256).
# Matches both:
#   fusion-<triple>.tar.gz
#   fusion-<tag>-<triple>.tar.gz
pick_download_url() {
    suffix="$1"
    printf '%s\n' "$RELEASE_JSON" \
        | grep -o "https://[^\"]*${BINARY}-[^\"]*${suffix}" \
        | grep -v '\.sha256$' \
        | grep "/${BINARY}-[^/]*${suffix}\$" || true
}

# Collect candidate URLs in preference order
CANDIDATES=""
# 1) tar.gz for primary triple (versioned + unversioned)
for url in $(pick_download_url "${TARGET}.tar.gz"); do
    CANDIDATES="${CANDIDATES}${url}
"
done
# 2) bare binary for primary triple (no .tar.gz / checksum)
for url in $(pick_download_url "${TARGET}"); do
    case "$url" in
        *.tar.gz|*.sha256) ;;
        *) CANDIDATES="${CANDIDATES}${url}
" ;;
    esac
done

# 3) Legacy Termux fallback: older releases used -aarch64-unknown-linux-musl
if [ "$PLATFORM" = "termux" ] && [ "$TARGET_ARCH" = "aarch64" ]; then
    for url in $(pick_download_url "aarch64-unknown-linux-musl.tar.gz"); do
        CANDIDATES="${CANDIDATES}${url}
"
    done
    for url in $(pick_download_url "aarch64-unknown-linux-musl"); do
        case "$url" in
            *.tar.gz|*.sha256) ;;
            *) CANDIDATES="${CANDIDATES}${url}
" ;;
        esac
    done
fi

DOWNLOAD_URL=$(printf '%s' "$CANDIDATES" | grep -v '^$' | head -1)

if [ -z "$DOWNLOAD_URL" ]; then
    err "No release asset found for $TARGET.
  Expected something like:
    fusion-<tag>-${TARGET}.tar.gz
  Check: https://github.com/${REPO}/releases"
fi

# Create install dir
mkdir -p "$INSTALL_DIR"

# Download into a private temp dir (do not clobber TMPDIR env used by Termux)
WORK=$(mktemp -d)
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

info "Downloading $DOWNLOAD_URL..."

case "$DOWNLOAD_URL" in
    *.tar.gz)
        curl -fsSL "$DOWNLOAD_URL" -o "$WORK/archive.tar.gz"
        # Verify SHA256 checksum if the .sha256 asset exists
        SHA256_URL="${DOWNLOAD_URL}.sha256"
        if curl -fsSL "$SHA256_URL" -o "$WORK/archive.tar.gz.sha256" 2>/dev/null; then
            EXPECTED=$(cat "$WORK/archive.tar.gz.sha256" | awk '{print $1}')
            if command -v sha256sum >/dev/null 2>&1; then
                ACTUAL=$(sha256sum "$WORK/archive.tar.gz" | awk '{print $1}')
            elif command -v shasum >/dev/null 2>&1; then
                ACTUAL=$(shasum -a 256 "$WORK/archive.tar.gz" | awk '{print $1}')
            else
                info "Warning: cannot verify checksum (sha256sum/shasum not found)"
                ACTUAL=""
            fi
            if [ -n "$ACTUAL" ] && [ "$ACTUAL" != "$EXPECTED" ]; then
                err "SHA256 mismatch!\n  expected: $EXPECTED\n  got:      $ACTUAL"
            fi
            if [ -n "$ACTUAL" ]; then
                info "Checksum verified"
            fi
        fi
        tar xzf "$WORK/archive.tar.gz" -C "$WORK"
        if [ -f "$WORK/$BINARY" ]; then
            FOUND="$WORK/$BINARY"
        elif [ -f "$WORK/bin/$BINARY" ]; then
            FOUND="$WORK/bin/$BINARY"
        else
            err "Archive downloaded but did not contain a '$BINARY' binary"
        fi
        mv "$FOUND" "$INSTALL_DIR/$BINARY"
        ;;
    *)
        curl -fsSL -o "$INSTALL_DIR/$BINARY" "$DOWNLOAD_URL"
        ;;
esac

chmod +x "$INSTALL_DIR/$BINARY"

# Smoke-check: binary exists and is executable
if [ ! -x "$INSTALL_DIR/$BINARY" ]; then
    err "Install failed: $INSTALL_DIR/$BINARY is not executable"
fi

echo ""
ok "Fusion installed to $INSTALL_DIR/$BINARY"

# Show version if the binary can run
if "$INSTALL_DIR/$BINARY" --version >/dev/null 2>&1; then
    info "Version: $("$INSTALL_DIR/$BINARY" --version 2>/dev/null | head -1)"
fi

# PATH hint
case ":$PATH:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        echo ""
        info "Add to your PATH:"
        echo ""
        echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
        echo ""
        info "Add that line to ~/.bashrc or ~/.zshrc to make it permanent."
        ;;
esac

echo ""
ok "To get started, sign in and run Fusion:"
echo ""
echo "  fusion login"
echo "  fusion"
echo ""
