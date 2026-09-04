#!/bin/sh
# ==============================================================================
# Fusion — Universal One-Line Installer
# ==============================================================================
# Usage:
#   curl -fsSL https://fusioncode.app/install | bash
#   or:
#   curl -fsSL https://raw.githubusercontent.com/theaungmyatmoe/fusion/main/scripts/install.sh | sh
#
# Supported Operating Systems:
#   - macOS (Darwin): Apple Silicon (aarch64), Intel (x86_64)
#   - Linux: x86_64, aarch64, armv7l (glibc, musl, Alpine)
#   - Android / Termux: aarch64, armv7l, x86_64
#   - FreeBSD: x86_64, aarch64
#   - Windows (MSYS2 / Git Bash / Cygwin): x86_64, aarch64
#
# Features:
#   1. Automatic OS and CPU architecture detection
#   2. Download latest GitHub release binary with SHA-256 verification
#   3. Fallback to Cargo build from source if prebuilt binary is unavailable
#   4. Safe installation into ~/.local/bin or /usr/local/bin
#   5. Automatic PATH configuration (.bashrc, .zshrc, .config/fish/config.fish)
# ==============================================================================

set -eu

REPO="${FUSION_REPO:-FusionCodeAI/fusion}"
BINARY="fusion"
VERSION="${FUSION_VERSION:-latest}"

# ── Terminal Colors ───────────────────────────────────────────────────────────
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    DIM='\033[0;90m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' BLUE='' BOLD='' DIM='' NC=''
fi

info()    { printf '%b\n' "${BLUE}==>${NC} ${BOLD}$1${NC}"; }
subinfo() { printf '%b\n' "${DIM}   $1${NC}"; }
ok()      { printf '%b\n' "${GREEN}${BOLD}✓ $1${NC}"; }
warn()    { printf '%b\n' "${YELLOW}${BOLD}⚠ $1${NC}"; }
err()     { printf '%b\n' "${RED}${BOLD}✗ $1${NC}" >&2; exit 1; }

need_cmd() {
    command -v "$1" >/dev/null 2>&1 || err "Required command not found: $1"
}

# ── Environment & Command Checks ──────────────────────────────────────────────
need_cmd uname
need_cmd mktemp
need_cmd grep

# Detect HTTP client
if command -v curl >/dev/null 2>&1; then
    HTTP_CLIENT="curl"
elif command -v wget >/dev/null 2>&1; then
    HTTP_CLIENT="wget"
else
    err "Neither 'curl' nor 'wget' found. Please install curl or wget."
fi

# ── 1. OS and Architecture Detection ──────────────────────────────────────────
OS_RAW="$(uname -s 2>/dev/null || echo "Unknown")"
OS="$(echo "$OS_RAW" | tr '[:upper:]' '[:lower:]')"

ARCH_RAW="$(uname -m 2>/dev/null || echo "Unknown")"
ARCH="$(echo "$ARCH_RAW" | tr '[:upper:]' '[:lower:]')"

PLATFORM=""

# Detect OS / Platform
case "$OS" in
    darwin)
        PLATFORM="macos"
        ;;
    linux)
        # Check for Alpine Linux
        if [ -f "/etc/alpine-release" ] || ( [ -f "/etc/os-release" ] && grep -qi "alpine" /etc/os-release 2>/dev/null ); then
            PLATFORM="alpine"
        # Check for Termux on Android
        elif [ -n "${PREFIX:-}" ] && printf '%s' "$PREFIX" | grep -q "com.termux"; then
            PLATFORM="termux"
        elif [ -d "/data/data/com.termux" ]; then
            PLATFORM="termux"
        else
            PLATFORM="linux"
        fi
        ;;
    freebsd)
        PLATFORM="freebsd"
        ;;
    dragonfly|openbsd|netbsd)
        PLATFORM="bsd"
        ;;
    mingw*|msys*|cygwin*|windows*)
        PLATFORM="windows"
        BINARY="fusion.exe"
        ;;
    *)
        err "Unsupported Operating System: $OS_RAW ($OS)"
        ;;
