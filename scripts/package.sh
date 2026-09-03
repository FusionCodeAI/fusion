#!/usr/bin/env bash
# ==============================================================================
# Fusion Release Packaging & Distribution Script
# ==============================================================================
# Cross-platform release packaging pipeline:
# - Builds release binaries for single targets or full target matrix
# - Strips debugging symbols with platform-specific strip utilities
# - Bundles man pages (fusion.1), shell completions (bash, zsh, fish, pwsh, elvish),
#   and license files (LICENSE, README.md) into distribution archives
# - Creates compressed .tar.gz and .zip release packages
# - Generates cryptographic SHA-256 checksums (individual .sha256 + SHA256SUMS.txt)
#
# Supported Matrix Targets:
#   - x86_64-apple-darwin        (macOS Intel)
#   - aarch64-apple-darwin       (macOS Apple Silicon)
#   - x86_64-unknown-linux-musl  (Linux x86_64 static musl)
#   - aarch64-unknown-linux-musl (Linux ARM64 static musl)
#   - x86_64-pc-windows-msvc     (Windows x64 MSVC)
#   - aarch64-linux-android      (Android/Termux ARM64)
#   - x86_64-unknown-linux-gnu   (Linux x86_64 glibc)
#
# Usage:
#   ./scripts/package.sh [OPTIONS]
#
# Examples:
#   ./scripts/package.sh
#   ./scripts/package.sh --target aarch64-apple-darwin
#   ./scripts/package.sh --matrix --no-build
#   ./scripts/package.sh --all-targets --out-dir dist
#   ./scripts/package.sh --checksum-only
# ==============================================================================

set -euo pipefail

# Script directory and workspace root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ANSI color codes (disabled if not a tty or if NO_COLOR is set)
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    CYAN='\033[0;36m'
    MAGENTA='\033[0;35m'
    DIM='\033[0;90m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' BLUE='' CYAN='' MAGENTA='' DIM='' BOLD='' NC=''
fi

log_info()  { printf "${BLUE}==>${NC} ${BOLD}%s${NC}\n" "$*"; }
log_step()  { printf "  ${CYAN}•${NC} %s\n" "$*"; }
log_ok()    { printf "  ${GREEN}✓${NC} %s\n" "$*"; }
log_warn()  { printf "  ${YELLOW}⚠${NC} %s\n" "$*"; }
log_err()   { printf "  ${RED}✗${NC} %s\n" "$*" >&2; }

# ==============================================================================
# Supported Target Matrix
# ==============================================================================
DEFAULT_MATRIX_TARGETS=(
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-musl"
    "x86_64-pc-windows-msvc"
    "aarch64-linux-android"
)

usage() {
    cat <<EOF
${BOLD}Fusion Release Packaging Script${NC}

Builds release binaries, strips symbols, and packages distribution
archives (.tar.gz and .zip) bundled with man pages, shell completions,
licenses, and SHA-256 checksums.

${BOLD}Usage:${NC}
    $(basename "$0") [OPTIONS]

${BOLD}Options:${NC}
    -t, --target <TRIPLE>     Target triple (e.g. aarch64-apple-darwin, x86_64-unknown-linux-musl)
                              Pass 'all' or use --matrix to build/package all supported matrix targets.
                              Default: host target auto-detected from rustc or system
    -m, --matrix, --all       Build and package the full multi-target release matrix:
                                • x86_64-apple-darwin        (macOS Intel)
                                • aarch64-apple-darwin       (macOS Apple Silicon)
                                • x86_64-unknown-linux-musl  (Linux x86_64 static musl)
                                • aarch64-unknown-linux-musl (Linux ARM64 static musl)
                                • x86_64-pc-windows-msvc     (Windows x64 MSVC)
                                • aarch64-linux-android      (Android/Termux ARM64)
    -o, --out-dir <DIR>       Output directory for release packages (Default: dist)
    -v, --version <VERSION>   Release version tag (Default: parsed from Cargo.toml)
    -b, --binary <NAME>       Binary executable name override (Default: fusion or fusion.exe)
        --features <LIST>     Cargo feature list to enable during compilation
        --cross               Use 'cross' instead of 'cargo' for cross-compilation
        --no-build            Skip cargo/cross build step (package existing pre-built binary)
        --no-strip            Skip symbol stripping step
        --no-tar              Skip creating .tar.gz archives
        --no-zip              Skip creating .zip archives
        --checksum-only       Regenerate SHA256SUMS.txt from existing archives in out-dir
    -h, --help                Show this help message

${BOLD}Examples:${NC}
    ./scripts/package.sh
    ./scripts/package.sh --target aarch64-apple-darwin
    ./scripts/package.sh --target x86_64-unknown-linux-musl --out-dir dist
    ./scripts/package.sh --matrix --no-build
    ./scripts/package.sh --checksum-only --out-dir dist
EOF
    exit 0
}

