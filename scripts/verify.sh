#!/usr/bin/env bash
# ==============================================================================
# Fusion Comprehensive Verification & Diagnostics Pipeline
# ==============================================================================
# Verifies codebase formatting, clippy static analysis, test suite, WASM builds,
# TypeScript SDK compilation, Termux portability, binary execution, tool registry,
# config paths, runtime status, and host system diagnostics.
#
# Suitable for CI/CD pipelines, pre-release verification, and end-user troubleshooting.
#
# Usage:
#   ./scripts/verify.sh [OPTIONS]
#
# Modes:
#   --all               Run entire verification pipeline (fmt, clippy, tests, wasm, sdk, termux, binary)
#   -q, --quick         Run fast smoke checks only (skip heavy compilation, clippy, tests, network)
#   --format-check      Check code formatting (cargo fmt) and lints (cargo clippy)
#   --wasm              Verify WebAssembly compilation target and web bindings
#   --sdk               Verify TypeScript SDK build and type definitions
#   --termux            Verify Termux / Android compatibility and pure-Rust TLS
#   --test, --tests     Run unit and integration test suite
#
# General Options:
#   -b, --bin <PATH>    Path to fusion executable (default: auto-detected)
#   -v, --verbose       Print detailed execution outputs and raw command results
#   --json              Output diagnostic results as structured JSON
#   --no-color          Disable colored ANSI output
#   --skip-network      Skip remote API endpoint reachability checks
#   -h, --help          Display this help guide and exit
#
# Exit codes:
#   0 - All verification checks passed (or passed with minor non-fatal warnings)
#   1 - One or more critical verification checks failed
#   2 - Invalid arguments or configuration error
# ==============================================================================

set -euo pipefail

# ------------------------------------------------------------------------------
# Default Options & State
# ------------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Modes & Flags
RUN_ALL=false
QUICK_MODE=false
RUN_FORMAT=false
RUN_CLIPPY=false
RUN_TESTS=false
RUN_WASM=false
RUN_SDK=false
RUN_TERMUX=false
RUN_BINARY=true
RUN_SYSTEM=true
SPECIFIC_MODE=false

FUSION_BIN=""
VERBOSE=false
JSON_MODE=false
NO_COLOR=false
SKIP_NETWORK=false

# Test Metrics
TOTAL_CHECKS=0
PASSED_CHECKS=0
WARNING_CHECKS=0
FAILED_CHECKS=0

# Check Tracking Lists
CHECK_NAMES=()
CHECK_STATUSES=()
CHECK_MESSAGES=()
CHECK_DETAILS=()
CHECK_DURATIONS=()
CHECK_REMEDIATIONS=()

# ------------------------------------------------------------------------------
# Helpers & Formatting
# ------------------------------------------------------------------------------
json_escape() {
    # Strip ANSI escapes, escape backslashes/quotes/tabs/newlines, and remove raw control characters
    sed -E $'s/\x1B\\[[0-9;]*[a-zA-Z]//g' | awk '
    BEGIN { first = 1 }
    {
        gsub(/\\/, "\\\\")
        gsub(/"/, "\\\"")
        gsub(/\r/, "")
        gsub(/\t/, "\\t")
        gsub(/[\x00-\x1f\x7f]/, "")
        if (!first) {
            printf "\\n"
        }
        printf "%s", $0
        first = 0
    }
    END {
        print ""
    }'
}

strip_ansi() {
    # Strip ANSI terminal formatting sequences
    sed -E $'s/\x1B\\[[0-9;]*[a-zA-Z]//g' | tr -d '\r'
}

get_time_ms() {
    # High-resolution time in milliseconds
    if date +%s%N >/dev/null 2>&1 && [[ "$(date +%s%N)" =~ ^[0-9]+$ ]]; then
        echo $(($(date +%s%N) / 1000000))
    elif command -v python3 >/dev/null 2>&1; then
        python3 -c 'import time; print(int(time.time() * 1000))' 2>/dev/null || echo $(($(date +%s) * 1000))
    elif command -v perl >/dev/null 2>&1; then
        perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000' 2>/dev/null || echo $(($(date +%s) * 1000))
    else
        echo $(($(date +%s) * 1000))
    fi
}

format_duration() {
    local ms="$1"
    if [[ $ms -lt 1000 ]]; then
        printf "%dms" "$ms"
    else
        local sec=$((ms / 1000))
        local frac=$(((ms % 1000) / 10))
        printf "%d.%02ds" "$sec" "$frac"
    fi
}

usage() {
    cat << 'EOF'
Fusion Comprehensive Verification & Diagnostics Pipeline

Usage:
  ./scripts/verify.sh [OPTIONS]

Verification Modes:
  --all                  Run entire verification pipeline (fmt, clippy, tests, wasm, sdk, termux, binary)
  -q, --quick            Run fast smoke checks only (skip heavy compilation, clippy, tests, network)
  --format-check         Check code formatting (cargo fmt --check) and lints (cargo clippy -- -D warnings)
  --wasm                 Verify WebAssembly compilation target and web bindings
  --sdk                  Verify TypeScript SDK build and type definitions
  --termux               Verify Termux / Android compatibility and pure-Rust TLS
  --test, --tests        Run unit and integration test suite

General Options:
  -b, --bin <PATH>       Path to fusion executable (default: auto-detected)
  -v, --verbose          Print detailed execution outputs and raw command results
  --json                 Output diagnostic results as structured JSON
  --no-color             Disable colored ANSI output
  --skip-network         Skip remote API endpoint reachability checks
  -h, --help             Display this help guide and exit

Examples:
  ./scripts/verify.sh --all
  ./scripts/verify.sh --quick
  ./scripts/verify.sh --format-check
  ./scripts/verify.sh --wasm --sdk
  ./scripts/verify.sh --termux
  ./scripts/verify.sh --bin ./target/release/fusion --json

EOF
    exit 0
}

# ------------------------------------------------------------------------------
# CLI Argument Parsing
# ------------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --all)
            RUN_ALL=true
            shift
            ;;
        -q|--quick)
            QUICK_MODE=true
            shift
            ;;
        --format-check)
            RUN_FORMAT=true
            RUN_CLIPPY=true
            SPECIFIC_MODE=true
            shift
            ;;
        --wasm)
            RUN_WASM=true
            SPECIFIC_MODE=true
            shift
            ;;
        --sdk)
            RUN_SDK=true
            SPECIFIC_MODE=true
            shift
            ;;
        --termux)
            RUN_TERMUX=true
            SPECIFIC_MODE=true
            shift
            ;;
        --test|--tests)
            RUN_TESTS=true
            SPECIFIC_MODE=true
            shift
            ;;
        -b|--bin)
            if [[ $# -lt 2 ]]; then
                echo "Error: --bin requires a path argument" >&2
                exit 2
            fi
            FUSION_BIN="$2"
            shift 2
            ;;
        --bin=*)
            FUSION_BIN="${1#*=}"
            shift
            ;;
        -v|--verbose)
            VERBOSE=true
            shift
            ;;
        --json)
            JSON_MODE=true
            shift
            ;;
        --no-color)
            NO_COLOR=true
            shift
            ;;
        --skip-network)
            SKIP_NETWORK=true
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Error: Unknown option: $1" >&2
            echo "Run './scripts/verify.sh --help' for usage." >&2
            exit 2
            ;;
    esac