esac

# Normalize Architecture
case "$ARCH" in
    x86_64|amd64|x64)
        TARGET_ARCH="x86_64"
        ;;
    aarch64|arm64|armv8*|arm64v8)
        TARGET_ARCH="aarch64"
        ;;
    armv7*|armhf|armv7l|arm)
        TARGET_ARCH="armv7l"
        ;;
    i386|i686)
        TARGET_ARCH="i686"
        ;;
    *)
        err "Unsupported CPU Architecture: $ARCH_RAW ($ARCH)"
        ;;
esac

# ── Platform-Specific Dependency Bootstrapping ─────────────────────────────────
if [ "$PLATFORM" = "alpine" ]; then
    if ! command -v git >/dev/null 2>&1 \
        || ! command -v rg >/dev/null 2>&1 \
        || [ ! -f /etc/ssl/certs/ca-certificates.crt ]; then
        info "Alpine Linux detected. Installing missing dependencies (git, ripgrep, ca-certificates)..."
        if command -v apk >/dev/null 2>&1; then
            apk update && apk add git ripgrep ca-certificates || true
        fi
    fi
elif [ "$PLATFORM" = "termux" ]; then
    PREFIX="${PREFIX:-/data/data/com.termux/files/usr}"
    export TMPDIR="${PREFIX}/tmp"
    mkdir -p "$TMPDIR" 2>/dev/null || true

    if ! command -v git >/dev/null 2>&1 \
        || ! command -v rg >/dev/null 2>&1 \
        || ! command -v curl >/dev/null 2>&1 \
        || [ ! -f "$PREFIX/etc/tls/cert.pem" ]; then
        info "Termux detected. Installing missing packages (git, ripgrep, curl, ca-certificates)..."
        pkg update -y || true
        pkg install -y git ripgrep curl ca-certificates || true
    fi
fi

# ── Target Triple Mapping ─────────────────────────────────────────────────────
TARGET=""
FALLBACK_TARGETS=""

case "$PLATFORM" in
    macos)
        case "$TARGET_ARCH" in
            aarch64)
                TARGET="aarch64-apple-darwin"
                FALLBACK_TARGETS="x86_64-apple-darwin" # Rosetta 2 compatibility
                ;;
            x86_64)
                TARGET="x86_64-apple-darwin"
                ;;
            *)
                err "Unsupported macOS architecture: $TARGET_ARCH"
                ;;
        esac
        ;;
    linux)
        case "$TARGET_ARCH" in
            x86_64)
                TARGET="x86_64-unknown-linux-gnu"
                FALLBACK_TARGETS="x86_64-unknown-linux-musl"
                ;;
            aarch64)
                TARGET="aarch64-unknown-linux-gnu"
                FALLBACK_TARGETS="aarch64-unknown-linux-musl aarch64-linux-android"
                ;;
            armv7l)
                TARGET="armv7-unknown-linux-gnueabihf"
                FALLBACK_TARGETS="armv7-unknown-linux-musleabihf arm-unknown-linux-gnueabihf"
                ;;
            *)
                TARGET="${TARGET_ARCH}-unknown-linux-gnu"
                FALLBACK_TARGETS="${TARGET_ARCH}-unknown-linux-musl"
                ;;
        esac
        ;;
    alpine)
        case "$TARGET_ARCH" in
            x86_64)
                TARGET="x86_64-unknown-linux-musl"
                FALLBACK_TARGETS="x86_64-unknown-linux-gnu"
                ;;
            aarch64)
                TARGET="aarch64-unknown-linux-musl"
                FALLBACK_TARGETS="aarch64-unknown-linux-gnu"
                ;;
            armv7l)
                TARGET="armv7-unknown-linux-musleabihf"
                ;;
            *)
                TARGET="${TARGET_ARCH}-unknown-linux-musl"
                ;;
        esac
        ;;
    termux)
        case "$TARGET_ARCH" in
            aarch64)
                TARGET="aarch64-linux-android"
                FALLBACK_TARGETS="aarch64-unknown-linux-musl aarch64-unknown-linux-gnu"
                ;;
            armv7l)
                TARGET="armv7-linux-androideabi"
                FALLBACK_TARGETS="arm-linux-androideabi armv7-unknown-linux-musleabihf"
                ;;
            x86_64)
                TARGET="x86_64-linux-android"
                FALLBACK_TARGETS="x86_64-unknown-linux-gnu x86_64-unknown-linux-musl"
                ;;
            *)
                TARGET="${TARGET_ARCH}-linux-android"
                ;;
        esac
        ;;
    freebsd)
        case "$TARGET_ARCH" in
            x86_64)
                TARGET="x86_64-unknown-freebsd"
                ;;
            aarch64)
                TARGET="aarch64-unknown-freebsd"
                ;;
            *)
                TARGET="${TARGET_ARCH}-unknown-freebsd"
                ;;
        esac
        ;;
    bsd)
        TARGET="${TARGET_ARCH}-unknown-freebsd"
        ;;
    windows)
        case "$TARGET_ARCH" in
            x86_64)
                TARGET="x86_64-pc-windows-msvc"
                FALLBACK_TARGETS="x86_64-pc-windows-gnu"
                ;;
            aarch64)
                TARGET="aarch64-pc-windows-msvc"
                ;;
            *)
                TARGET="${TARGET_ARCH}-pc-windows-msvc"
                ;;
        esac
        ;;
