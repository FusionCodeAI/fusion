#!/usr/bin/env bash
# ==============================================================================
# Fusion — Android Termux Bootstrap & Environment Setup
# ==============================================================================
# Complete, foolproof environment setup for running and compiling Fusion
# on Android devices inside Termux.
#
# Key Tasks Handled:
# 1. Termux environment validation ($PREFIX, writable $TMPDIR initialization)
# 2. Package management & minimal dependencies (rust, git, curl, clang, binutils)
# 3. Optimized build flags for mobile CPUs (AArch64 / ARMv7 / x86_64)
# 4. Binary installation test and Bash / Zsh / Fish autocompletions
# ==============================================================================

set -euo pipefail

# ANSI color codes (automatically disabled if stdout is not a TTY)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    CYAN='\033[0;36m'
    BOLD='\033[1m'
    DIM='\033[2m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    CYAN=''
    BOLD=''
    DIM=''
    NC=''
fi

log_info()  { printf "${BLUE}==>${NC} ${BOLD}%s${NC}\n" "$*"; }
log_ok()    { printf "${GREEN}✓${NC} %s\n" "$*"; }
log_warn()  { printf "${YELLOW}⚠${NC} %s\n" "$*"; }
log_err()   { printf "${RED}✗${NC} %s\n" "$*" >&2; }
log_step()  { printf "\n${CYAN}──> %s${NC}\n" "$*"; }

# ------------------------------------------------------------------------------
# Usage & Help
# ------------------------------------------------------------------------------
usage() {
    cat <<EOF
${BOLD}Fusion Android Termux Bootstrap${NC}

${BOLD}USAGE:${NC}
    bash scripts/termux-bootstrap.sh [OPTIONS]

${BOLD}OPTIONS:${NC}
    -h, --help               Show this help message and exit
    --skip-pkgs              Skip package installation (pkg update & pkg install)
    --build-from-source      Force building Fusion from source using cargo
    --install-prebuilt       Force downloading the prebuilt release binary
    --completions-only       Only generate and configure shell auto-completions
    --force                  Bypass Termux environment detection checks
    --verbose                Enable verbose command logging

${BOLD}EXAMPLES:${NC}
    bash scripts/termux-bootstrap.sh
    bash scripts/termux-bootstrap.sh --build-from-source
    bash scripts/termux-bootstrap.sh --completions-only
EOF
}

# ------------------------------------------------------------------------------
# Command-line options
# ------------------------------------------------------------------------------
SKIP_PKGS=false
BUILD_FROM_SOURCE=false
INSTALL_PREBUILT=false
COMPLETIONS_ONLY=false
FORCE=false
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        --skip-pkgs)
            SKIP_PKGS=true
            shift
            ;;
        --build-from-source)
            BUILD_FROM_SOURCE=true
            shift
            ;;
        --install-prebuilt)
            INSTALL_PREBUILT=true
            shift
            ;;
        --completions-only)
            COMPLETIONS_ONLY=true
            shift
            ;;
        --force)
            FORCE=true
            shift
            ;;
        --verbose)
            VERBOSE=true
            set -x
            shift
            ;;
        *)
            log_err "Unknown argument: $1"
            usage
            exit 1
            ;;
    esac
done

echo ""
printf "${BOLD}${CYAN}  ███████╗██╗   ██╗███████╗██╗ ██████╗ ███╗   ██╗${NC}\n"
printf "${BOLD}${CYAN}  ██╔════╝██║   ██║██╔════╝██║██╔═══██╗████╗  ██║${NC}\n"
printf "${BOLD}${CYAN}  █████╗  ██║   ██║███████╗██║██║   ██║██╔██╗ ██║${NC}\n"
printf "${BOLD}${CYAN}  ██╔══╝  ██║   ██║╚════██║██║██║   ██║██║╚██╗██║${NC}\n"
printf "${BOLD}${CYAN}  ██║     ╚██████╔╝███████║██║╚██████╔╝██║ ╚████║${NC}\n"
printf "${BOLD}${CYAN}  ╚═╝      ╚═════╝ ╚══════╝╚═╝ ╚═════╝ ╚═╝  ╚═══╝${NC}\n"
printf "  ${DIM}Pure Rust AI Coding Assistant — Android Termux Bootstrap${NC}\n"
echo ""

# ------------------------------------------------------------------------------
# Step 1: Check for Termux Environment & Configure Storage/Temp
# ------------------------------------------------------------------------------
log_step "1/4 Checking Termux environment & directory layout"