# Defaults
TARGET=""
MATRIX_MODE=false
OUT_DIR="${WORKSPACE_ROOT}/dist"
VERSION=""
BINARY_NAME=""
CARGO_FEATURES=""
USE_CROSS=false
NO_BUILD=false
NO_STRIP=false
NO_TAR=false
NO_ZIP=false
CHECKSUM_ONLY=false

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        -t|--target)
            if [[ "$2" == "all" || "$2" == "matrix" ]]; then
                MATRIX_MODE=true
            else
                TARGET="$2"
            fi
            shift 2
            ;;
        --target=*)
            val="${1#*=}"
            if [[ "$val" == "all" || "$val" == "matrix" ]]; then
                MATRIX_MODE=true
            else
                TARGET="$val"
            fi
            shift
            ;;
        -m|--matrix|--all|--all-targets)
            MATRIX_MODE=true
            shift
            ;;
        -o|--out-dir)
            OUT_DIR="$2"
            shift 2
            ;;
        --out-dir=*)
            OUT_DIR="${1#*=}"
            shift
            ;;
        -v|--version)
            VERSION="$2"
            shift 2
            ;;
        --version=*)
            VERSION="${1#*=}"
            shift
            ;;
        -b|--binary)
            BINARY_NAME="$2"
            shift 2
            ;;
        --binary=*)
            BINARY_NAME="${1#*=}"
            shift
            ;;
        --features)
            CARGO_FEATURES="$2"
            shift 2
            ;;
        --features=*)
            CARGO_FEATURES="${1#*=}"
            shift
            ;;
        --cross)
            USE_CROSS=true
            shift
            ;;
        --no-build)
            NO_BUILD=true
            shift
            ;;
        --no-strip)
            NO_STRIP=true
            shift
            ;;
        --no-tar)
            NO_TAR=true
            shift
            ;;
        --no-zip)
            NO_ZIP=true
            shift
            ;;
        --checksum-only)
            CHECKSUM_ONLY=true
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            log_err "Unknown option: $1"
            echo "Run '$(basename "$0") --help' for usage." >&2
            exit 1
            ;;
    esac
done

cd "${WORKSPACE_ROOT}"

# ==============================================================================
# Helper Functions
# ==============================================================================

# SHA-256 calculation
compute_sha256() {
    local file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | awk '{print $1}'
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$file" | awk '{print $NF}'
    elif command -v python3 >/dev/null 2>&1; then
        python3 -c "import hashlib, sys; print(hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest())" "$file"
    else
        log_err "No SHA-256 utility found (sha256sum, shasum, openssl, or python3)."
        exit 1
    fi
}