esac

# ── 3. Installation Directory Selection ───────────────────────────────────────
if [ -n "${FUSION_INSTALL_DIR:-}" ]; then
    INSTALL_DIR="$FUSION_INSTALL_DIR"
elif [ -n "${INSTALL_DIR:-}" ]; then
    INSTALL_DIR="$INSTALL_DIR"
elif [ "$PLATFORM" = "termux" ]; then
    INSTALL_DIR="${PREFIX}/bin"
elif [ "$PLATFORM" = "windows" ]; then
    INSTALL_DIR="${HOME}/.local/bin"
elif [ "$(id -u 2>/dev/null || echo 1)" = "0" ]; then
    # Running as root: default to system-wide /usr/local/bin
    INSTALL_DIR="/usr/local/bin"
elif [ -w "/usr/local/bin" ]; then
    # /usr/local/bin is writable without root
    INSTALL_DIR="/usr/local/bin"
else
    # User-level installation
    INSTALL_DIR="${HOME}/.local/bin"
fi

info "Installing Fusion..."
subinfo "Platform:     $PLATFORM ($TARGET_ARCH)"
subinfo "Target:       $TARGET"
subinfo "Destination:  $INSTALL_DIR/$BINARY"
echo ""

# ── SHA-256 Checksum Helper ───────────────────────────────────────────────────
compute_sha256() {
    target_file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$target_file" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$target_file" | awk '{print $1}'
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$target_file" | awk '{print $NF}'
    elif command -v python3 >/dev/null 2>&1; then
        python3 -c "import hashlib, sys; f=open(sys.argv[1],'rb'); print(hashlib.sha256(f.read()).hexdigest())" "$target_file" 2>/dev/null || echo ""
    elif command -v python >/dev/null 2>&1; then
        python -c "import hashlib, sys; f=open(sys.argv[1],'rb'); print(hashlib.sha256(f.read()).hexdigest())" "$target_file" 2>/dev/null || echo ""
    elif command -v perl >/dev/null 2>&1; then
        perl -MDigest::SHA=sha256_hex -e 'print sha256_hex(<>) . "\n"' "$target_file" 2>/dev/null || echo ""
    else
        echo ""
    fi
}