IS_TERMUX=false
DEFAULT_PREFIX="/data/data/com.termux/files/usr"

if [ -n "${PREFIX:-}" ] && [ -d "${PREFIX}" ]; then
    IS_TERMUX=true
elif [ -d "${DEFAULT_PREFIX}" ]; then
    export PREFIX="${DEFAULT_PREFIX}"
    IS_TERMUX=true
elif [ -n "${TERMUX_VERSION:-}" ]; then
    IS_TERMUX=true
    export PREFIX="${DEFAULT_PREFIX}"
fi

if [ "$IS_TERMUX" = false ]; then
    if [ "$FORCE" = true ]; then
        log_warn "Termux prefix not found, but --force was provided. Proceeding with PREFIX=${PREFIX:-/usr/local}..."
        export PREFIX="${PREFIX:-/usr/local}"
    else
        log_err "Termux environment not detected."
        log_err "Expected \$PREFIX or '${DEFAULT_PREFIX}' to exist."
        log_err "If you are running in a custom chroot or proot, run with --force."
        exit 1
    fi
fi

log_ok "Termux environment verified (PREFIX=${PREFIX})"

# Android's /tmp is non-standard, restricted, or read-only without root.
# We must ensure $TMPDIR points to a writable directory inside $PREFIX.
export TMPDIR="${PREFIX}/tmp"
export TMP="${TMPDIR}"
export TEMP="${TMPDIR}"

# Create all necessary runtime and config directories
mkdir -p "${TMPDIR}"
mkdir -p "${PREFIX}/bin"
mkdir -p "${HOME}/.fusion/tmp"
mkdir -p "${HOME}/.fusion/bin"
mkdir -p "${HOME}/.config/fusion"

log_ok "Temporary directory initialized: ${TMPDIR}"
log_ok "Fusion directory initialized: ${HOME}/.fusion"

# Ensure PATH includes Termux bin, Cargo bin, and Fusion bin
case ":${PATH}:" in
    *":${PREFIX}/bin:"*) ;;
    *) export PATH="${PREFIX}/bin:${PATH}" ;;
esac
case ":${PATH}:" in
    *":${HOME}/.cargo/bin:"*) ;;
    *) export PATH="${HOME}/.cargo/bin:${PATH}" ;;
esac
case ":${PATH}:" in
    *":${HOME}/.fusion/bin:"*) ;;
    *) export PATH="${HOME}/.fusion/bin:${PATH}" ;;
esac