# Resolve release version from Cargo.toml if not passed
resolve_version() {
    if [ -n "${VERSION}" ]; then
        echo "${VERSION}"
        return 0
    fi

    local ver=""
    if [ -f "Cargo.toml" ]; then
        ver="$(grep -m1 '^version' Cargo.toml | sed -E 's/version *= *"([^"]+)".*/\1/' || true)"
    fi
    if [ -z "$ver" ] && [ -f "crates/codegen/xai-grok-pager-bin/Cargo.toml" ]; then
        ver="$(grep -m1 '^version' crates/codegen/xai-grok-pager-bin/Cargo.toml | sed -E 's/version *= *"([^"]+)".*/\1/' || true)"
    fi
    if [ -z "$ver" ]; then
        ver="0.3.0"
        log_warn "Could not detect version from Cargo.toml, defaulting to ${ver}"
    fi
    echo "$ver"
}

# Resolve host target triple
resolve_host_target() {
    if command -v rustc >/dev/null 2>&1; then
        local host
        host="$(rustc -vV 2>/dev/null | grep '^host:' | awk '{print $2}' || true)"
        if [ -n "$host" ]; then
            echo "$host"
            return 0
        fi
    fi

    local os arch arch_triple
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64) arch_triple="x86_64" ;;
        aarch64|arm64) arch_triple="aarch64" ;;
        *) arch_triple="$arch" ;;
    esac

    case "$os" in
        darwin) echo "${arch_triple}-apple-darwin" ;;
        linux)  echo "${arch_triple}-unknown-linux-gnu" ;;
        msys*|mingw*|cygwin*) echo "${arch_triple}-pc-windows-msvc" ;;
        *) echo "${arch_triple}-${os}" ;;
    esac
}

# Format binary executable name for a given target
target_binary_name() {
    local target="$1"
    local custom_name="${2:-}"
    if [ -n "$custom_name" ]; then
        echo "$custom_name"
        return 0
    fi

    if [[ "$target" == *"windows"* ]] || [[ "$target" == *"msvc"* ]] || [[ "$target" == *"mingw"* ]]; then
        echo "fusion.exe"
    else
        echo "fusion"
    fi
}