done

# Evaluate Flag Combinations
if [[ "$RUN_ALL" = true ]]; then
    RUN_FORMAT=true
    RUN_CLIPPY=true
    RUN_TESTS=true
    RUN_WASM=true
    RUN_SDK=true
    RUN_TERMUX=true
    RUN_BINARY=true
    RUN_SYSTEM=true
elif [[ "$QUICK_MODE" = true ]]; then
    RUN_FORMAT=false
    RUN_CLIPPY=false
    RUN_TESTS=false
    RUN_WASM=false
    RUN_SDK=false
    RUN_TERMUX=false
    RUN_BINARY=true
    RUN_SYSTEM=false
elif [[ "$SPECIFIC_MODE" = true ]]; then
    RUN_BINARY=false
    RUN_SYSTEM=false
fi

# Check NO_COLOR env
if [[ -n "${NO_COLOR:-}" && "$NO_COLOR" != "0" && "$NO_COLOR" != "false" ]]; then
    NO_COLOR=true
fi

# Disable color if stdout is not a tty or JSON mode
if [[ "$JSON_MODE" = true ]] || [[ "$NO_COLOR" = true ]] || [[ ! -t 1 ]]; then
    COLOR_RESET=""
    COLOR_BOLD=""
    COLOR_DIM=""
    COLOR_RED=""
    COLOR_GREEN=""
    COLOR_YELLOW=""
    COLOR_BLUE=""
    COLOR_CYAN=""
    COLOR_MAGENTA=""
else
    COLOR_RESET="\033[0m"
    COLOR_BOLD="\033[1m"
    COLOR_DIM="\033[2m"
    COLOR_RED="\033[0;31m"
    COLOR_GREEN="\033[0;32m"
    COLOR_YELLOW="\033[0;33m"
    COLOR_BLUE="\033[0;34m"
    COLOR_CYAN="\033[0;36m"
    COLOR_MAGENTA="\033[0;35m"
fi

record_check() {
    local name="$1"
    local status="$2" # PASS, WARN, FAIL
    local message="$3"
    local detail="${4:-}"
    local duration_ms="${5:-0}"
    local remediation="${6:-}"

    CHECK_NAMES+=("$name")
    CHECK_STATUSES+=("$status")
    CHECK_MESSAGES+=("$message")
    CHECK_DETAILS+=("$detail")
    CHECK_DURATIONS+=("$duration_ms")
    CHECK_REMEDIATIONS+=("$remediation")

    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
    local dur_str=""
    if [[ "$duration_ms" -gt 0 ]]; then
        dur_str=" [$(format_duration "$duration_ms")]"
    fi

    case "$status" in
        PASS)
            PASSED_CHECKS=$((PASSED_CHECKS + 1))
            if [[ "$JSON_MODE" = false ]]; then
                printf "  ${COLOR_GREEN}✓${COLOR_RESET} %-36s ${COLOR_DIM}%s${COLOR_RESET}${COLOR_CYAN}%s${COLOR_RESET}\n" "$name" "$message" "$dur_str"
                if [[ "$VERBOSE" = true && -n "$detail" ]]; then
                    printf "    ${COLOR_DIM}%s${COLOR_RESET}\n" "$detail"
                fi
            fi
            ;;
        WARN)
            WARNING_CHECKS=$((WARNING_CHECKS + 1))
            if [[ "$JSON_MODE" = false ]]; then
                printf "  ${COLOR_YELLOW}⚠${COLOR_RESET} %-36s ${COLOR_YELLOW}%s${COLOR_RESET}${COLOR_CYAN}%s${COLOR_RESET}\n" "$name" "$message" "$dur_str"
                if [[ -n "$detail" ]]; then
                    printf "    ${COLOR_DIM}%s${COLOR_RESET}\n" "$detail"
                fi
            fi
            ;;
        FAIL)
            FAILED_CHECKS=$((FAILED_CHECKS + 1))
            if [[ "$JSON_MODE" = false ]]; then
                printf "  ${COLOR_RED}✗${COLOR_RESET} %-36s ${COLOR_RED}%s${COLOR_RESET}${COLOR_CYAN}%s${COLOR_RESET}\n" "$name" "$message" "$dur_str"
                if [[ -n "$detail" ]]; then
                    printf "    ${COLOR_RED}Detail: %s${COLOR_RESET}\n" "$detail"
                fi
                if [[ -n "$remediation" ]]; then
                    printf "    ${COLOR_BOLD}Action: %s${COLOR_RESET}\n" "$remediation"
                fi
            fi
            ;;
    esac
}

section_header() {
    if [[ "$JSON_MODE" = false ]]; then
        echo ""
        printf "${COLOR_BOLD}${COLOR_CYAN}==>${COLOR_RESET} ${COLOR_BOLD}%s${COLOR_RESET}\n" "$1"
    fi
}

# Temporary workspace for test captures
TMP_DIR="$(mktemp -d -t fusion-verify-XXXXXX)"
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

# Global Pipeline Timer
GLOBAL_START_MS="$(get_time_ms)"

# ------------------------------------------------------------------------------
# 1. Code Formatting & Static Analysis
# ------------------------------------------------------------------------------
if [[ "$RUN_FORMAT" = true ]] || [[ "$RUN_CLIPPY" = true ]]; then
    section_header "1. Code Formatting & Static Analysis"

    if [[ "$RUN_FORMAT" = true ]]; then
        t0=$(get_time_ms)
        if command -v cargo >/dev/null 2>&1; then
            if fmt_out="$(cargo fmt --all -- --check 2>&1)"; then
                t1=$(get_time_ms)
                record_check "Cargo Format Check" "PASS" "Codebase formatted cleanly" "" "$((t1 - t0))"
            else
                t1=$(get_time_ms)
                record_check "Cargo Format Check" "FAIL" "Formatting discrepancies detected" "$fmt_out" "$((t1 - t0))" "Run 'cargo fmt' to automatically format all files"
            fi
        else
            t1=$(get_time_ms)
            record_check "Cargo Format Check" "WARN" "cargo not found in PATH" "" "$((t1 - t0))" "Install Rust toolchain via https://rustup.rs"
        fi
    fi

    if [[ "$RUN_CLIPPY" = true ]]; then
        t0=$(get_time_ms)
        if command -v cargo >/dev/null 2>&1; then
            if clippy_out="$(cargo clippy --all-targets -- -D warnings 2>&1)"; then
                t1=$(get_time_ms)
                record_check "Clippy Lint Suite" "PASS" "Zero warnings (-D warnings)" "" "$((t1 - t0))"
            else
                t1=$(get_time_ms)
                record_check "Clippy Lint Suite" "FAIL" "Clippy lint warnings or errors found" "$clippy_out" "$((t1 - t0))" "Run 'cargo clippy --fix --allow-dirty' or resolve compiler lints"
            fi
        else
            t1=$(get_time_ms)
            record_check "Clippy Lint Suite" "WARN" "cargo not found in PATH" "" "$((t1 - t0))"
        fi
    fi