download_file() {
    url="$1"
    output="$2"
    if [ "$HTTP_CLIENT" = "curl" ]; then
        curl -fsSL "$url" -o "$output"
    else
        wget -qO "$output" "$url"
    fi
}

# ── Fallback: Build from Source via Cargo ─────────────────────────────────────
build_from_cargo() {
    info "Attempting fallback build from source via Cargo..."
    if ! command -v cargo >/dev/null 2>&1; then
        err "No prebuilt release binary available for ${TARGET} and 'cargo' is not installed.
Please install Rust & Cargo (https://rustup.rs) and re-run, or build manually:
  cargo install --git https://github.com/${REPO}.git"
    fi

    info "Building Fusion with cargo install (this may take a few minutes)..."
    mkdir -p "$INSTALL_DIR"
    INSTALL_ROOT="$(dirname "$INSTALL_DIR")"
    if [ "$INSTALL_ROOT" != "/" ] && [ "$INSTALL_ROOT" != "." ] && [ -w "$INSTALL_ROOT" ]; then
        if cargo install --git "https://github.com/${REPO}.git" --bin fusion --root "$INSTALL_ROOT" --force; then
            ok "Successfully built and installed Fusion to $INSTALL_DIR/$BINARY"
            return 0
        fi
    fi

    # Standard cargo install fallback into ~/.cargo/bin then copy
    if cargo install --git "https://github.com/${REPO}.git" --bin fusion --force; then
        CARGO_BIN="${HOME}/.cargo/bin/${BINARY}"
        if [ -f "$CARGO_BIN" ]; then
            cp "$CARGO_BIN" "$INSTALL_DIR/$BINARY"
            ok "Successfully built and installed Fusion to $INSTALL_DIR/$BINARY"
            return 0
        fi
    fi

    err "Cargo build failed. Please report an issue at https://github.com/${REPO}/issues"
}

# ── 2. Download Release Binary ────────────────────────────────────────────────
if [ "$VERSION" = "latest" ]; then
    # Fetch first release from /releases (includes latest release even if marked prerelease)
    LATEST_TAG=$(curl -fsSL ${CURL_AUTH} "https://api.github.com/repos/${REPO}/releases" 2>/dev/null \
        | grep -m1 '"tag_name":' \
        | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/' || true)
    if [ -n "$LATEST_TAG" ]; then
        RELEASE_API_URL="https://api.github.com/repos/${REPO}/releases/tags/${LATEST_TAG}"
        VERSION="$LATEST_TAG"
    else
        RELEASE_API_URL="https://api.github.com/repos/${REPO}/releases/latest"
    fi
else
    RELEASE_API_URL="https://api.github.com/repos/${REPO}/releases/tags/${VERSION}"
fi

# Fetch release metadata from GitHub API
CURL_AUTH=""
if [ -n "${GITHUB_TOKEN:-}" ]; then
    CURL_AUTH="-H Authorization: Bearer ${GITHUB_TOKEN}"
elif [ -n "${GH_TOKEN:-}" ]; then
    CURL_AUTH="-H Authorization: Bearer ${GH_TOKEN}"
fi

RELEASE_JSON=""
if [ "$HTTP_CLIENT" = "curl" ]; then
    # shellcheck disable=SC2086
    RELEASE_JSON=$(curl -fsSL ${CURL_AUTH} "$RELEASE_API_URL" 2>/dev/null || true)
else
    RELEASE_JSON=$(wget -qO- "$RELEASE_API_URL" 2>/dev/null || true)
fi

find_asset_url() {
    suffix="$1"
    if [ -z "$RELEASE_JSON" ]; then
        return
    fi
    printf '%s\n' "$RELEASE_JSON" \
        | grep -o "https://[^\"]*${BINARY}-[^\"]*${suffix}" \
        | grep -v '\.sha256$' \
        | grep "/${BINARY}-[^/]*${suffix}\$" || true
}

DOWNLOAD_URL=""
TARGET_USED=""
ALL_TARGETS="$TARGET $FALLBACK_TARGETS"

# Search candidates from Release API JSON
for t in $ALL_TARGETS; do
    # 1) .tar.gz archive
    url=$(find_asset_url "${t}.tar.gz" | head -n 1)
    if [ -n "$url" ]; then
        DOWNLOAD_URL="$url"
        TARGET_USED="$t"
        break
    fi
    # 2) .zip archive
    url=$(find_asset_url "${t}.zip" | head -n 1)
    if [ -n "$url" ]; then
        DOWNLOAD_URL="$url"
        TARGET_USED="$t"
        break
    fi
    # 3) Bare binary
    url=$(find_asset_url "${t}" | grep -v '\.tar\.gz$' | grep -v '\.zip$' | head -n 1)
    if [ -n "$url" ]; then
        DOWNLOAD_URL="$url"
        TARGET_USED="$t"
        break
    fi
done

# If API lookup was empty (e.g. rate limit), try direct GitHub release download endpoints
if [ -z "$DOWNLOAD_URL" ]; then
    for t in $ALL_TARGETS; do
        if [ "$VERSION" = "latest" ]; then
            DIRECT_URL="https://github.com/${REPO}/releases/latest/download/fusion-${t}.tar.gz"
        else
            DIRECT_URL="https://github.com/${REPO}/releases/download/${VERSION}/fusion-${VERSION}-${t}.tar.gz"
        fi
        if [ "$HTTP_CLIENT" = "curl" ]; then
            if curl -fsIL "$DIRECT_URL" >/dev/null 2>&1; then
                DOWNLOAD_URL="$DIRECT_URL"
                TARGET_USED="$t"
                break
            fi
        else
            if wget --spider -q "$DIRECT_URL" >/dev/null 2>&1; then
                DOWNLOAD_URL="$DIRECT_URL"
                TARGET_USED="$t"
                break
            fi
        fi
    done
fi

INSTALLED_VIA_CARGO=0

# If still no download URL found, fallback to Cargo
if [ -z "$DOWNLOAD_URL" ]; then
    warn "No prebuilt release binary found for target '${TARGET}' on GitHub."
    build_from_cargo
    INSTALLED_VIA_CARGO=1
fi

if [ "$INSTALLED_VIA_CARGO" -eq 0 ]; then
    info "Downloading Fusion (${TARGET_USED})..."
    subinfo "URL: $DOWNLOAD_URL"

    WORK="$(mktemp -d 2>/dev/null || mktemp -d -t 'fusion-install')"
    cleanup() {
        rm -rf "$WORK"
    }
    trap cleanup EXIT INT TERM

    ARCHIVE_FILE="$WORK/downloaded_asset"
    download_file "$DOWNLOAD_URL" "$ARCHIVE_FILE" || err "Failed to download $DOWNLOAD_URL"

    # ── 3. Checksum Verification ──────────────────────────────────────────────
    SHA256_URL="${DOWNLOAD_URL}.sha256"
    CHECKSUM_FILE="$WORK/asset.sha256"

    if download_file "$SHA256_URL" "$CHECKSUM_FILE" 2>/dev/null && [ -s "$CHECKSUM_FILE" ]; then
        EXPECTED_SHA=$(awk '{print $1}' "$CHECKSUM_FILE" | tr -d ' \t\r\n')
        ACTUAL_SHA=$(compute_sha256 "$ARCHIVE_FILE")

        if [ -n "$ACTUAL_SHA" ]; then
            if [ "$ACTUAL_SHA" = "$EXPECTED_SHA" ]; then
                ok "SHA-256 checksum verified: $ACTUAL_SHA"
            else
                err "SHA-256 checksum mismatch!
  Expected: $EXPECTED_SHA
  Actual:   $ACTUAL_SHA
Security verification failed. Aborting installation."
            fi
        else
            warn "Unable to compute SHA-256 (no sha256sum/shasum/openssl/python tool available). Skipping checksum verification."
        fi
    else
        subinfo "No remote .sha256 checksum file found. Proceeding with TLS-verified download."
    fi

    # ── Extract and Place Binary ──────────────────────────────────────────────
    mkdir -p "$INSTALL_DIR"
    case "$DOWNLOAD_URL" in
        *.tar.gz|*.tgz)
            tar -xzf "$ARCHIVE_FILE" -C "$WORK"
            if [ -f "$WORK/$BINARY" ]; then
                FOUND_BIN="$WORK/$BINARY"
            elif [ -f "$WORK/bin/$BINARY" ]; then
                FOUND_BIN="$WORK/bin/$BINARY"
            else
                FOUND_BIN="$(find "$WORK" -type f -name "$BINARY" 2>/dev/null | head -n 1 || true)"
            fi
            if [ -z "$FOUND_BIN" ] || [ ! -f "$FOUND_BIN" ]; then
                err "Downloaded archive did not contain expected binary '$BINARY'"
            fi
            mv "$FOUND_BIN" "$INSTALL_DIR/$BINARY"
            ;;
        *.zip)
            if command -v unzip >/dev/null 2>&1; then
                unzip -q "$ARCHIVE_FILE" -d "$WORK"
                if [ -f "$WORK/$BINARY" ]; then
                    FOUND_BIN="$WORK/$BINARY"
                elif [ -f "$WORK/bin/$BINARY" ]; then
                    FOUND_BIN="$WORK/bin/$BINARY"
                else
                    FOUND_BIN="$(find "$WORK" -type f -name "$BINARY" 2>/dev/null | head -n 1 || true)"
                fi
                if [ -z "$FOUND_BIN" ] || [ ! -f "$FOUND_BIN" ]; then
                    err "Downloaded zip archive did not contain expected binary '$BINARY'"
                fi
                mv "$FOUND_BIN" "$INSTALL_DIR/$BINARY"
            else
                err "'unzip' command is required to extract the zip archive."
            fi
            ;;
        *)
            mv "$ARCHIVE_FILE" "$INSTALL_DIR/$BINARY"
            ;;
    esac

    chmod +x "$INSTALL_DIR/$BINARY"