# ------------------------------------------------------------------------------
# Step 2: Install Minimal Required Packages
# ------------------------------------------------------------------------------
if [ "$COMPLETIONS_ONLY" = false ]; then
    log_step "2/4 Verifying and installing required packages"

    # Core required packages:
    # - rust: Rust compiler (rustc) & Cargo package manager
    # - git: Workspace version control & repo cloning
    # - curl: HTTP download tool for installer and APIs
    # - clang: Native C/C++ compiler and linker for Android NDK
    # - binutils: Linker (ld), llvm-ar, strip, and binary inspection tools
    # - ca-certificates: Secure TLS trust store for HTTPS/API connectivity
    # - ripgrep: High-speed codebase search engine used by Fusion tools
    # - openssl: Native cryptographic libraries
    # - pkg-config: C library detection for Cargo dependencies
    REQUIRED_PACKAGES=(
        "rust"
        "git"
        "curl"
        "clang"
        "binutils"
        "ca-certificates"
        "ripgrep"
        "openssl"
        "pkg-config"
    )

    # Optional Termux helpers (e.g. termux-api for mobile notifications/clipboard)
    OPTIONAL_PACKAGES=(
        "termux-api"
    )

    if [ "$SKIP_PKGS" = true ]; then
        log_info "Skipping package updates as requested (--skip-pkgs)"
    else
        # Determine package manager
        PKG_MGR=""
        if command -v pkg >/dev/null 2>&1; then
            PKG_MGR="pkg"
        elif command -v apt-get >/dev/null 2>&1; then
            PKG_MGR="apt-get"
        elif command -v apt >/dev/null 2>&1; then
            PKG_MGR="apt"
        fi

        if [ -n "$PKG_MGR" ]; then
            log_info "Updating package catalog with $PKG_MGR..."
            "$PKG_MGR" update -y || log_warn "Package update encountered non-critical warnings"

            log_info "Installing core packages: ${REQUIRED_PACKAGES[*]}..."
            "$PKG_MGR" install -y "${REQUIRED_PACKAGES[@]}" || {
                log_warn "Some packages failed to install in batch, attempting individual installation..."
                for pkg in "${REQUIRED_PACKAGES[@]}"; do
                    "$PKG_MGR" install -y "$pkg" || log_warn "Failed to install $pkg (may already be provided or built-in)"
                done
            }

            # Attempt optional packages quietly
            for opt in "${OPTIONAL_PACKAGES[@]}"; do
                "$PKG_MGR" install -y "$opt" >/dev/null 2>&1 || true
            done
        else
            log_warn "No supported package manager (pkg/apt) found. Skipping package installation."
        fi
    fi

    # Verify essential command availability
    MISSING_COMMANDS=()
    for cmd in rustc cargo git curl clang; do
        if ! command -v "$cmd" >/dev/null 2>&1; then
            MISSING_COMMANDS+=("$cmd")
        fi
    done

    if [ ${#MISSING_COMMANDS[@]} -eq 0 ]; then
        log_ok "All core build and runtime packages are verified: rustc, cargo, git, curl, clang"
    else
        log_warn "The following recommended commands are missing: ${MISSING_COMMANDS[*]}"
        log_warn "Prebuilt binaries can still be installed, but source builds may fail."
    fi
else
    log_info "Skipping package installation (--completions-only mode)"
fi

# ------------------------------------------------------------------------------
# Step 3: Set Optimized Build Flags for AArch64 / ARMv7 Mobile Devices
# ------------------------------------------------------------------------------
log_step "3/4 Configuring optimized mobile build flags and environment"

ARCH="$(uname -m 2>/dev/null || echo "unknown")"
log_info "Detected hardware architecture: ${BOLD}${ARCH}${NC}"

case "${ARCH}" in
    aarch64|arm64)
        TARGET_ARCH="aarch64"
        CARGO_BUILD_TARGET="aarch64-linux-android"
        # Mobile optimization: fast native CPU code, minimal binary bloat, single codegen unit
        RUSTFLAGS="-C target-cpu=native -C opt-level=3 -C codegen-units=1 -C panic=abort"
        CFLAGS="-O3 -pipe -fomit-frame-pointer"
        CXXFLAGS="-O3 -pipe -fomit-frame-pointer"
        MAX_PARALLEL_JOBS=4
        ;;
    armv7l|armv7|armv8l|armhf|arm)
        TARGET_ARCH="armv7"
        CARGO_BUILD_TARGET="armv7-linux-androideabi"
        # ARM 32-bit: conserve virtual memory space, enable NEON if supported
        RUSTFLAGS="-C target-cpu=generic -C opt-level=2 -C codegen-units=1 -C panic=abort"
        CFLAGS="-O2 -pipe -march=armv7-a -mfpu=neon-vfpv4 -mfloat-abi=softfp"
        CXXFLAGS="-O2 -pipe -march=armv7-a -mfpu=neon-vfpv4 -mfloat-abi=softfp"
        MAX_PARALLEL_JOBS=2
        ;;
    x86_64)
        TARGET_ARCH="x86_64"
        CARGO_BUILD_TARGET="x86_64-linux-android"
        RUSTFLAGS="-C target-cpu=native -C opt-level=3 -C codegen-units=1 -C panic=abort"
        CFLAGS="-O3 -pipe -fomit-frame-pointer"
        CXXFLAGS="-O3 -pipe -fomit-frame-pointer"
        MAX_PARALLEL_JOBS=4
        ;;
    i686|x86)
        TARGET_ARCH="i686"
        CARGO_BUILD_TARGET="i686-linux-android"
        RUSTFLAGS="-C target-cpu=generic -C opt-level=2 -C codegen-units=1 -C panic=abort"
        CFLAGS="-O2 -pipe"
        CXXFLAGS="-O2 -pipe"
        MAX_PARALLEL_JOBS=2
        ;;
    *)
        TARGET_ARCH="${ARCH}"
        CARGO_BUILD_TARGET=""
        RUSTFLAGS="-C opt-level=2 -C codegen-units=1"
        CFLAGS="-O2"
        CXXFLAGS="-O2"
        MAX_PARALLEL_JOBS=2
        ;;
esac

# Mobile CPU throttling and memory constraint protection:
# Prevent out-of-memory (OOM) killer on multi-core phones during heavy compilation
SYSTEM_CORES="$(nproc 2>/dev/null || echo 2)"
if [ "$SYSTEM_CORES" -gt "$MAX_PARALLEL_JOBS" ]; then
    CARGO_BUILD_JOBS="$MAX_PARALLEL_JOBS"