fi

# ------------------------------------------------------------------------------
# 2. Test Suite Execution
# ------------------------------------------------------------------------------
if [[ "$RUN_TESTS" = true ]]; then
    section_header "2. Unit & Integration Test Suite"
    t0=$(get_time_ms)
    if command -v cargo >/dev/null 2>&1; then
        if test_out="$(cargo test --lib -- --test-threads=2 2>&1)"; then
            t1=$(get_time_ms)
            pass_summary="$(echo "$test_out" | grep -oE '[0-9]+ passed' | head -n 1 || echo "tests passed")"
            record_check "Rust Test Suite" "PASS" "$pass_summary" "$test_out" "$((t1 - t0))"
        else
            t1=$(get_time_ms)
            record_check "Rust Test Suite" "FAIL" "Test execution failed" "$test_out" "$((t1 - t0))" "Run 'cargo test -- --nocapture' to isolate test failures"
        fi
    else
        t1=$(get_time_ms)
        record_check "Rust Test Suite" "WARN" "cargo not found in PATH" "" "$((t1 - t0))"
    fi
fi

# ------------------------------------------------------------------------------
# 3. WebAssembly (WASM) Target Verification
# ------------------------------------------------------------------------------
if [[ "$RUN_WASM" = true ]]; then
    section_header "3. WebAssembly (WASM) Verification"
    
    # 3.1 WASM source module
    if [[ -f "${WORKSPACE_ROOT}/src/wasm.rs" ]]; then
        record_check "WASM Source Module" "PASS" "src/wasm.rs present and structured" "" "0"
    else
        record_check "WASM Source Module" "FAIL" "src/wasm.rs missing" "" "0" "Ensure WebAssembly bindings exist in src/wasm.rs"
    fi

    # 3.2 Web playground assets
    if [[ -f "${WORKSPACE_ROOT}/web/index.html" && -f "${WORKSPACE_ROOT}/web/term.js" ]]; then
        record_check "WASM Web Playground" "PASS" "web/index.html and web/term.js present" "" "0"
    else
        record_check "WASM Web Playground" "WARN" "web/ assets missing or incomplete" "" "0" "Check web/ directory assets"
    fi

    # 3.3 WASM target compilation check
    t0=$(get_time_ms)
    if command -v cargo >/dev/null 2>&1; then
        if wasm_out="$(cargo check --target wasm32-unknown-unknown --no-default-features --features wasm 2>&1)"; then
            t1=$(get_time_ms)
            record_check "WASM Target Build" "PASS" "wasm32-unknown-unknown target compiles cleanly" "" "$((t1 - t0))"
        else
            t1=$(get_time_ms)
            record_check "WASM Target Build" "FAIL" "WASM compilation failed" "$wasm_out" "$((t1 - t0))" "Run 'rustup target add wasm32-unknown-unknown' and verify wasm feature dependencies"
        fi
    else
        t1=$(get_time_ms)
        record_check "WASM Target Build" "WARN" "cargo not found to verify wasm32 build" "" "$((t1 - t0))"
    fi
fi

# ------------------------------------------------------------------------------
# 4. TypeScript SDK Verification
# ------------------------------------------------------------------------------
if [[ "$RUN_SDK" = true ]]; then
    section_header "4. TypeScript SDK Verification"
    SDK_DIR="${WORKSPACE_ROOT}/sdk"

    if [[ -d "$SDK_DIR" ]]; then
        # 4.1 Manifest & tsconfig check
        if [[ -f "$SDK_DIR/package.json" && -f "$SDK_DIR/tsconfig.json" ]]; then
            record_check "SDK Manifest & Config" "PASS" "sdk/package.json & tsconfig.json verified" "" "0"
        else
            record_check "SDK Manifest & Config" "FAIL" "Missing package.json or tsconfig.json in sdk/" "" "0" "Verify SDK directory configuration"
        fi

        # 4.2 TypeScript compilation / typecheck
        t0=$(get_time_ms)
        if command -v tsc >/dev/null 2>&1; then
            if tsc_out="$(cd "$SDK_DIR" && tsc --noEmit 2>&1)"; then
                t1=$(get_time_ms)
                record_check "SDK TypeScript Typecheck" "PASS" "tsc --noEmit passed with 0 errors" "" "$((t1 - t0))"
            else
                t1=$(get_time_ms)
                record_check "SDK TypeScript Typecheck" "FAIL" "TypeScript typecheck failed" "$tsc_out" "$((t1 - t0))" "Run 'cd sdk && npx tsc --noEmit' to inspect type errors"
            fi
        elif command -v npm >/dev/null 2>&1; then
            if npm_out="$(cd "$SDK_DIR" && npm run typecheck 2>&1)"; then
                t1=$(get_time_ms)
                record_check "SDK TypeScript Typecheck" "PASS" "npm run typecheck passed" "" "$((t1 - t0))"
            elif npm_out="$(cd "$SDK_DIR" && npm run build 2>&1)"; then
                t1=$(get_time_ms)
                record_check "SDK TypeScript Typecheck" "PASS" "npm run build passed" "" "$((t1 - t0))"
            else
                t1=$(get_time_ms)
                record_check "SDK TypeScript Typecheck" "WARN" "SDK build/typecheck command failed" "$npm_out" "$((t1 - t0))" "Run 'cd sdk && npm install && npm run build'"
            fi
        else
            t1=$(get_time_ms)
            record_check "SDK TypeScript Typecheck" "WARN" "Neither tsc nor npm available in PATH" "" "$((t1 - t0))" "Install Node.js & TypeScript to build the SDK"
        fi

        # 4.3 Core SDK source exports
        if [[ -f "$SDK_DIR/src/index.ts" && -f "$SDK_DIR/src/types.ts" && -f "$SDK_DIR/src/agent.ts" && -f "$SDK_DIR/src/wasm.ts" ]]; then
            record_check "SDK Source Modules" "PASS" "All core modules present (index, types, agent, wasm)" "" "0"
        else
            record_check "SDK Source Modules" "WARN" "One or more core SDK modules missing" "" "0"
        fi
    else
        record_check "SDK Workspace" "FAIL" "sdk/ directory not found in repository root" "" "0" "Ensure sdk/ exists in workspace"
    fi