fi

# Smoke check
if [ ! -x "$INSTALL_DIR/$BINARY" ]; then
    err "Installation failed: $INSTALL_DIR/$BINARY is not executable"
fi

echo ""
ok "Fusion successfully installed to $INSTALL_DIR/$BINARY"

# Show installed version if binary is executable on current host
if "$INSTALL_DIR/$BINARY" --version >/dev/null 2>&1; then
    INSTALLED_VER="$("$INSTALL_DIR/$BINARY" --version 2>/dev/null | head -n 1)"
    subinfo "$INSTALLED_VER"
fi

# ── 4. Automatic PATH Configuration ───────────────────────────────────────────
PATH_UPDATED=""
case ":$PATH:" in
    *":${INSTALL_DIR}:"*)
        # Already in PATH
        ;;
    *)
        info "Configuring PATH..."

        # 1. Fish shell (~/.config/fish/config.fish)
        FISH_CONFIG="${HOME}/.config/fish/config.fish"
        if [ -d "${HOME}/.config/fish" ] || [ -f "$FISH_CONFIG" ] || [ "${SHELL:-}" = "fish" ] || [ "${SHELL##*/}" = "fish" ]; then
            mkdir -p "${HOME}/.config/fish" 2>/dev/null || true
            touch "$FISH_CONFIG" 2>/dev/null || true
            if [ -w "$FISH_CONFIG" ] && ! grep -q "fish_add_path.*${INSTALL_DIR}" "$FISH_CONFIG" 2>/dev/null && ! grep -q "${INSTALL_DIR}" "$FISH_CONFIG" 2>/dev/null; then
                printf '\n# Added by Fusion installer\nfish_add_path %s\n' "$INSTALL_DIR" >> "$FISH_CONFIG"
                PATH_UPDATED="${PATH_UPDATED}  - $FISH_CONFIG\n"
            fi
        fi

        # 2. Zsh shell (~/.zshrc)
        ZSH_CONFIG="${HOME}/.zshrc"
        if [ -f "$ZSH_CONFIG" ] || [ "${SHELL:-}" = "zsh" ] || [ "${SHELL##*/}" = "zsh" ]; then
            touch "$ZSH_CONFIG" 2>/dev/null || true
            if [ -w "$ZSH_CONFIG" ] && ! grep -q "${INSTALL_DIR}" "$ZSH_CONFIG" 2>/dev/null; then
                printf '\n# Added by Fusion installer\nexport PATH="%s:$PATH"\n' "$INSTALL_DIR" >> "$ZSH_CONFIG"
                PATH_UPDATED="${PATH_UPDATED}  - $ZSH_CONFIG\n"
            fi
        fi

        # 3. Bash shell (~/.bashrc)
        BASH_CONFIG="${HOME}/.bashrc"
        if [ -f "$BASH_CONFIG" ] || [ "${SHELL:-}" = "bash" ] || [ "${SHELL##*/}" = "bash" ]; then
            touch "$BASH_CONFIG" 2>/dev/null || true
            if [ -w "$BASH_CONFIG" ] && ! grep -q "${INSTALL_DIR}" "$BASH_CONFIG" 2>/dev/null; then
                printf '\n# Added by Fusion installer\nexport PATH="%s:$PATH"\n' "$INSTALL_DIR" >> "$BASH_CONFIG"
                PATH_UPDATED="${PATH_UPDATED}  - $BASH_CONFIG\n"
            fi
        fi

        # 4. macOS login shell (~/.zprofile) or POSIX fallback (~/.profile)
        if [ "$PLATFORM" = "macos" ]; then
            MAC_PROFILE="${HOME}/.zprofile"
            if [ -f "$MAC_PROFILE" ] && [ -w "$MAC_PROFILE" ] && ! grep -q "${INSTALL_DIR}" "$MAC_PROFILE" 2>/dev/null; then
                printf '\n# Added by Fusion installer\nexport PATH="%s:$PATH"\n' "$INSTALL_DIR" >> "$MAC_PROFILE"
                PATH_UPDATED="${PATH_UPDATED}  - $MAC_PROFILE\n"
            fi
        elif [ -f "${HOME}/.profile" ] && [ -w "${HOME}/.profile" ] && ! grep -q "${INSTALL_DIR}" "${HOME}/.profile" 2>/dev/null; then
            if [ -z "$PATH_UPDATED" ]; then
                printf '\n# Added by Fusion installer\nexport PATH="%s:$PATH"\n' "$INSTALL_DIR" >> "${HOME}/.profile"
                PATH_UPDATED="${PATH_UPDATED}  - ${HOME}/.profile\n"
            fi
        fi

        if [ -n "$PATH_UPDATED" ]; then
            ok "Added $INSTALL_DIR to PATH in:"
            printf "%b" "$PATH_UPDATED"
            echo ""
            info "To apply PATH changes to your current terminal session, run:"
            if [ -f "${HOME}/.zshrc" ] && [ "${SHELL##*/}" = "zsh" ]; then
                echo "  source ~/.zshrc"
            elif [ -f "${HOME}/.bashrc" ]; then
                echo "  source ~/.bashrc"
            elif [ -f "${HOME}/.config/fish/config.fish" ] && [ "${SHELL##*/}" = "fish" ]; then
                echo "  source ~/.config/fish/config.fish"
            else
                echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
            fi
        else
            warn "Please ensure $INSTALL_DIR is in your PATH:"
            echo ""
            echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
            echo ""
        fi
        ;;
esac

echo ""
ok "To get started, sign in and run Fusion:"
echo ""
echo "  fusion login"
echo "  fusion"
echo ""