else
    CARGO_BUILD_JOBS="$SYSTEM_CORES"
fi

# Set native C compiler and linker toolchain from Clang / LLVM in Termux
export CC="clang"
export CXX="clang++"
if command -v llvm-ar >/dev/null 2>&1; then
    export AR="llvm-ar"
elif command -v ar >/dev/null 2>&1; then
    export AR="ar"
fi

export RUSTFLAGS="${RUSTFLAGS}"
export CFLAGS="${CFLAGS}"
export CXXFLAGS="${CXXFLAGS}"
export CARGO_BUILD_TARGET="${CARGO_BUILD_TARGET}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS}"

log_ok "Optimized build configuration for ${TARGET_ARCH}:"
log_info "  CARGO_BUILD_TARGET = ${CARGO_BUILD_TARGET}"
log_info "  CARGO_BUILD_JOBS   = ${CARGO_BUILD_JOBS} (capped for mobile stability)"
log_info "  RUSTFLAGS          = ${RUSTFLAGS}"
log_info "  CC / CXX / AR      = ${CC} / ${CXX} / ${AR:-ar}"

# Persist environment variables in ~/.fusion/env
ENV_FILE="${HOME}/.fusion/env"
cat > "${ENV_FILE}" <<EOF
# Fusion Android Termux Environment Configuration
# Auto-generated by scripts/termux-bootstrap.sh on $(date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date)

export PREFIX="${PREFIX}"
export TMPDIR="${TMPDIR}"
export TMP="${TMPDIR}"
export TEMP="${TMPDIR}"

# Paths
export PATH="${PREFIX}/bin:${HOME}/.cargo/bin:${HOME}/.fusion/bin:\$PATH"

# Compiler & Linker Toolchain
export CC="${CC}"
export CXX="${CXX}"
${AR:+export AR="${AR}"}

# Mobile Hardware & Cargo Optimizations
export RUSTFLAGS="${RUSTFLAGS}"
export CFLAGS="${CFLAGS}"
export CXXFLAGS="${CXXFLAGS}"
export CARGO_BUILD_TARGET="${CARGO_BUILD_TARGET}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS}"
EOF
chmod 600 "${ENV_FILE}"
log_ok "Saved persistent environment to ${ENV_FILE}"

# Hook into ~/.bashrc and ~/.zshrc if present
inject_profile_hook() {
    local rc_file="$1"
    local hook_line='[ -f "$HOME/.fusion/env" ] && . "$HOME/.fusion/env"'
    if [ -f "$rc_file" ]; then
        if ! grep -Fq "$hook_line" "$rc_file" 2>/dev/null; then
            echo "" >> "$rc_file"
            echo "# Load Fusion environment" >> "$rc_file"
            echo "$hook_line" >> "$rc_file"
            log_ok "Configured environment hook in $(basename "$rc_file")"
        fi
    fi
}

inject_profile_hook "${HOME}/.bashrc"
inject_profile_hook "${HOME}/.zshrc"
inject_profile_hook "${HOME}/.profile"

# ------------------------------------------------------------------------------
# Step 4: Binary Installation Test & Shell Auto-Completions
# ------------------------------------------------------------------------------
log_step "4/4 Testing binary installation & configuring shell completions"

# Locate existing Fusion binary
find_fusion_bin() {
    if command -v fusion >/dev/null 2>&1; then
        command -v fusion
    elif [ -x "${PREFIX}/bin/fusion" ]; then
        echo "${PREFIX}/bin/fusion"
    elif [ -x "${HOME}/.cargo/bin/fusion" ]; then
        echo "${HOME}/.cargo/bin/fusion"
    elif [ -x "${HOME}/.fusion/bin/fusion" ]; then
        echo "${HOME}/.fusion/bin/fusion"
    elif [ -x "./target/release/fusion" ]; then
        echo "./target/release/fusion"
    else
        echo ""
    fi
}

FUSION_BIN="$(find_fusion_bin)"