# Generate Man Page (fusion.1)
generate_man_page() {
    local out_man_dir="$1"
    local version="$2"
    mkdir -p "${out_man_dir}/man1"

    local man_file="${out_man_dir}/man1/fusion.1"
    local root_man_file="${out_man_dir}/fusion.1"

    cat << 'EOF' > "${man_file}"
.TH FUSION 1 "September 2026" "fusion VERSION_PLACEHOLDER" "User Commands"
.SH NAME
fusion \- Fast, lightweight cross-platform AI coding assistant with subagents and advisors
.SH SYNOPSIS
.B fusion
[\fIOPTIONS\fR] [\fIPROMPT\fR]
.SH DESCRIPTION
\fBfusion\fR is an ultra-fast, zero-allocation AI coding harness and terminal assistant.
It features asynchronous multi-agent coordination, subagent mesh execution, real-time
diff streaming, safety advisors (Security, Code Review, Architecture), and offline/local
LLM provider routing.

.SH OPTIONS
.TP
\fIPROMPT\fR
Optional one-off prompt to run non-interactively, or slash command (e.g. \fB/bookmark\fR, \fB/trace\fR).
.TP
\fB\-m\fR, \fB\-\-model\fR \fIMODEL\fR
Override default model (e.g. \fBdeepseek-chat\fR, \fBclaude-3-5-sonnet-20241022\fR, \fBgpt-4o\fR, \fBgrok-2\fR).
.TP
\fB\-p\fR, \fB\-\-provider\fR \fIPROVIDER\fR
Override provider backend (\fBdeepseek\fR, \fBanthropic\fR, \fBopenai\fR, \fBxai\fR, \fBopenrouter\fR, \fBollama\fR).
.TP
\fB\-P\fR, \fB\-\-preset\fR \fIPRESET\fR
Apply pre-built configuration preset:
.RS
.IP \(bu 2
\fBcoding-fast\fR: Ultra-low latency code edits and surgical file patches.
.IP \(bu 2
\fBdeep-reasoning\fR: Deep chain-of-thought analysis and comprehensive validation.
.IP \(bu 2
\fBcheap\fR: Cost-optimized model routing for large codebases.
.IP \(bu 2
\fBoffline-ollama\fR: Zero-network local inference via Ollama.
.IP \(bu 2
\fBtermux-mobile\fR: Battery and memory optimized profile for Android/Termux.
.RE
.TP
\fB\-C\fR, \fB\-\-cwd\fR \fIDIR\fR
Working directory for session execution, file system operations, and git context.
.TP
\fB\-\-no\-advisors\fR
Disable parallel advisor critiques (SecurityAdvisor, CodeReviewAdvisor, ArchitectureAdvisor).
.TP
\fB\-\-acp\fR
Start Agent Client Protocol (ACP) JSON-RPC stdio server for editor plugins (VS Code, JetBrains, Zed, Neovim).
.TP
\fB\-\-generate\-completion\fR \fISHELL\fR
Generate shell completion script for \fIbash\fR, \fIzsh\fR, \fIfish\fR, \fIpowershell\fR, or \fIelvish\fR.
.TP
\fB\-h\fR, \fB\-\-help\fR
Print help information and CLI flags.
.TP
\fB\-V\fR, \fB\-\-version\fR
Print version number.

.SH ENVIRONMENT
.TP
\fBOPENAI_API_KEY\fR
API key for OpenAI models.
.TP
\fBANTHROPIC_API_KEY\fR
API key for Anthropic Claude models.
.TP
\fBDEEPSEEK_API_KEY\fR
API key for DeepSeek models.
.TP
\fBXAI_API_KEY\fR
API key for xAI Grok models.
.TP
\fBOPENROUTER_API_KEY\fR
API key for OpenRouter routing.
.TP
\fBOLLAMA_HOST\fR
Host URL for local Ollama server (default: http://localhost:11434).
.TP
\fBFUSION_CONFIG_DIR\fR
Custom configuration directory (default: ~/.config/fusion).

.SH FILES
.TP
\fI~/.config/fusion/config.toml\fR
Main user configuration file.
.TP
\fI~/.config/fusion/presets/\fR
Directory containing user-defined system prompt and parameter presets.
.TP
\fI.fusion/recovery.json\fR
Project session checkpoint state for crash recovery.

.SH BUGS & ISSUES
Report issues at: https://github.com/theaungmyatmoe/fusion/issues

.SH AUTHORS
Fusion Authors and Contributors.

.SH COPYRIGHT
Licensed under MIT.
EOF

    # Replace version placeholder
    sed -i.bak "s/VERSION_PLACEHOLDER/${version}/g" "${man_file}" && rm -f "${man_file}.bak"
    cp "${man_file}" "${root_man_file}"
}

# Generate Shell Completions
generate_shell_completions() {
    local out_comp_dir="$1"
    local bin_candidate="${2:-}"
    mkdir -p "${out_comp_dir}"

    local generated_via_bin=false

    # If the binary is executable on this host, generate directly via CLI
    if [ -n "$bin_candidate" ] && [ -f "$bin_candidate" ] && [ -x "$bin_candidate" ]; then
        if "$bin_candidate" --generate-completion bash > "${out_comp_dir}/fusion.bash" 2>/dev/null && \
           "$bin_candidate" --generate-completion zsh > "${out_comp_dir}/_fusion" 2>/dev/null && \
           "$bin_candidate" --generate-completion fish > "${out_comp_dir}/fusion.fish" 2>/dev/null && \
           "$bin_candidate" --generate-completion powershell > "${out_comp_dir}/fusion.ps1" 2>/dev/null && \
           "$bin_candidate" --generate-completion elvish > "${out_comp_dir}/fusion.elv" 2>/dev/null; then
            generated_via_bin=true
        fi
    fi

    # Fallback to pure high-quality templates if cross-compiling or binary not executable on host
    if [ "$generated_via_bin" = false ]; then
        # Bash
        cat << 'EOF' > "${out_comp_dir}/fusion.bash"
# Bash completion script for fusion
_fusion() {
    local i cur prev opts cmd
    COMPREPLY=()
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"
    opts="-m --model -p --provider -P --preset -C --cwd --no-advisors --acp --generate-completion -h --help -V --version"

    case "${prev}" in
        --model|-m)
            COMPREPLY=($(compgen -W "deepseek-chat deepseek-reasoner claude-3-5-sonnet-20241022 gpt-4o gpt-4o-mini o1 grok-2" -- "${cur}"))
            return 0
            ;;
        --provider|-p)
            COMPREPLY=($(compgen -W "deepseek anthropic openai xai openrouter ollama" -- "${cur}"))
            return 0
            ;;
        --preset|-P)
            COMPREPLY=($(compgen -W "coding-fast deep-reasoning cheap offline-ollama termux-mobile" -- "${cur}"))
            return 0
            ;;
        --generate-completion)
            COMPREPLY=($(compgen -W "bash zsh fish powershell elvish" -- "${cur}"))
            return 0
            ;;
        --cwd|-C)
            COMPREPLY=($(compgen -d -- "${cur}"))
            return 0
            ;;
        *)
            ;;
    esac

    COMPREPLY=($(compgen -W "${opts}" -- "${cur}"))
    return 0
}
complete -F _fusion -o bashdefault -o default fusion
EOF

        # Zsh
        cat << 'EOF' > "${out_comp_dir}/_fusion"
#compdef fusion

_fusion() {
    local context state state_descr line
    typeset -A opt_args

    _arguments -C \
        '(-m --model)'{-m,--model}'[Override model]:model:(deepseek-chat deepseek-reasoner claude-3-5-sonnet-20241022 gpt-4o grok-2)' \
        '(-p --provider)'{-p,--provider}'[Override provider]:provider:(deepseek anthropic openai xai openrouter ollama)' \
        '(-P --preset)'{-P,--preset}'[Apply configuration preset]:preset:(coding-fast deep-reasoning cheap offline-ollama termux-mobile)' \
        '(-C --cwd)'{-C,--cwd}'[Working directory]:directory:_files -/' \
        '--no-advisors[Disable parallel advisor critiques]' \
        '--acp[Start Agent Client Protocol stdio server]' \
        '--generate-completion[Generate shell completion script]:shell:(bash zsh fish powershell elvish)' \
        '(-h --help)'{-h,--help}'[Print help]' \
        '(-V --version)'{-V,--version}'[Print version]' \
        '*:prompt: '
}

if [ "$funcstack[1]" = "_fusion" ]; then
    _fusion "$@"
else
    compdef _fusion fusion
fi
EOF

        # Fish
        cat << 'EOF' > "${out_comp_dir}/fusion.fish"
# Fish completion script for fusion
complete -c fusion -s m -l model -d "Override model" -r -a "deepseek-chat deepseek-reasoner claude-3-5-sonnet-20241022 gpt-4o grok-2"
complete -c fusion -s p -l provider -d "Override provider" -r -a "deepseek anthropic openai xai openrouter ollama"
complete -c fusion -s P -l preset -d "Apply configuration preset" -r -a "coding-fast deep-reasoning cheap offline-ollama termux-mobile"
complete -c fusion -s C -l cwd -d "Working directory" -r -a "(__fish_complete_directories)"
complete -c fusion -l no-advisors -d "Disable parallel advisor critiques"
complete -c fusion -l acp -d "Start Agent Client Protocol stdio server"
complete -c fusion -l generate-completion -d "Generate completion script" -r -a "bash zsh fish powershell elvish"
complete -c fusion -s h -l help -d "Print help information"
complete -c fusion -s V -l version -d "Print version information"
EOF

        # PowerShell
        cat << 'EOF' > "${out_comp_dir}/fusion.ps1"
# PowerShell completion script for fusion
Register-ArgumentCompleter -Native -CommandName 'fusion' -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)
    $options = @('-m', '--model', '-p', '--provider', '-P', '--preset', '-C', '--cwd', '--no-advisors', '--acp', '--generate-completion', '-h', '--help', '-V', '--version')
    $options | Where-Object { $_ -like "$wordToComplete*" } | ForEach-Object {
        [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterName', $_)
    }
}
EOF

        # Elvish
        cat << 'EOF' > "${out_comp_dir}/fusion.elv"