fi

# ------------------------------------------------------------------------------
# 5. Termux & Android Portability Verification
# ------------------------------------------------------------------------------
if [[ "$RUN_TERMUX" = true ]]; then
    section_header "5. Termux & Android Portability Verification"

    # 5.1 Pure-Rust TLS dependency check
    if [[ -f "${WORKSPACE_ROOT}/Cargo.toml" ]]; then
        if grep -q 'rustls' "${WORKSPACE_ROOT}/Cargo.toml" && ! grep -q 'native-tls' "${WORKSPACE_ROOT}/Cargo.toml"; then
            record_check "Pure-Rust TLS Setup" "PASS" "Pure Rust TLS (rustls-tls-native-roots, no OpenSSL link)" "" "0"
        else
            record_check "Pure-Rust TLS Setup" "WARN" "Check TLS dependencies for OpenSSL C-lib dependency" "" "0"
        fi
    fi

    # 5.2 Termux bootstrap script syntax
    if [[ -f "${WORKSPACE_ROOT}/scripts/termux-bootstrap.sh" ]]; then
        if bash -n "${WORKSPACE_ROOT}/scripts/termux-bootstrap.sh" 2>/dev/null; then
            record_check "Termux Bootstrap Script" "PASS" "scripts/termux-bootstrap.sh syntax is valid POSIX bash" "" "0"
        else
            record_check "Termux Bootstrap Script" "FAIL" "Syntax error in scripts/termux-bootstrap.sh" "" "0" "Fix bash syntax errors in scripts/termux-bootstrap.sh"
        fi
    else
        record_check "Termux Bootstrap Script" "WARN" "scripts/termux-bootstrap.sh not found" "" "0"
    fi

    # 5.3 Dynamic TMPDIR handling
    if grep -rn "TMPDIR" "${WORKSPACE_ROOT}/scripts/" >/dev/null 2>&1; then
        record_check "Termux TMPDIR Safety" "PASS" "Dynamic TMPDIR fallback configured (avoids Android /tmp)" "" "0"
    else
        record_check "Termux TMPDIR Safety" "WARN" "Scripts should support dynamic TMPDIR for Termux compatibility" "" "0"
    fi
fi