# Handle build from source or prebuilt install if binary missing or requested
if [ "$BUILD_FROM_SOURCE" = true ]; then
    log_info "Building Fusion from source with cargo release profile..."
    if [ -f "Cargo.toml" ]; then
        cargo build --release --target "${CARGO_BUILD_TARGET}" || cargo build --release
        if [ -x "target/${CARGO_BUILD_TARGET}/release/fusion" ]; then
            cp "target/${CARGO_BUILD_TARGET}/release/fusion" "${PREFIX}/bin/fusion"
        elif [ -x "target/release/fusion" ]; then
            cp "target/release/fusion" "${PREFIX}/bin/fusion"
        fi
        FUSION_BIN="${PREFIX}/bin/fusion"
    else
        log_err "Cargo.toml not found in current directory. Cannot build from source."
        exit 1
    fi
elif [ "$INSTALL_PREBUILT" = true ] || [ -z "$FUSION_BIN" ]; then
    if [ "$COMPLETIONS_ONLY" = false ]; then
        log_info "No existing Fusion binary found. Installing latest prebuilt release..."
        if [ -f "scripts/install.sh" ]; then
            bash scripts/install.sh || log_warn "Local installer script returned non-zero"
        else
            curl -fsSL https://fusioncode.app/install | bash || log_warn "Online installer returned non-zero"
        fi
        FUSION_BIN="$(find_fusion_bin)"
    fi
fi

# Verify binary execution
if [ -n "$FUSION_BIN" ] && [ -x "$FUSION_BIN" ]; then
    log_ok "Found executable binary at: ${FUSION_BIN}"
    VERSION_OUTPUT="$("$FUSION_BIN" --version 2>&1 || true)"
    if [ -n "$VERSION_OUTPUT" ]; then
        log_ok "Binary verification passed: ${VERSION_OUTPUT}"
    else
        log_warn "Binary executed but returned empty version string."
    fi
else
    log_warn "Fusion binary is not yet in PATH. Run 'scripts/install.sh' or 'cargo build --release' to install."
fi

# Configure Shell Auto-Completions
log_info "Configuring shell completions for Bash, Zsh, and Fish..."

# 1. Bash completion configuration
BASH_COMP_DIRS=(
    "${PREFIX}/etc/bash_completion.d"
    "${PREFIX}/share/bash-completion/completions"
    "${HOME}/.bash_completion.d"
)
BASH_TARGET_DIR=""
for dir in "${BASH_COMP_DIRS[@]}"; do
    if mkdir -p "$dir" 2>/dev/null; then
        BASH_TARGET_DIR="$dir"
        break
    fi
done

if [ -n "$BASH_TARGET_DIR" ]; then
    BASH_COMP_FILE="${BASH_TARGET_DIR}/fusion"
    if [ -n "$FUSION_BIN" ] && "$FUSION_BIN" --generate-completion bash > "${BASH_COMP_FILE}" 2>/dev/null; then
        log_ok "Generated Bash completions: ${BASH_COMP_FILE}"
    else
        # Fallback static bash completion definition
        cat > "${BASH_COMP_FILE}" <<'EOF'
_fusion_completion() {
    local cur prev words cword
    _init_completion || return
    local commands="login logout chat config update doctor models providers acp help"
    local flags="--help -h --version -V --model -m --provider -p --generate-completion --verbose"
    if [[ $cword -eq 1 ]]; then
        COMPREPLY=( $(compgen -W "${commands} ${flags}" -- "$cur") )
    else
        case "${prev}" in
            --generate-completion)
                COMPREPLY=( $(compgen -W "bash zsh fish powershell elvish" -- "$cur") )
                ;;
            --provider|-p)
                COMPREPLY=( $(compgen -W "openai anthropic deepseek groq openrouter gemini ollama cloudflare bedrock" -- "$cur") )
                ;;
            *)
                COMPREPLY=( $(compgen -f -- "$cur") )
                ;;
        esac
    fi
}
complete -F _fusion_completion fusion
EOF
        log_ok "Installed fallback Bash completion script: ${BASH_COMP_FILE}"
    fi

    # Ensure ~/.bashrc sources bash completions if present
    if [ -f "${HOME}/.bashrc" ]; then
        BASHRC_HOOK="[ -f \"${BASH_COMP_FILE}\" ] && . \"${BASH_COMP_FILE}\""
        if ! grep -Fq "${BASH_COMP_FILE}" "${HOME}/.bashrc" 2>/dev/null; then
            echo "" >> "${HOME}/.bashrc"
            echo "# Fusion Bash autocompletion" >> "${HOME}/.bashrc"
            echo "${BASHRC_HOOK}" >> "${HOME}/.bashrc"
        fi
    fi
fi