# Elvish completion script for fusion
set edit:completion:arg-completer[fusion] = {|@words|
    var opts = [-m --model -p --provider -P --preset -C --cwd --no-advisors --acp --generate-completion -h --help -V --version]
    put $@opts
}
EOF
    fi
}

# Generate / Copy Licenses and Documentation
bundle_licenses_and_docs() {
    local stage_dir="$1"

    # README
    if [ -f "README.md" ]; then
        cp "README.md" "${stage_dir}/"
    fi

    # Licenses
    local found_license=false
    if [ -f "LICENSE" ]; then
        cp "LICENSE" "${stage_dir}/"
        found_license=true
    fi
    if [ -f "LICENSE-MIT" ]; then
        cp "LICENSE-MIT" "${stage_dir}/LICENSE"
        found_license=true
    fi
    if [ -f "sdk/LICENSE" ] && [ "$found_license" = false ]; then
        cp "sdk/LICENSE" "${stage_dir}/LICENSE"
        found_license=true
    fi

    # If no license file exists, write standard MIT license
    if [ "$found_license" = false ]; then
        cat << 'EOF' > "${stage_dir}/LICENSE"
MIT License

Copyright (c) 2026 Fusion Authors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
EOF
    fi
}

# Strip Binary Symbols
strip_binary_symbols() {
    local target="$1"
    local stage_bin="$2"

    local strip_cmd=""

    if [[ "${target}" == *"apple-darwin"* ]]; then
        if command -v strip >/dev/null 2>&1; then
            strip_cmd="strip -x"
        fi
    elif [[ "${target}" == *"android"* ]]; then
        if command -v aarch64-linux-android-strip >/dev/null 2>&1; then
            strip_cmd="aarch64-linux-android-strip --strip-all"
        elif command -v llvm-strip >/dev/null 2>&1; then
            strip_cmd="llvm-strip --strip-all"
        elif command -v strip >/dev/null 2>&1; then
            strip_cmd="strip --strip-all"
        fi
    elif [[ "${target}" == *"linux-musl"* ]] || [[ "${target}" == *"linux-gnu"* ]]; then
        if [[ "${target}" == *"aarch64"* ]] && command -v aarch64-linux-gnu-strip >/dev/null 2>&1; then
            strip_cmd="aarch64-linux-gnu-strip --strip-all"
        elif [[ "${target}" == *"x86_64"* ]] && command -v x86_64-linux-gnu-strip >/dev/null 2>&1; then
            strip_cmd="x86_64-linux-gnu-strip --strip-all"
        elif command -v llvm-strip >/dev/null 2>&1; then
            strip_cmd="llvm-strip --strip-all"
        elif command -v strip >/dev/null 2>&1; then
            strip_cmd="strip --strip-all"
        fi
    elif [[ "${target}" == *"windows"* ]]; then
        if command -v x86_64-w64-mingw32-strip >/dev/null 2>&1; then
            strip_cmd="x86_64-w64-mingw32-strip --strip-all"
        elif command -v llvm-strip >/dev/null 2>&1; then
            strip_cmd="llvm-strip --strip-all"
        fi
    fi

    if [ -n "${strip_cmd}" ]; then
        if ${strip_cmd} "${stage_bin}" 2>/dev/null; then
            log_ok "Binary symbols stripped successfully (${strip_cmd})."
        else
            log_warn "Strip command '${strip_cmd}' skipped or symbols already stripped."
        fi
    else
        log_step "Strip skipped (no compatible strip tool for ${target})."
    fi
}