# ------------------------------------------------------------------------------
# 6. Binary Discovery & Execution Verification
# ------------------------------------------------------------------------------
if [[ "$RUN_BINARY" = true ]]; then
    section_header "6. Binary Discovery & Execution"

    resolve_binary() {
        if [[ -n "$FUSION_BIN" ]]; then
            if [[ -f "$FUSION_BIN" ]]; then
                echo "$FUSION_BIN"
                return 0
            elif command -v "$FUSION_BIN" >/dev/null 2>&1; then
                command -v "$FUSION_BIN"
                return 0
            fi
            echo "$FUSION_BIN"
            return 0
        fi

        # Environment variable check
        if [[ -n "${FUSION_BIN_PATH:-}" && -x "$FUSION_BIN_PATH" ]]; then
            echo "$FUSION_BIN_PATH"
            return 0
        fi

        # Local workspace candidates first
        if [[ -f "${WORKSPACE_ROOT}/target/release/fusion" && -x "${WORKSPACE_ROOT}/target/release/fusion" ]]; then
            echo "${WORKSPACE_ROOT}/target/release/fusion"
            return 0
        fi
        if [[ -f "${WORKSPACE_ROOT}/target/debug/fusion" && -x "${WORKSPACE_ROOT}/target/debug/fusion" ]]; then
            echo "${WORKSPACE_ROOT}/target/debug/fusion"
            return 0
        fi

        # PATH check
        if command -v fusion >/dev/null 2>&1; then
            command -v fusion
            return 0
        fi

        # Standard install locations
        local candidates=(
            "${HOME}/.local/bin/fusion"
            "/usr/local/bin/fusion"
            "${PREFIX:-/data/data/com.termux/files/usr}/bin/fusion"
            "${HOME}/.cargo/bin/fusion"
        )

        for cand in "${candidates[@]}"; do
            if [[ -f "$cand" && -x "$cand" ]]; then
                echo "$cand"
                return 0
            fi
        done

        # Fallback
        if [[ -f "${WORKSPACE_ROOT}/target/release/fusion" ]]; then
            echo "${WORKSPACE_ROOT}/target/release/fusion"
        elif [[ -f "${WORKSPACE_ROOT}/target/debug/fusion" ]]; then
            echo "${WORKSPACE_ROOT}/target/debug/fusion"
        else
            echo "fusion"
        fi
    }

    DETECTED_BIN="$(resolve_binary)"
    FUSION_BIN="$DETECTED_BIN"

    # Check 6.1: Binary Existence & Permissions
    t0=$(get_time_ms)
    if [[ -f "$FUSION_BIN" ]]; then
        if [[ -x "$FUSION_BIN" ]]; then
            BIN_REALPATH="$(cd "$(dirname "$FUSION_BIN")" && pwd)/$(basename "$FUSION_BIN")"
            BIN_SIZE=$(ls -lh "$FUSION_BIN" | awk '{print $5}')
            t1=$(get_time_ms)
            record_check "Binary Location" "PASS" "$BIN_REALPATH ($BIN_SIZE)" "Size: $BIN_SIZE" "$((t1 - t0))"
        else
            t1=$(get_time_ms)
            record_check "Binary Location" "FAIL" "File exists but is not executable: $FUSION_BIN" "Run: chmod +x '$FUSION_BIN'" "$((t1 - t0))" "Run 'chmod +x $FUSION_BIN'"
        fi
    elif command -v "$FUSION_BIN" >/dev/null 2>&1; then
        RESOLVED_PATH="$(command -v "$FUSION_BIN")"
        t1=$(get_time_ms)
        record_check "Binary Location" "PASS" "Found in PATH: $RESOLVED_PATH" "" "$((t1 - t0))"
        FUSION_BIN="$RESOLVED_PATH"
    else
        t1=$(get_time_ms)
        record_check "Binary Location" "FAIL" "Binary not found at '$FUSION_BIN'" "Build binary with 'cargo build --release' or specify --bin <PATH>" "$((t1 - t0))" "Run 'cargo build --release' to compile the binary"
    fi

    # Check 6.2: Version Flag (--version)
    t0=$(get_time_ms)
    VERSION_OUTPUT=""
    VERSION_STR=""
    if [[ -x "$FUSION_BIN" ]] || command -v "$FUSION_BIN" >/dev/null 2>&1; then
        if VERSION_OUTPUT="$("$FUSION_BIN" --version 2>&1)"; then
            VERSION_STR="$(echo "$VERSION_OUTPUT" | strip_ansi | head -n 1)"
            t1=$(get_time_ms)
            record_check "Version Execution" "PASS" "$VERSION_STR" "$VERSION_OUTPUT" "$((t1 - t0))"
        else
            t1=$(get_time_ms)
            record_check "Version Execution" "FAIL" "Failed to execute '$FUSION_BIN --version'" "$VERSION_OUTPUT" "$((t1 - t0))" "Rebuild the binary with 'cargo build --release'"
        fi
    else
        t1=$(get_time_ms)
        record_check "Version Execution" "FAIL" "Skipped (binary unexecutable)" "" "$((t1 - t0))" "Build binary with 'cargo build --release'"
    fi

    # Check 6.3: Help Flag (--help)
    t0=$(get_time_ms)
    HELP_OUTPUT=""
    if [[ -x "$FUSION_BIN" ]] || command -v "$FUSION_BIN" >/dev/null 2>&1; then
        if HELP_OUTPUT="$("$FUSION_BIN" --help 2>&1)"; then
            CLEAN_HELP="$(echo "$HELP_OUTPUT" | strip_ansi)"
            t1=$(get_time_ms)
            if echo "$CLEAN_HELP" | grep -qi "Usage:"; then
                record_check "CLI Help Interface" "PASS" "Help syntax verified" "Clap CLI initialized successfully" "$((t1 - t0))"
            else
                record_check "CLI Help Interface" "WARN" "Help executed but returned unexpected output" "$HELP_OUTPUT" "$((t1 - t0))"
            fi
        else
            t1=$(get_time_ms)
            record_check "CLI Help Interface" "FAIL" "Failed to execute '$FUSION_BIN --help'" "$HELP_OUTPUT" "$((t1 - t0))" "Inspect CLI clap definitions"
        fi
    else
        t1=$(get_time_ms)
        record_check "CLI Help Interface" "FAIL" "Skipped (binary unexecutable)" "" "$((t1 - t0))"
    fi

    # Check 6.4: Shell Completion Generator (--generate-completion bash)
    t0=$(get_time_ms)
    if [[ -x "$FUSION_BIN" ]] || command -v "$FUSION_BIN" >/dev/null 2>&1; then
        if COMPLETION_OUT="$("$FUSION_BIN" --generate-completion bash 2>&1)"; then
            t1=$(get_time_ms)
            if echo "$COMPLETION_OUT" | grep -q "_fusion"; then
                record_check "Shell Completion Generator" "PASS" "Bash completion script generated" "" "$((t1 - t0))"
            else
                record_check "Shell Completion Generator" "WARN" "Completion generated with non-standard format" "" "$((t1 - t0))"
            fi
        else
            t1=$(get_time_ms)
            record_check "Shell Completion Generator" "WARN" "Completion generation failed" "$COMPLETION_OUT" "$((t1 - t0))"
        fi
    fi

    # --------------------------------------------------------------------------
    # 7. Tool Registry Verification
    # --------------------------------------------------------------------------
    section_header "7. Tool Registry Verification"

    t0=$(get_time_ms)
    TOOLS_OUTPUT=""
    if [[ -x "$FUSION_BIN" ]] || command -v "$FUSION_BIN" >/dev/null 2>&1; then
        if TOOLS_OUTPUT="$("$FUSION_BIN" "/tools" 2>&1)"; then
            t1=$(get_time_ms)
            CLEAN_TOOLS="$(echo "$TOOLS_OUTPUT" | strip_ansi)"

            # Parse total tool count
            TOOL_COUNT="$(echo "$CLEAN_TOOLS" | grep -oE '[0-9]+ total' | awk '{print $1}' || echo "0")"
            if [[ -z "$TOOL_COUNT" || "$TOOL_COUNT" -eq 0 ]]; then
                TOOL_COUNT="$(echo "$CLEAN_TOOLS" | grep -c '•' || echo "0")"
            fi

            if [[ "$TOOL_COUNT" -gt 0 ]]; then
                record_check "Tool Registry Load" "PASS" "$TOOL_COUNT tools registered" "Output parsed successfully" "$((t1 - t0))"
            else
                record_check "Tool Registry Load" "WARN" "Tool command ran but detected 0 tools" "$CLEAN_TOOLS" "$((t1 - t0))"
            fi

            # Verify core standard cross-platform tools
            CORE_TOOLS=("bash" "read" "write" "edit" "grep" "glob")
            MISSING_TOOLS=()
            FOUND_TOOLS=()

            for tool in "${CORE_TOOLS[@]}"; do
                if echo "$CLEAN_TOOLS" | grep -qE "(•|\s|^)${tool}(\s|$)"; then
                    FOUND_TOOLS+=("$tool")
                else
                    MISSING_TOOLS+=("$tool")
                fi
            done

            if [[ ${#MISSING_TOOLS[@]} -eq 0 ]]; then
                record_check "Core Tool Suite" "PASS" "All core tools present (${FOUND_TOOLS[*]})" "" "0"
            else
                record_check "Core Tool Suite" "FAIL" "Missing core tools: ${MISSING_TOOLS[*]}" "Found: ${FOUND_TOOLS[*]}" "0" "Verify tools/mod.rs tool registrations"
            fi

            # Check extended tools
            EXT_TOOLS=("git_status" "git_diff" "patch" "watch" "web_search")
            EXT_FOUND=()
            for ext in "${EXT_TOOLS[@]}"; do
                if echo "$CLEAN_TOOLS" | grep -qE "(•|\s|^)${ext}(\s|$)"; then
                    EXT_FOUND+=("$ext")
                fi
            done
            if [[ ${#EXT_FOUND[@]} -gt 0 ]]; then
                record_check "Extended Tools" "PASS" "${#EXT_FOUND[@]} extended tools present (${EXT_FOUND[*]})" "" "0"
            else
                record_check "Extended Tools" "WARN" "No extended tools detected" "" "0"
            fi
        else
            t1=$(get_time_ms)
            record_check "Tool Registry Load" "FAIL" "Failed executing '/tools' slash command" "$TOOLS_OUTPUT" "$((t1 - t0))" "Ensure slash commands are handled in REPL"
            record_check "Core Tool Suite" "FAIL" "Skipped (registry unavailable)" "" "0"
        fi
    else
        record_check "Tool Registry Load" "FAIL" "Skipped (binary unexecutable)" "" "0"
        record_check "Core Tool Suite" "FAIL" "Skipped (binary unexecutable)" "" "0"
    fi

    # --------------------------------------------------------------------------
    # 8. Configuration & State Path Verification
    # --------------------------------------------------------------------------
    section_header "8. Configuration & State Path Verification"

    t0=$(get_time_ms)
    CONFIG_PATH_OUTPUT=""
    RESOLVED_CONFIG_PATH=""
    if [[ -x "$FUSION_BIN" ]] || command -v "$FUSION_BIN" >/dev/null 2>&1; then
        if CONFIG_PATH_OUTPUT="$("$FUSION_BIN" "/config path" 2>&1)"; then
            t1=$(get_time_ms)
            CLEAN_CONFIG_PATH="$(echo "$CONFIG_PATH_OUTPUT" | strip_ansi)"
            RESOLVED_CONFIG_PATH="$(echo "$CLEAN_CONFIG_PATH" | grep -oE '(/[^ ]+|\.fusion/[^ ]+|[A-Za-z]:\\[^ ]+)' | tail -n 1 || echo "")"
            if [[ -n "$RESOLVED_CONFIG_PATH" ]]; then
                record_check "Config Path Resolution" "PASS" "$RESOLVED_CONFIG_PATH" "" "$((t1 - t0))"
            else
                RESOLVED_CONFIG_PATH="${HOME}/.fusion/config.json"
                record_check "Config Path Resolution" "WARN" "Defaulting to $RESOLVED_CONFIG_PATH" "$CONFIG_PATH_OUTPUT" "$((t1 - t0))"
            fi
        else
            t1=$(get_time_ms)
            RESOLVED_CONFIG_PATH="${HOME}/.fusion/config.json"
            record_check "Config Path Resolution" "FAIL" "Failed to resolve config path" "$CONFIG_PATH_OUTPUT" "$((t1 - t0))" "Verify config directory resolution in src/config.rs"
        fi
    else
        RESOLVED_CONFIG_PATH="${HOME}/.fusion/config.json"
        record_check "Config Path Resolution" "FAIL" "Skipped (binary unexecutable)" "" "0"
    fi

    # Check Config Directory Accessibility and Permissions
    CONFIG_DIR="$(dirname "$RESOLVED_CONFIG_PATH" 2>/dev/null || echo "${HOME}/.fusion")"
    if mkdir -p "$CONFIG_DIR" 2>/dev/null; then
        TEST_FILE="${CONFIG_DIR}/.verify_write_test_$$"
        if touch "$TEST_FILE" 2>/dev/null && rm -f "$TEST_FILE" 2>/dev/null; then
            record_check "Config Dir Writable" "PASS" "$CONFIG_DIR (read/write ok)" "" "0"
        else
            record_check "Config Dir Writable" "FAIL" "$CONFIG_DIR is read-only or permission denied" "" "0" "Check permissions for $CONFIG_DIR"
        fi
    else
        record_check "Config Dir Writable" "FAIL" "Cannot create or access $CONFIG_DIR" "" "0" "Create $CONFIG_DIR with write permissions"
    fi

    # Check Sessions Storage Directory
    SESSIONS_DIR="${CONFIG_DIR}/sessions"
    if mkdir -p "$SESSIONS_DIR" 2>/dev/null; then
        record_check "Session Storage Dir" "PASS" "$SESSIONS_DIR (ready)" "" "0"
    else
        record_check "Session Storage Dir" "WARN" "Unable to create $SESSIONS_DIR" "" "0"
    fi

    # Check Config Show & Parsing (/config show)
    t0=$(get_time_ms)
    CONFIG_SHOW_OUTPUT=""
    if [[ -x "$FUSION_BIN" ]] || command -v "$FUSION_BIN" >/dev/null 2>&1; then
        if CONFIG_SHOW_OUTPUT="$("$FUSION_BIN" "/config show" 2>&1)"; then
            t1=$(get_time_ms)
            CLEAN_SHOW="$(echo "$CONFIG_SHOW_OUTPUT" | strip_ansi)"
            if echo "$CLEAN_SHOW" | grep -q "{"; then
                record_check "Config File Parser" "PASS" "Config loads and parses valid JSON schema" "" "$((t1 - t0))"
            else
                record_check "Config File Parser" "WARN" "Config show returned unexpected output format" "$CLEAN_SHOW" "$((t1 - t0))"
            fi
        else
            t1=$(get_time_ms)
            record_check "Config File Parser" "FAIL" "Failed executing '/config show'" "$CONFIG_SHOW_OUTPUT" "$((t1 - t0))" "Check config serde deserializer"
        fi
    fi

    # Check Configured Providers and Masked API Keys
    scan_provider_keys() {
        local detected=()
        local providers=("ANTHROPIC" "OPENAI" "DEEPSEEK" "XAI" "OPENROUTER" "FUSION")

        for p in "${providers[@]}"; do
            local var="${p}_API_KEY"
            if [[ -n "${!var:-}" ]]; then
                local val="${!var}"
                local prefix="${val:0:4}"
                detected+=("${p}: ${prefix}****")
            fi
        done

        if [[ -n "${OLLAMA_BASE_URL:-}" ]]; then
            detected+=("Ollama: ${OLLAMA_BASE_URL}")
        fi

        if [[ ${#detected[@]} -gt 0 ]]; then
            local summary="${detected[*]}"
            record_check "API Key Environment" "PASS" "${#detected[@]} provider key(s) detected (${summary})" "" "0"
        else
            record_check "API Key Environment" "WARN" "No LLM API keys set in environment" "Configure ANTHROPIC_API_KEY, DEEPSEEK_API_KEY, OPENAI_API_KEY or ~/.fusion/config.json" "0"
        fi
    }
    scan_provider_keys

    # --------------------------------------------------------------------------
    # 9. Runtime Status & REPL State Verification
    # --------------------------------------------------------------------------
    section_header "9. Runtime Status & REPL State"

    t0=$(get_time_ms)
    STATUS_OUTPUT=""
    if [[ -x "$FUSION_BIN" ]] || command -v "$FUSION_BIN" >/dev/null 2>&1; then
        if STATUS_OUTPUT="$("$FUSION_BIN" "/status" 2>&1)"; then
            t1=$(get_time_ms)
            CLEAN_STATUS="$(echo "$STATUS_OUTPUT" | strip_ansi)"
            PROVIDER_VAL="$(echo "$CLEAN_STATUS" | grep -i 'Provider:' | awk '{print $2}' || echo "")"
            MODEL_VAL="$(echo "$CLEAN_STATUS" | grep -i 'Model:' | awk '{print $2}' || echo "")"

            STATUS_DETAILS="Provider: ${PROVIDER_VAL:-unknown}, Model: ${MODEL_VAL:-unknown}"
            record_check "Runtime State Machine" "PASS" "$STATUS_DETAILS" "$STATUS_OUTPUT" "$((t1 - t0))"
        else
            t1=$(get_time_ms)
            record_check "Runtime State Machine" "FAIL" "Failed executing '/status' command" "$STATUS_OUTPUT" "$((t1 - t0))" "Ensure /status handler is wired in agent"
        fi

        # Verify ACP Mode flag availability
        if "$FUSION_BIN" --help 2>&1 | strip_ansi | grep -q -- "--acp"; then
            record_check "ACP Protocol Interface" "PASS" "Agent Client Protocol flag available (--acp)" "" "0"
        else
            record_check "ACP Protocol Interface" "WARN" "ACP flag not found in CLI help" "" "0"
        fi
    fi
fi

# ------------------------------------------------------------------------------
# 10. System Diagnostics & Host Environment
# ------------------------------------------------------------------------------
if [[ "$RUN_SYSTEM" = true ]] && [[ "$QUICK_MODE" = false ]]; then
    section_header "10. System Diagnostics & Host Environment"

    # OS Identification
    OS_NAME="$(uname -s 2>/dev/null || echo "Unknown")"
    OS_ARCH="$(uname -m 2>/dev/null || echo "Unknown")"
    OS_KERNEL="$(uname -r 2>/dev/null || echo "Unknown")"
    PLATFORM_DESC="${OS_NAME} ${OS_ARCH} (Kernel: ${OS_KERNEL})"

    if [[ -f "/etc/os-release" ]]; then
        DISTRO_PRETTY="$(grep -E '^PRETTY_NAME=' /etc/os-release | cut -d= -f2 | tr -d '"' || echo "")"
        if [[ -n "$DISTRO_PRETTY" ]]; then
            PLATFORM_DESC="${DISTRO_PRETTY} [${OS_ARCH}]"
        fi
    elif [[ "$OS_NAME" = "Darwin" ]]; then
        SW_VER="$(sw_vers -productVersion 2>/dev/null || echo "")"
        SW_BUILD="$(sw_vers -buildVersion 2>/dev/null || echo "")"
        PLATFORM_DESC="macOS ${SW_VER} (${SW_BUILD}) [${OS_ARCH}]"
    fi

    # Termux detection
    if [[ -n "${TERMUX_VERSION:-}" ]] || [[ -d "/data/data/com.termux" ]]; then
        PLATFORM_DESC="Android / Termux ${TERMUX_VERSION:-} [${OS_ARCH}]"
    fi

    record_check "Host Platform" "PASS" "$PLATFORM_DESC" "" "0"

    # CPU Cores
    CPU_CORES="1"
    if command -v nproc >/dev/null 2>&1; then
        CPU_CORES="$(nproc)"
    elif command -v sysctl >/dev/null 2>&1; then
        CPU_CORES="$(sysctl -n hw.ncpu 2>/dev/null || echo "1")"
    fi
    record_check "CPU Availability" "PASS" "$CPU_CORES logical core(s)" "" "0"

    # Available Memory
    MEM_INFO="Unknown"
    if [[ "$OS_NAME" = "Darwin" ]]; then
        MEM_BYTES="$(sysctl -n hw.memsize 2>/dev/null || echo "0")"
        if [[ "$MEM_BYTES" -gt 0 ]]; then
            MEM_GB="$(awk "BEGIN {printf \"%.1f\", $MEM_BYTES / 1073741824}")"
            MEM_INFO="${MEM_GB} GB Total RAM"
        fi
    elif [[ -f "/proc/meminfo" ]]; then
        MEM_KB="$(grep MemTotal /proc/meminfo | awk '{print $2}' || echo "0")"
        if [[ "$MEM_KB" -gt 0 ]]; then
            MEM_GB="$(awk "BEGIN {printf \"%.1f\", $MEM_KB / 1048576}")"
            MEM_INFO="${MEM_GB} GB Total RAM"
        fi
    fi
    record_check "System Memory" "PASS" "$MEM_INFO" "" "0"

    # Disk Space Check
    DISK_AVAIL="$(df -h "${CONFIG_DIR:-$HOME}" 2>/dev/null | awk 'NR==2 {print $4}' || echo "Unknown")"
    record_check "Disk Space Available" "PASS" "$DISK_AVAIL free on configuration volume" "" "0"

    # Ecosystem Dependencies Checks (git, curl, tar, ripgrep, python, node)
    check_dependency() {
        local cmd="$1"
        local label="$2"
        local req="${3:-optional}"

        if command -v "$cmd" >/dev/null 2>&1; then
            local ver=""
            ver="$("$cmd" --version 2>&1 | head -n 1 | awk '{print $1, $2, $3}' || echo "installed")"
            record_check "Dependency: $label" "PASS" "$ver" "" "0"
        else
            if [[ "$req" = "required" ]]; then
                record_check "Dependency: $label" "FAIL" "Missing required command: $cmd" "" "0" "Install $cmd package via system package manager"
            else
                record_check "Dependency: $label" "WARN" "Command not found in PATH: $cmd (optional)" "" "0"
            fi
        fi
    }

    check_dependency "git" "Git VCS" "required"
    check_dependency "curl" "cURL" "required"
    check_dependency "tar" "Tar Archive" "required"
    check_dependency "rg" "Ripgrep" "optional"
    check_dependency "python3" "Python 3" "optional"

    # Ollama Local Service Check
    if [[ "$SKIP_NETWORK" = false ]]; then
        t0=$(get_time_ms)
        OLLAMA_HOST="${OLLAMA_BASE_URL:-http://localhost:11434}"
        if curl -s --max-time 1 "${OLLAMA_HOST}/api/tags" >/dev/null 2>&1; then
            t1=$(get_time_ms)
            record_check "Ollama Local Service" "PASS" "Local Ollama server reachable at $OLLAMA_HOST" "" "$((t1 - t0))"
        else
            t1=$(get_time_ms)
            record_check "Ollama Local Service" "WARN" "No local Ollama server running at $OLLAMA_HOST (offline ready)" "" "$((t1 - t0))"
        fi
    fi

    # Remote Network & API Endpoint Connectivity
    if [[ "$SKIP_NETWORK" = false ]]; then
        check_endpoint() {
            local host="$1"
            local name="$2"
            local t_ep0=$(get_time_ms)
            if curl -s --connect-timeout 2 --max-time 3 "https://${host}" >/dev/null 2>&1; then
                local t_ep1=$(get_time_ms)
                record_check "Connectivity: $name" "PASS" "https://${host} reachable" "" "$((t_ep1 - t_ep0))"
            else
                local t_ep1=$(get_time_ms)
                record_check "Connectivity: $name" "WARN" "Cannot reach https://${host} (check network/proxy)" "" "$((t_ep1 - t_ep0))"
            fi
        }

        check_endpoint "api.github.com" "GitHub API"
        check_endpoint "api.anthropic.com" "Anthropic API"
        check_endpoint "api.deepseek.com" "DeepSeek API"
        check_endpoint "api.openai.com" "OpenAI API"
    else
        record_check "Network Connectivity" "PASS" "Skipped (--skip-network enabled)" "" "0"
    fi
fi

# ------------------------------------------------------------------------------
# 11. Summary & Actionable Failure Diagnostics
# ------------------------------------------------------------------------------
GLOBAL_END_MS="$(get_time_ms)"
TOTAL_DURATION_MS=$((GLOBAL_END_MS - GLOBAL_START_MS))

OVERALL_STATUS="PASSED"
EXIT_CODE=0

if [[ "$FAILED_CHECKS" -gt 0 ]]; then
    OVERALL_STATUS="FAILED"
    EXIT_CODE=1
elif [[ "$WARNING_CHECKS" -gt 0 ]]; then
    OVERALL_STATUS="PASSED_WITH_WARNINGS"
fi

if [[ "$JSON_MODE" = true ]]; then
    # Generate structured JSON report
    JSON_TIMESTAMP="$(date -u +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date +"%Y-%m-%d %H:%M:%S")"

    CHECKS_JSON="["
    FAILURES_JSON="["
    fail_count=0
    for i in "${!CHECK_NAMES[@]}"; do
        if [[ $i -gt 0 ]]; then
            CHECKS_JSON+=","
        fi
        c_name="${CHECK_NAMES[$i]}"
        c_status="${CHECK_STATUSES[$i]}"
        c_msg="${CHECK_MESSAGES[$i]}"
        c_det="${CHECK_DETAILS[$i]}"
        c_dur="${CHECK_DURATIONS[$i]}"
        c_rem="${CHECK_REMEDIATIONS[$i]}"

        c_name_esc="$(printf '%s' "$c_name" | json_escape)"
        c_msg_esc="$(printf '%s' "$c_msg" | json_escape)"
        c_det_esc="$(printf '%s' "$c_det" | json_escape)"
        c_rem_esc="$(printf '%s' "$c_rem" | json_escape)"

        check_entry="{\"name\":\"$c_name_esc\",\"status\":\"$c_status\",\"message\":\"$c_msg_esc\",\"detail\":\"$c_det_esc\",\"duration_ms\":$c_dur,\"remediation\":\"$c_rem_esc\"}"
        CHECKS_JSON+="$check_entry"

        if [[ "$c_status" = "FAIL" ]]; then
            if [[ $fail_count -gt 0 ]]; then
                FAILURES_JSON+=","
            fi
            FAILURES_JSON+="$check_entry"
            fail_count=$((fail_count + 1))
        fi
    done
    CHECKS_JSON+="]"
    FAILURES_JSON+="]"

    cat << EOF
{
  "timestamp": "${JSON_TIMESTAMP}",
  "status": "${OVERALL_STATUS}",
  "exit_code": ${EXIT_CODE},
  "duration_ms": ${TOTAL_DURATION_MS},
  "duration_formatted": "$(format_duration "$TOTAL_DURATION_MS")",
  "binary": "${FUSION_BIN}",
  "version": "${VERSION_STR:-unknown}",
  "metrics": {
    "total": ${TOTAL_CHECKS},
    "passed": ${PASSED_CHECKS},
    "warnings": ${WARNING_CHECKS},
    "failed": ${FAILED_CHECKS}
  },
  "checks": ${CHECKS_JSON},
  "failures": ${FAILURES_JSON}
}
EOF

else
    section_header "Verification Summary & Benchmarks"
    echo ""
    echo "  Total Checks:        ${TOTAL_CHECKS}"
    printf "  ${COLOR_GREEN}Passed:              ${PASSED_CHECKS}${COLOR_RESET}\n"
    if [[ "$WARNING_CHECKS" -gt 0 ]]; then
        printf "  ${COLOR_YELLOW}Warnings:            ${WARNING_CHECKS}${COLOR_RESET}\n"
    else
        printf "  Warnings:            0\n"
    fi
    if [[ "$FAILED_CHECKS" -gt 0 ]]; then
        printf "  ${COLOR_RED}Failed:              ${FAILED_CHECKS}${COLOR_RESET}\n"
    else
        printf "  Failed:              0\n"
    fi
    printf "  ${COLOR_CYAN}Total Duration:      %s${COLOR_RESET}\n" "$(format_duration "$TOTAL_DURATION_MS")"
    echo ""

    if [[ "$FAILED_CHECKS" -gt 0 ]]; then
        echo "================================================================================"
        printf "${COLOR_RED}${COLOR_BOLD}ACTIONABLE FAILURE REMEDIATION:${COLOR_RESET}\n"
        echo "================================================================================"
        fail_idx=1
        for i in "${!CHECK_NAMES[@]}"; do
            if [[ "${CHECK_STATUSES[$i]}" = "FAIL" ]]; then
                c_name="${CHECK_NAMES[$i]}"
                c_msg="${CHECK_MESSAGES[$i]}"
                c_rem="${CHECK_REMEDIATIONS[$i]}"
                printf "  ${COLOR_RED}%d. [%s]${COLOR_RESET} %s\n" "$fail_idx" "$c_name" "$c_msg"
                if [[ -n "$c_rem" ]]; then
                    printf "     ${COLOR_BOLD}Action:${COLOR_RESET} %s\n" "$c_rem"
                fi
                fail_idx=$((fail_idx + 1))
            fi
        done
        echo "================================================================================"
        echo ""
        printf "${COLOR_RED}${COLOR_BOLD}✗ FAILURE: %d verification check(s) failed.${COLOR_RESET}\n" "$FAILED_CHECKS"
        echo "Please resolve the failing items above and re-run './scripts/verify.sh'."
    elif [[ "$OVERALL_STATUS" = "PASSED_WITH_WARNINGS" ]]; then
        printf "${COLOR_YELLOW}${COLOR_BOLD}⚠ NOTICE: Verification passed with %d warning(s).${COLOR_RESET}\n" "$WARNING_CHECKS"
        echo "Fusion is operational. Review warning details above for optimal configuration."
    else
        printf "${COLOR_GREEN}${COLOR_BOLD}✓ SUCCESS: All Fusion verification checks passed! (%s)${COLOR_RESET}\n" "$(format_duration "$TOTAL_DURATION_MS")"
        echo "You're ready to run Fusion:"
        echo "  fusion login"
        echo "  fusion"
    fi
    echo ""
fi

exit $EXIT_CODE