# 2. Zsh completion configuration
ZSH_COMP_DIRS=(
    "${PREFIX}/share/zsh/site-functions"
    "${HOME}/.zsh/completion"
)
ZSH_TARGET_DIR=""
for dir in "${ZSH_COMP_DIRS[@]}"; do
    if mkdir -p "$dir" 2>/dev/null; then
        ZSH_TARGET_DIR="$dir"
        break
    fi
done

if [ -n "$ZSH_TARGET_DIR" ]; then
    ZSH_COMP_FILE="${ZSH_TARGET_DIR}/_fusion"
    if [ -n "$FUSION_BIN" ] && "$FUSION_BIN" --generate-completion zsh > "${ZSH_COMP_FILE}" 2>/dev/null; then
        log_ok "Generated Zsh completions: ${ZSH_COMP_FILE}"
    else
        # Fallback static zsh completion definition
        cat > "${ZSH_COMP_FILE}" <<'EOF'
#compdef fusion

_fusion() {
    local -a commands flags
    commands=(
        'login:Authenticate with AI provider or Fusion account'
        'logout:Clear stored authentication credentials'
        'chat:Start interactive terminal coding session'
        'config:Manage configuration settings'
        'models:List available AI models and pricing'
        'providers:Manage model providers and keys'
        'acp:Launch Agent Client Protocol JSON-RPC server'
        'doctor:Run system diagnostics and verify toolchain'
    )
    flags=(
        '--help[Show help information]'
        '--version[Show version information]'
        '--model[Specify LLM model]'
        '--provider[Specify provider]'
        '--generate-completion[Generate shell completions]'
    )
    _arguments -s \
        '1: :->command' \
        '*: :->args' && return 0

    case $state in
        command)
            _describe -t commands 'fusion command' commands
            ;;
    esac
}

_fusion "$@"
EOF
        log_ok "Installed fallback Zsh completion script: ${ZSH_COMP_FILE}"
    fi

    # Ensure ~/.zshrc configures fpath and compinit
    if [ -f "${HOME}/.zshrc" ]; then
        if ! grep -Fq "${ZSH_TARGET_DIR}" "${HOME}/.zshrc" 2>/dev/null; then
            echo "" >> "${HOME}/.zshrc"
            echo "# Fusion Zsh autocompletion" >> "${HOME}/.zshrc"
            echo "fpath=(\"${ZSH_TARGET_DIR}\" \$fpath)" >> "${HOME}/.zshrc"
            echo "autoload -Uz compinit && compinit -C" >> "${HOME}/.zshrc"
        fi
    fi
fi

# 3. Fish completion configuration
FISH_COMP_DIR="${PREFIX}/share/fish/vendor_completions.d"
if mkdir -p "${FISH_COMP_DIR}" 2>/dev/null; then
    FISH_COMP_FILE="${FISH_COMP_DIR}/fusion.fish"
    if [ -n "$FUSION_BIN" ] && "$FUSION_BIN" --generate-completion fish > "${FISH_COMP_FILE}" 2>/dev/null; then
        log_ok "Generated Fish completions: ${FISH_COMP_FILE}"
    fi
fi

# ------------------------------------------------------------------------------
# Summary & Next Steps
# ------------------------------------------------------------------------------
echo ""
log_ok "Fusion Termux bootstrap completed successfully!"
echo ""
printf "${BOLD}Summary:${NC}\n"
printf "  • Prefix:       ${CYAN}%s${NC}\n" "${PREFIX}"
printf "  • Temp dir:     ${CYAN}%s${NC}\n" "${TMPDIR}"
printf "  • Config:       ${CYAN}%s${NC}\n" "${HOME}/.config/fusion/fusion.toml"
printf "  • Env profile:  ${CYAN}%s${NC}\n" "${ENV_FILE}"
if [ -n "$FUSION_BIN" ]; then
    printf "  • Binary:       ${GREEN}%s${NC}\n" "${FUSION_BIN}"
fi
echo ""
printf "${BOLD}Quick Start:${NC}\n"
printf "  1. Reload shell environment:\n"
printf "     ${DIM}source ~/.bashrc${NC}  (or ${DIM}source ~/.fusion/env${NC})\n\n"
printf "  2. Sign in or configure providers:\n"
printf "     ${GREEN}fusion login${NC}\n\n"
printf "  3. Start interactive coding session:\n"
printf "     ${GREEN}fusion${NC}\n\n"
printf "  4. Verify system diagnostics:\n"
printf "     ${GREEN}fusion doctor${NC}\n"
echo ""
EOF