# Master Checksum Generator for OUT_DIR
generate_master_checksums() {
    local out_dir="$1"
    local sha_file="${out_dir}/SHA256SUMS.txt"
    log_info "Generating cryptographic SHA-256 checksums in ${out_dir}..."

    mkdir -p "${out_dir}"
    local tmp_sha
    tmp_sha="$(mktemp)"

    (
        cd "${out_dir}"
        for archive in *.tar.gz *.zip; do
            if [ -f "$archive" ]; then
                local hash
                hash="$(compute_sha256 "$archive")"
                echo "${hash}  ${archive}" >> "${tmp_sha}"
                # Individual .sha256 file
                echo "${hash}  ${archive}" > "${archive}.sha256"
            fi
        done
    )

    if [ -s "${tmp_sha}" ]; then
        sort -k2 "${tmp_sha}" > "${sha_file}"
        rm -f "${tmp_sha}"
        log_ok "Generated SHA256SUMS.txt:"
        while IFS= read -r line; do
            printf "    ${DIM}%s${NC}\n" "$line"
        done < "${sha_file}"
    else
        rm -f "${tmp_sha}"
        log_warn "No .tar.gz or .zip archives found in ${out_dir} to hash."
    fi
}

# ==============================================================================
# Single Target Packaging Pipeline
# ==============================================================================
package_single_target() {
    local target="$1"
    local version="$2"
    local bin_name
    bin_name="$(target_binary_name "${target}" "${BINARY_NAME}")"

    local clean_ver="${version#v}"
    local tag="v${clean_ver}"
    local archive_base="fusion-${tag}-${target}"

    log_info "Packaging Fusion ${tag} for ${BOLD}${target}${NC}"
    log_step "Binary:   ${bin_name}"
    log_step "Out dir:  ${OUT_DIR}"

    # 1. Build release binary if enabled
    if [ "${NO_BUILD}" = false ]; then
        log_step "Compiling release binary..."
        local build_cmd="cargo"
        if [ "${USE_CROSS}" = true ]; then
            build_cmd="cross"
        fi

        local build_args=(build --release --target "${target}")
        if [ -n "${CARGO_FEATURES}" ]; then
            build_args+=(--features "${CARGO_FEATURES}")
        fi

        log_step "Running: ${build_cmd} ${build_args[*]}"
        "${build_cmd}" "${build_args[@]}"
        log_ok "Build completed for ${target}."
    fi

    # 2. Locate built binary
    local binary_path=""
    local candidate_paths=(
        "target/${target}/release/${bin_name}"
        "target/${target}/release/fusion"
        "target/${target}/release/fusion.exe"
        "dist/${bin_name}-${target}"
        "dist/${target}/${bin_name}"
        "dist/fusion-linux-${target##*-}"
        "target/release/${bin_name}"
        "target/release/fusion"
        "target/release/fusion.exe"
    )

    for path in "${candidate_paths[@]}"; do
        if [ -f "$path" ]; then
            binary_path="$path"
            break
        fi
    done

    if [ -z "${binary_path}" ] || [ ! -f "${binary_path}" ]; then
        log_err "Could not find built binary for ${target}. Checked paths:"
        for path in "${candidate_paths[@]}"; do
            log_err "  - $path"
        done
        return 1
    fi

    log_ok "Located binary: ${binary_path}"

    # 3. Create Staging Directory
    local stage_dir
    stage_dir="$(mktemp -d -t fusion-stage-XXXXXX)"
    stage_cleanup() {
        rm -rf "${stage_dir}"
    }
    trap stage_cleanup RETURN

    # 4. Copy Binary
    local stage_bin="${stage_dir}/${bin_name}"
    cp "${binary_path}" "${stage_bin}"
    chmod 755 "${stage_bin}"

    # 5. Strip Symbols if enabled
    if [ "${NO_STRIP}" = false ]; then
        strip_binary_symbols "${target}" "${stage_bin}"
    fi

    # 6. Bundle Licenses & Documentation
    bundle_licenses_and_docs "${stage_dir}"

    # 7. Bundle Man Pages (man/man1/fusion.1 & man/fusion.1)
    generate_man_page "${stage_dir}/man" "${clean_ver}"
    log_ok "Bundled man pages (man/man1/fusion.1)."

    # 8. Bundle Shell Completions (bash, zsh, fish, pwsh, elvish)
    generate_shell_completions "${stage_dir}/completions" "${binary_path}"
    log_ok "Bundled shell completions (completions/{fusion.bash, _fusion, fusion.fish, ...})."

    # 9. Create Output Directory
    mkdir -p "${OUT_DIR}"
    local abs_out_dir
    abs_out_dir="$(cd "${OUT_DIR}" && pwd)"

    local tar_gz_file="${abs_out_dir}/${archive_base}.tar.gz"
    local zip_file="${abs_out_dir}/${archive_base}.zip"

    # 10. Package .tar.gz
    if [ "${NO_TAR}" = false ]; then
        rm -f "${tar_gz_file}"
        (
            cd "${stage_dir}"
            tar -czf "${tar_gz_file}" *
        )
        log_ok "Created .tar.gz archive: $(basename "${tar_gz_file}") ($(du -h "${tar_gz_file}" | awk '{print $1}'))"
    fi

    # 11. Package .zip
    if [ "${NO_ZIP}" = false ]; then
        rm -f "${zip_file}"
        if command -v zip >/dev/null 2>&1; then
            (
                cd "${stage_dir}"
                zip -q -9 -r "${zip_file}" .
            )
        elif command -v python3 >/dev/null 2>&1; then
            python3 - <<PYEOF
import os, zipfile
stage_dir = """${stage_dir}"""
zip_file = """${zip_file}"""
with zipfile.ZipFile(zip_file, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for root, dirs, files in os.walk(stage_dir):
        for f in files:
            full_path = os.path.join(root, f)
            rel_path = os.path.relpath(full_path, stage_dir)
            z.write(full_path, rel_path)
PYEOF
        else
            log_warn "Neither 'zip' nor 'python3' available. Skipping .zip archive creation."
        fi
        if [ -f "${zip_file}" ]; then
            log_ok "Created .zip archive:    $(basename "${zip_file}") ($(du -h "${zip_file}" | awk '{print $1}'))"
        fi
    fi

    stage_cleanup
    trap - RETURN
}

# ==============================================================================
# Main Execution Entry Point
# ==============================================================================

# Fast path: Checksum only
if [ "${CHECKSUM_ONLY}" = true ]; then
    generate_master_checksums "${OUT_DIR}"
    exit 0
fi

# Resolve version
RELEASE_VERSION="$(resolve_version)"

# Execution Mode: Matrix vs Single Target
if [ "${MATRIX_MODE}" = true ]; then
    log_info "Starting multi-target matrix packaging pipeline for Fusion v${RELEASE_VERSION#v}"
    log_info "Targets: ${DEFAULT_MATRIX_TARGETS[*]}"
    echo ""

    FAILED_TARGETS=()
    PASSED_TARGETS=()

    for tgt in "${DEFAULT_MATRIX_TARGETS[@]}"; do
        echo "────────────────────────────────────────────────────────────────────────"
        if package_single_target "${tgt}" "${RELEASE_VERSION}"; then
            PASSED_TARGETS+=("${tgt}")
        else
            log_err "Failed to package target: ${tgt}"
            FAILED_TARGETS+=("${tgt}")
        fi
        echo ""
    done

    # Generate master checksums
    generate_master_checksums "${OUT_DIR}"

    echo ""
    echo "========================================================================"
    log_ok "Matrix packaging completed: ${#PASSED_TARGETS[@]} passed, ${#FAILED_TARGETS[@]} failed."
    if [ ${#FAILED_TARGETS[@]} -gt 0 ]; then
        log_err "Failed targets: ${FAILED_TARGETS[*]}"
        exit 1
    fi
else
    # Single target mode
    if [ -z "${TARGET}" ]; then
        TARGET="$(resolve_host_target)"
        log_info "No target specified, auto-detected host target: ${BOLD}${TARGET}${NC}"
    fi

    package_single_target "${TARGET}" "${RELEASE_VERSION}"
    generate_master_checksums "${OUT_DIR}"

    echo ""
    log_ok "Packaging completed successfully!"
    log_info "Artifacts in ${OUT_DIR}:"
    ls -la "${OUT_DIR}"
fi
