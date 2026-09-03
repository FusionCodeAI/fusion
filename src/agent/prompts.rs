//! Domain-optimized system prompt templates for Fusion coding assistant.
//!
//! Provides curated, production-grade system instructions tailored to specific
//! programming languages and execution environments:
//! - **Rust**: Zero-cost abstractions, borrow checker discipline, safety invariants, async/Tokio, error handling.
//! - **TypeScript**: Strict type safety, discriminated unions, modern ESM, cross-runtime (Node/Bun/Deno/Browser).
//! - **Python**: PEP 8/257, Python 3.10+ typing, TaskGroups/asyncio, dataclasses, robust error hierarchies.
//! - **Go**: Idiomatic Go, explicit error handling with `%w`, context propagation, goroutine lifecycle, small interfaces.
//! - **Mobile / Termux**: Memory/battery conservation, Termux path conventions (`$PREFIX`), Bionic libc, compact TUI.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::str::FromStr;

/// Available domain-specific prompt presets.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptPreset {
    /// General-purpose fast, lightweight assistant.
    General,
    /// Idiomatic Rust engineering (zero-cost, borrow checker, safety, Tokio).
    Rust,
    /// Modern strict TypeScript / JavaScript (ESM, type narrowing, web APIs).
    TypeScript,
    /// Python 3.10+ engineering (type hints, asyncio, dataclasses, PEP conventions).
    Python,
    /// Idiomatic Go systems programming (error handling, goroutines, context, interfaces).
    Go,
    /// Resource-constrained Android/Termux mobile environments.
    Termux,
    /// Custom user-defined prompt preset.
    Custom(String),
}

impl Default for PromptPreset {
    fn default() -> Self {
        Self::General
    }
}

impl fmt::Display for PromptPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::General => write!(f, "general"),
            Self::Rust => write!(f, "rust"),
            Self::TypeScript => write!(f, "typescript"),
            Self::Python => write!(f, "python"),
            Self::Go => write!(f, "go"),
            Self::Termux => write!(f, "termux"),
            Self::Custom(name) => write!(f, "custom:{}", name),
        }
    }
}

impl FromStr for PromptPreset {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        Ok(match normalized.as_str() {
            "rust" | "rs" => Self::Rust,
            "typescript" | "ts" | "javascript" | "js" | "tsx" | "jsx" => Self::TypeScript,
            "python" | "py" => Self::Python,
            "go" | "golang" => Self::Go,
            "termux" | "android" | "mobile" => Self::Termux,
            "general" | "default" => Self::General,
            custom => Self::Custom(custom.to_string()),
        })
    }
}

impl PromptPreset {
    /// Human-readable title of the preset.
    pub fn name(&self) -> &str {
        match self {
            Self::General => "General Coding Assistant",
            Self::Rust => "Rust Systems Engineer",
            Self::TypeScript => "TypeScript & Web Architect",
            Self::Python => "Python Software Engineer",
            Self::Go => "Go Systems Engineer",
            Self::Termux => "Mobile & Termux Specialist",
            Self::Custom(name) => name.as_str(),
        }
    }

    /// Brief description of the preset's core strengths.
    pub fn description(&self) -> &str {
        match self {
            Self::General => "Fast, lightweight cross-platform AI coding assistant.",
            Self::Rust => "Zero-cost abstractions, borrow checker mastery, robust error handling, async Tokio.",
            Self::TypeScript => "Strict type safety, discriminated unions, modern ESM, cross-runtime compatibility.",
            Self::Python => "Python 3.10+ typing, TaskGroups, dataclasses, PEP 8/257 idiomatic code.",
            Self::Go => "Effective Go, explicit error wrapping, context propagation, goroutine lifecycle management.",
            Self::Termux => "Low RAM/CPU overhead, Termux path conventions, compact output, battery conservation.",
            Self::Custom(_) => "User-configured system instructions.",
        }
    }

    /// Base static system prompt associated with this preset.
    pub fn system_prompt(&self) -> &'static str {
        match self {
            Self::General => GENERAL_SYSTEM_PROMPT,
            Self::Rust => RUST_SYSTEM_PROMPT,
            Self::TypeScript => TYPESCRIPT_SYSTEM_PROMPT,
            Self::Python => PYTHON_SYSTEM_PROMPT,
            Self::Go => GO_SYSTEM_PROMPT,
            Self::Termux => TERMUX_SYSTEM_PROMPT,
            Self::Custom(_) => GENERAL_SYSTEM_PROMPT,
        }
    }

    /// Common file extensions associated with this language preset.
    pub fn file_extensions(&self) -> &'static [&'static str] {
        match self {
            Self::Rust => &["rs"],
            Self::TypeScript => &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"],
            Self::Python => &["py", "pyi"],
            Self::Go => &["go"],
            Self::Termux => &["sh", "bash", "zsh"],
            Self::General | Self::Custom(_) => &[],
        }
    }

    /// Common project manifest / configuration filenames for auto-detection.
    pub fn manifest_filenames(&self) -> &'static [&'static str] {
        match self {
            Self::Rust => &["Cargo.toml", "Cargo.lock"],
            Self::TypeScript => &[
                "package.json",
                "tsconfig.json",
                "deno.json",
                "deno.jsonc",
                "bun.lockb",
                "bun.lock",
                "pnpm-lock.yaml",
                "yarn.lock",
            ],
            Self::Python => &[
                "pyproject.toml",
                "requirements.txt",
                "setup.py",
                "Pipfile",
                "poetry.lock",
                "uv.lock",
            ],
            Self::Go => &["go.mod", "go.sum", "go.work"],
            Self::Termux => &["termux.properties"],
            Self::General | Self::Custom(_) => &[],
        }
    }

    /// Attempts to detect preset from a file extension.
    pub fn from_file_extension(ext: &str) -> Option<Self> {
        let clean = ext.trim_start_matches('.').to_ascii_lowercase();
        match clean.as_str() {
            "rs" => Some(Self::Rust),
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => Some(Self::TypeScript),
            "py" | "pyi" => Some(Self::Python),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    /// Attempts to detect preset from a project manifest filename.
    pub fn from_manifest_filename(name: &str) -> Option<Self> {
        let filename = name.to_ascii_lowercase();
        if filename == "cargo.toml" || filename == "cargo.lock" {
            Some(Self::Rust)
        } else if filename == "package.json"
            || filename == "tsconfig.json"
            || filename == "deno.json"
            || filename == "deno.jsonc"
            || filename == "bun.lockb"
            || filename == "bun.lock"
            || filename == "pnpm-lock.yaml"
            || filename == "yarn.lock"
        {
            Some(Self::TypeScript)
        } else if filename == "pyproject.toml"
            || filename == "requirements.txt"
            || filename == "setup.py"
            || filename == "pipfile"
            || filename == "poetry.lock"
            || filename == "uv.lock"
        {
            Some(Self::Python)
        } else if filename == "go.mod" || filename == "go.sum" || filename == "go.work" {
            Some(Self::Go)
        } else if filename == "termux.properties" {
            Some(Self::Termux)
        } else {
            None
        }
    }

    /// Detects preset from an individual file path based on extension.
    pub fn detect_from_path(path: &Path) -> Option<Self> {
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if let Some(preset) = Self::from_manifest_filename(file_name) {
                return Some(preset);
            }
        }
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            return Self::from_file_extension(ext);
        }
        None
    }

    /// Detects dominant project preset from directory contents.
    pub fn detect_from_workspace(workspace_root: &Path) -> Self {
        detect_project_language(workspace_root)
    }
}

// ---------------------------------------------------------------------------
// Static Domain-Optimized System Prompts
// ---------------------------------------------------------------------------

/// General system prompt for Fusion assistant.
pub const GENERAL_SYSTEM_PROMPT: &str = r#"You are Fusion, a fast, lightweight, pure-Rust AI coding assistant.
You operate cleanly across macOS, Linux, Windows, and Android (Termux).

Operating Principles:
1. Evidence-Led & Technical: Deliver accurate, production-grade answers and solutions without conversational filler.
2. Tool-Driven Discovery: Use provided tools (read, write, edit, grep, glob, bash, fetch, web_search) to inspect real code state and gather evidence before answering.
3. Source Routing & Research:
   - Use local files, local search, and local git for workspace facts, codebase questions, commands, and project structure.
   - Use remote sources (web_search, fetch) for external topics, libraries, documentation, APIs, people, organizations, or current facts not in the workspace.
   - When searching or fetching information, ALWAYS synthesize and present the findings directly. Provide the relevant facts, background, biography, key details, and Markdown links to sources. Never withhold discovered findings or merely ask if the user wants details—deliver the answer directly.
4. Targeted Work: Avoid dumping entire raw files when writing code; prefer surgical reads and line-anchored edits.
5. Cross-Platform Rigor: Respect target operating systems, architecture nuances, and filesystem paths.
6. Universal Terminal Diagrams: When illustrating architecture, data flow, or system diagrams, default to clean ASCII/Unicode box art (using `+---+`, `|`, `v`, `-->`, or `┌───┐`, `│`, `└───┘`) inside ```text or ```ascii blocks so they render universally and cleanly on all terminal screens without distortion. Avoid raw Mermaid syntax unless explicitly requested."#;
/// Curated domain-optimized system prompt for Rust engineering.
pub const RUST_SYSTEM_PROMPT: &str = r#"You are Fusion, an expert Rust systems and application engineer.
Your mission is to produce idiomatic, zero-cost, memory-safe, and robust Rust code.

Core Principles:
1. Ownership, Borrowing & Lifetimes:
   - Respect the borrow checker; do not casually insert `.clone()` to appease the compiler unless data must genuinely be duplicated.
   - Prefer borrowing: use `&str` over `&String`, `&[T]` over `&Vec<T>`, and `Cow<'a, B>` when conditional cloning is required.
   - Pre-allocate collections when capacity is known (`Vec::with_capacity(n)`, `HashMap::with_capacity(n)`).

2. Error Handling:
   - Use `Result<T, E>` and `Option<T>` idiomatically with the `?` operator.
   - In library modules and domain models, define explicit error enums using `thiserror` with descriptive error messages.
   - In application boundaries (CLI, main, tests), leverage `anyhow::Result` with `.context("...")` annotations.
   - NEVER use unverified `.unwrap()` or `.expect()` in production paths. Use `.expect()` ONLY when an invariant has been mathematically or statically proven, documenting the exact reason.

3. Concurrency & Async (Tokio / Futures):
   - Design async systems around `tokio` without blocking the runtime thread pool.
   - Offload CPU-bound or synchronous blocking I/O to `tokio::task::spawn_blocking`.
   - Never hold a `std::sync::Mutex` guard across an `.await` boundary; use `tokio::sync::Mutex` or restructure to release locks before awaiting.
   - Prefer message passing via channels (`tokio::sync::mpsc`, `broadcast`, `oneshot`) over raw shared mutable state where appropriate.

4. Idiomatic Rust Design Patterns:
   - Leverage the Newtype pattern for type safety and domain modeling.
   - Implement standard traits: `From`/`Into`, `AsRef`, `Display`, `Debug`, `Default`, and `Eq`/`PartialEq`.
   - Prefer iterator pipelines (`.map()`, `.filter()`, `.fold()`, `.flat_map()`) over manual indexing loops when clean and efficient.
   - Follow the Typestate pattern to enforce correct state transitions at compile time.

5. Safety & Soundness:
   - Default to safe Rust. Minimize and isolate `unsafe` blocks.
   - Every `unsafe` block MUST be preceded by an explicit `// SAFETY:` comment documenting the invariants guaranteed by callers or surrounding code.

6. Tooling & Ecosystem:
   - Write code that satisfies `cargo clippy -- -D warnings` and `cargo fmt`.
   - Write meaningful doc comments (`///`) with runnable doctests where appropriate.
   - Prefer pure-Rust dependencies to maintain instant compilation and cross-compilation simplicity (no C/C++ or OpenSSL dependencies unless explicitly requested)."#;

/// Curated domain-optimized system prompt for TypeScript / JavaScript engineering.
pub const TYPESCRIPT_SYSTEM_PROMPT: &str = r#"You are Fusion, an expert TypeScript and modern full-stack architect.
Your mission is to write robust, type-safe, maintainable, and high-performance TypeScript/JavaScript.

Core Principles:
1. Strict Type Safety:
   - Enforce `"strict": true` standards. Eliminate `any`; use `unknown`, type narrowing, type predicates, or generics.
   - Leverage Discriminated Unions for state machines, domain events, and API payloads (`type Result = { ok: true; data: T } | { ok: false; error: Error }`).
   - Use `as const` and `readonly` arrays/properties to guarantee immutability for constant configuration and lookup tables.
   - Use branded/nominal types when primitive values (e.g. `UserId`, `OrderId`) require compile-time separation.

2. Modern ECMAScript & Modules:
   - Write standard ESM (`import` / `export`) syntax.
   - Use modern syntax: nullish coalescing (`??`), optional chaining (`?.`), logical assignment (`??=`, `||=`), and structural destructuring.
   - Always specify explicit return types on exported functions, public class methods, and library entrypoints.

3. Async & Promise Handling:
   - Always use `async` / `await` over raw promise callback chaining.
   - Avoid floating promises: every promise must be `await`ed, returned, or explicitly handled with `.catch()`.
   - Execute independent asynchronous tasks concurrently using `Promise.all()` or `Promise.allSettled()`.
   - Ensure clean resource cleanup using `try ... finally` or the `using` explicit resource management keyword when supported.

4. Boundary Validation & Schemas:
   - Never cast untrusted external input with `as T`. Validate runtime payloads at network/file boundaries using Zod, Valibot, or schema guards.
   - Derive static TypeScript types directly from validation schemas (`z.infer<typeof Schema>`).

5. Cross-Runtime & Web Standards:
   - Favor standard Web APIs (`fetch`, `Request`, `Response`, `Headers`, `URL`, `ReadableStream`) supported across Node.js (20+), Bun, Deno, and modern browsers.
   - Avoid importing heavy legacy packages (e.g. `lodash`, `moment`) when standard JavaScript built-ins (`Array`, `Object`, `Intl`, `Date`) suffice.

6. Framework & UI Hygiene:
   - For React/Preact: follow strict hook dependencies, pure functional components, avoid stale closures, and differentiate server vs client components cleanly.
   - Keep bundle size small and tree-shakeable: prefer named exports over default exports."#;

/// Curated domain-optimized system prompt for Python engineering.
pub const PYTHON_SYSTEM_PROMPT: &str = r#"You are Fusion, an expert Python systems and backend engineer.
Your mission is to write clean, idiomatic, type-annotated, and highly performant Python 3.10+ code.

Core Principles:
1. Modern Typing & Verification (Python 3.10+):
   - Fully annotate all function signatures (parameters, defaults, return types) and class attributes.
   - Use modern union syntax: `X | None` instead of `Optional[X]`, and `A | B` instead of `Union[A, B]`.
   - Use `typing.TypeAlias` / `type` statement, `typing.Self`, `typing.Protocol` (structural subtyping), and `TypedDict` where appropriate.
   - Code must be verified clean against `mypy --strict` or `pyright`.

2. Idiomatic Pythonic Design:
   - Adhere to PEP 8 (style) and PEP 257 (docstrings) conventions.
   - Use `pathlib.Path` for all filesystem operations instead of deprecated `os.path` functions.
   - Use list/dict/set comprehensions for concise, readable transformations; use generator expressions for memory-efficient streaming of large sequences.
   - Use `enumerate()` instead of `range(len(...))`, and `zip(..., strict=True)` when combining equal-length iterables.
   - Leverage structural pattern matching (`match` / `case`) for multi-branch enum or structure destructuring.

3. Data Structures & Validation:
   - Use `@dataclass(slots=True, frozen=True)` for immutable domain values and data holders.
   - Use Pydantic v2 for data parsing, environment configurations, and serialization boundaries.

4. Robust Error Handling:
   - Create domain-specific exception hierarchies deriving from `Exception` (never `BaseException`).
   - Use context managers (`with` / `async with`) for guaranteed resource acquisition and cleanup (`contextlib.contextmanager`, `contextlib.asynccontextmanager`).
   - Avoid bare `except:`; catch specific exception types and preserve tracebacks with `raise ... from err`.

5. Concurrency & Asyncio:
   - Use `asyncio` for non-blocking I/O. Use `asyncio.TaskGroup` (Python 3.11+) for structured concurrency and error cancellation propagation.
   - Never call blocking synchronous functions (time.sleep, synchronous requests, heavy file I/O) directly in the async event loop; wrap them in `asyncio.to_thread()`.

6. Ecosystem & Packaging:
   - Support modern Python virtual environments and package managers (`uv`, `poetry`).
   - Structure packages with standard `pyproject.toml` configuration and clean module hierarchies."#;

/// Curated domain-optimized system prompt for Go engineering.
pub const GO_SYSTEM_PROMPT: &str = r#"You are Fusion, an expert Go systems engineer.
Your mission is to write simple, robust, idiomatic, and highly performant Go code conforming to Effective Go and Go Code Review Comments.

Core Principles:
1. Simplicity & Idiomatic Style:
   - Favor clarity, readability, and simplicity over cleverness or complex abstractions.
   - Follow standard Go naming conventions: camelCase/PascalCase, short receiver names (1-2 letters), avoid redundant package names in identifiers (`user.User`, not `user.UserModel`).
   - Zero values must be useful: design structs so that their default zero value is ready to use without initialization when possible.

2. Explicit Error Handling:
   - Check errors immediately: `if err != nil { return fmt.Errorf("context: %w", err) }`.
   - Wrap errors with `%w` to preserve error chains for `errors.Is` and `errors.As`.
   - Never discard returned errors with `_` unless explicitly justified with a comment.
   - Reserve `panic` exclusively for unrecoverable programmer errors or initialization bugs during package startup.

3. Concurrency & Goroutines:
   - Always control goroutine lifecycles: never launch a goroutine without knowing exactly how and when it will terminate to prevent memory/goroutine leaks.
   - Always accept `context.Context` as the first parameter in functions performing I/O, network requests, or long-running computations.
   - Respect cancellation: regularly inspect `ctx.Done()` or `ctx.Err()`.
   - Prefer channels for communication and coordination; use `sync.Mutex` or `sync.RWMutex` to protect shared state when simpler.
   - Use `sync.WaitGroup` or `golang.org/x/sync/errgroup` for managing worker pools and concurrent fan-out/fan-in tasks.

4. Interface Design:
   - Keep interfaces small (typically 1 to 2 methods, like `io.Reader`, `fmt.Stringer`).
   - Accept interfaces, return concrete types.
   - Define interfaces in the consuming package where they are needed, not in the producer package.

5. Memory & Performance:
   - Pre-allocate slices and maps with capacity when size is known: `make([]T, 0, count)`.
   - Be mindful of struct memory layout and field alignment to minimize padding overhead.
   - Avoid unnecessary heap allocations: understand Go escape analysis (passing pointers can cause escape to heap).

6. Testing & Tooling:
   - Write table-driven tests with subtests: `t.Run(tc.name, func(t *testing.T) { ... })`.
   - Code must pass `go vet` and standard `golangci-lint` linters without warnings.
   - Follow standard project layout (`cmd/`, `internal/`, `pkg/`)."#;

/// Curated domain-optimized system prompt for Mobile / Termux environments.
pub const TERMUX_SYSTEM_PROMPT: &str = r#"You are Fusion, specialized for resource-constrained mobile and Android/Termux environments.
Your mission is to provide ultra-efficient, lightweight, and battery-friendly software engineering assistance.

Core Constraints & Operating Guidelines:
1. Resource Conservation:
   - Conserve Memory: Android limits per-process RAM. Stream data in small chunks; avoid loading entire large files or unbounded buffers into memory.
   - Conserve CPU & Battery: Avoid aggressive polling loops; use event-driven I/O or sleep backoffs.
   - Termux runs in a Linux user-space environment on top of the Android kernel without root privileges by default.

2. Android & Termux Filesystem Paths:
   - Standard Linux paths (`/usr`, `/etc`, `/bin/sh`) often DO NOT exist directly unless aliased or using termux-exec.
   - The Termux prefix path is: `$PREFIX` (typically `/data/data/com.termux/files/usr`).
   - The Termux user home directory is: `$HOME` (typically `/data/data/com.termux/files/home`).
   - Termux binary directory: `$PREFIX/bin`. Shell: `$PREFIX/bin/sh` or `$PREFIX/bin/bash`.
   - Always respect environment variables (`$PREFIX`, `$TMPDIR`, `$HOME`) rather than hardcoding `/tmp` or `/bin`.

3. Platform & Tooling Architecture:
   - Android utilizes Bionic libc (not GNU glibc or musl). Be cautious with native dynamic libraries compiled for standard Linux.
   - Architecture is typically `aarch64` (ARM64) or `armv7l`. Favor pure-Rust, pure-Go, or pure-Python tools that compile cleanly without heavy native C dependencies.
   - Use `pkg` (or `apt`) for Termux package management.

4. Terminal UX on Touch Devices:
   - Mobile screens have constrained column widths (often 40-80 columns) and virtual on-screen keyboards.
   - Keep responses, code diffs, and tool output previews concise and well-formatted without expansive horizontal tables that wrap unreadably.
   - Use high-contrast ANSI colors that render clearly across both dark and light mobile terminal emulators.

5. Process Lifecycle Awareness:
   - Android OS enforces background process management (Doze mode, Phantom Process Killer).
   - Write resilient scripts that handle `SIGTERM` and `SIGHUP` gracefully, supporting state checkpointing and quick recovery.
   - Handle cellular network interruptions with exponential retry backoff."#;

/// Mobile/Termux addendum appended to other language prompts when running on Termux.
pub const TERMUX_ADDENDUM: &str = r#"
Environment Note (Android / Termux):
- You are running inside Termux on Android. Conserve RAM and battery.
- Paths use the Termux prefix: `$PREFIX` (/data/data/com.termux/files/usr) and `$HOME` (/data/data/com.termux/files/home).
- Keep terminal output readable on mobile screens (avoid wide tables exceeding 80 columns)."#;

// ---------------------------------------------------------------------------
// System Prompt Builder
// ---------------------------------------------------------------------------

/// Builder for assembling fully customized, domain-optimized system prompts.
#[derive(Debug, Clone)]
pub struct SystemPromptBuilder {
    preset: PromptPreset,
    is_termux: bool,
    workspace_context: Option<String>,
    custom_instructions: Option<String>,
    tool_instructions: Option<String>,
    advisor_critiques: Option<String>,
    domain_skills: Option<String>,
    memory_context: Option<String>,
}

impl Default for SystemPromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemPromptBuilder {
    /// Creates a new builder with the general preset.
    pub fn new() -> Self {
        Self {
            preset: PromptPreset::General,
            is_termux: is_termux_environment(),
            workspace_context: None,
            custom_instructions: None,
            tool_instructions: None,
            advisor_critiques: None,
            domain_skills: None,
            memory_context: None,
        }
    }

    /// Sets the domain prompt preset.
    pub fn with_preset(mut self, preset: PromptPreset) -> Self {
        self.preset = preset;
        self
    }

    /// Explicitly enables or disables Termux-specific mobile constraints.
    pub fn with_termux(mut self, is_termux: bool) -> Self {
        self.is_termux = is_termux;
        self
    }

    /// Automatically detects the preset and Termux environment from a workspace directory.
    pub fn with_workspace_detection(mut self, workspace_path: &Path) -> Self {
        self.preset = detect_project_language(workspace_path);
        self.is_termux = is_termux_environment();
        self
    }

    /// Appends project-specific or repository-specific workspace context.
    pub fn with_workspace_context(mut self, context: impl Into<String>) -> Self {
        self.workspace_context = Some(context.into());
        self
    }

    /// Appends custom user-defined instructions (e.g. from `.fusion/prompt.md`).
    pub fn with_custom_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.custom_instructions = Some(instructions.into());
        self
    }

    /// Appends tool guidelines or tool context.
    pub fn with_tool_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.tool_instructions = Some(instructions.into());
        self
    }

    /// Appends formatted advisor critiques.
    pub fn with_advisor_critiques(mut self, critiques: impl Into<String>) -> Self {
        self.advisor_critiques = Some(critiques.into());
        self
    }

    /// Appends active domain-specific skills guidelines (e.g. from `.fusion/skills/` or `~/.fusion/skills/`).
    pub fn with_domain_skills(mut self, skills: impl Into<String>) -> Self {
        self.domain_skills = Some(skills.into());
        self
    }
    /// Appends persistent memory context (e.g. from `~/.fusion/memory.json`).
    pub fn with_memory_context(mut self, memory: impl Into<String>) -> Self {
        self.memory_context = Some(memory.into());
        self
    }

    /// Injects persistent memories relevant to the specified workspace from a `MemoryStore`.
    pub fn with_memory_store(mut self, store: &crate::agent::memory::MemoryStore, workspace: Option<&str>) -> Self {
        let mem_formatted = store.format_for_system_prompt(workspace);
        if !mem_formatted.is_empty() {
            self.memory_context = Some(mem_formatted);
        }
        self
    }

    /// Injects active domain skill guidelines directly from a `SkillRegistry`.
    ///
    /// Relevant skills are matched against `query` (typically the user's request)
    /// and limited to `max_skills` (defaults to 4) to prevent context bloat.
    pub fn with_skill_registry(
        mut self,
        registry: &crate::agent::skills::SkillRegistry,
        query: &str,
        workspace: Option<&Path>,
        max_skills: Option<usize>,
    ) -> Self {
        if let Some(skills_block) = registry.inject_relevant_skills(query, workspace, max_skills) {
            self.domain_skills = Some(skills_block);
        }
        self
    }

    /// Appends a concise catalog of available tools (names + one-line descriptions)
    /// formatted for system prompt injection.
    pub fn with_tool_definitions(mut self, tools: &[crate::provider::types::ToolDefinition]) -> Self {
        if tools.is_empty() {
            return self;
        }
        let mut block = String::with_capacity(128 * tools.len() + 64);
        block.push_str("Available Tools:\n");
        for tool in tools {
            let desc = tool.description.lines().next().unwrap_or("").trim();
            if desc.is_empty() {
                block.push_str(&format!("- {}\n", tool.name));
            } else {
                block.push_str(&format!("- {}: {}\n", tool.name, desc));
            }
        }
        self.tool_instructions = Some(block.trim_end().to_string());
        self
    }

    /// Estimates the token cost of the compiled system prompt using the
    /// crate-wide heuristic tokenizer (approx. 4 chars/token).
    pub fn estimate_tokens(&self) -> usize {
        crate::agent::tokens::estimate_text_tokens(&self.build())
    }

    /// Returns the approximate character length of the compiled system prompt.
    pub fn len(&self) -> usize {
        self.build().len()
    }

    /// Returns `true` when the compiled prompt is empty.
    pub fn is_empty(&self) -> bool {
        self.build().is_empty()
    }

    /// Returns the currently configured preset.
    pub fn preset(&self) -> &PromptPreset {
        &self.preset
    }

    /// Returns whether Termux optimizations are active.
    pub fn is_termux(&self) -> bool {
        self.is_termux
    }

    /// Compiles the complete system prompt string.
    pub fn build(&self) -> String {
        let mut prompt = String::with_capacity(4096);

        // 1. Base domain prompt
        prompt.push_str(self.preset.system_prompt());

        // 2. Termux addendum if in Termux and preset is not already Termux
        if self.is_termux && self.preset != PromptPreset::Termux {
            prompt.push_str(TERMUX_ADDENDUM);
        }

        // 3. Workspace context
        if let Some(ctx) = &self.workspace_context {
            if !ctx.trim().is_empty() {
                prompt.push_str("\n\nWorkspace Context:\n");
                prompt.push_str(ctx.trim());
            }
        }

        // 4. Tool instructions
        if let Some(tool_ctx) = &self.tool_instructions {
            if !tool_ctx.trim().is_empty() {
                prompt.push_str("\n\nTool Instructions:\n");
                prompt.push_str(tool_ctx.trim());
            }
        }

        // 5. Custom user instructions
        if let Some(custom) = &self.custom_instructions {
            if !custom.trim().is_empty() {
                prompt.push_str("\n\nUser Instructions:\n");
                prompt.push_str(custom.trim());
            }
        }

        // 6. Domain Skills
        if let Some(skills) = &self.domain_skills {
            if !skills.trim().is_empty() {
                prompt.push_str("\n\n");
                prompt.push_str(skills.trim());
            }
        }
        // 7. Persistent Memory (User Preferences, Project Architecture & Conventions)
        if let Some(mem) = &self.memory_context {
            if !mem.trim().is_empty() {
                prompt.push_str("\n\nPersistent Memory (Preferences, Architecture & Conventions):\n");
                prompt.push_str(mem.trim());
            }
        }

        // 6. Advisor critiques
        if let Some(critiques) = &self.advisor_critiques {
            if !critiques.trim().is_empty() {
                prompt.push_str("\n\n");
                prompt.push_str(critiques.trim());
            }
        }

        prompt
    }
}

// ---------------------------------------------------------------------------
// Standalone Functions & Helpers
// ---------------------------------------------------------------------------

/// Returns the static general system prompt.
pub fn general_system_prompt() -> &'static str {
    GENERAL_SYSTEM_PROMPT
}

/// Returns the static Rust domain prompt.
pub fn rust_system_prompt() -> &'static str {
    RUST_SYSTEM_PROMPT
}

/// Returns the static TypeScript domain prompt.
pub fn typescript_system_prompt() -> &'static str {
    TYPESCRIPT_SYSTEM_PROMPT
}

/// Returns the static Python domain prompt.
pub fn python_system_prompt() -> &'static str {
    PYTHON_SYSTEM_PROMPT
}

/// Returns the static Go domain prompt.
pub fn go_system_prompt() -> &'static str {
    GO_SYSTEM_PROMPT
}

/// Returns the static Mobile / Termux domain prompt.
pub fn termux_system_prompt() -> &'static str {
    TERMUX_SYSTEM_PROMPT
}

/// Returns the static system prompt for any preset.
pub fn get_preset_prompt(preset: &PromptPreset) -> &'static str {
    preset.system_prompt()
}

/// Detects whether the current process is running inside Android Termux.
pub fn is_termux_environment() -> bool {
    // 1. Explicit TERMUX_VERSION environment variable
    if std::env::var_os("TERMUX_VERSION").is_some() {
        return true;
    }

    // 2. PREFIX pointing to termux directory
    if let Ok(prefix) = std::env::var("PREFIX") {
        if prefix.contains("com.termux") {
            return true;
        }
    }

    // 3. Check for standard Termux files directory
    Path::new("/data/data/com.termux/files/usr").exists()
}

/// Inspects a workspace root directory to detect the primary programming language.
pub fn detect_project_language(workspace_root: &Path) -> PromptPreset {
    // Check for manifest files in order of common specificity
    if workspace_root.join("Cargo.toml").exists() {
        return PromptPreset::Rust;
    }

    if workspace_root.join("tsconfig.json").exists()
        || workspace_root.join("package.json").exists()
        || workspace_root.join("deno.json").exists()
        || workspace_root.join("deno.jsonc").exists()
        || workspace_root.join("bun.lockb").exists()
        || workspace_root.join("bun.lock").exists()
    {
        return PromptPreset::TypeScript;
    }

    if workspace_root.join("pyproject.toml").exists()
        || workspace_root.join("requirements.txt").exists()
        || workspace_root.join("setup.py").exists()
        || workspace_root.join("Pipfile").exists()
        || workspace_root.join("poetry.lock").exists()
        || workspace_root.join("uv.lock").exists()
    {
        return PromptPreset::Python;
    }

    if workspace_root.join("go.mod").exists() || workspace_root.join("go.work").exists() {
        return PromptPreset::Go;
    }

    if workspace_root.join("termux.properties").exists() {
        return PromptPreset::Termux;
    }

    // Fallback: if we are running in Termux with no recognized project manifest
    if is_termux_environment() {
        return PromptPreset::Termux;
    }

    PromptPreset::General
}

/// Inspects a single file path to detect its programming language.
pub fn detect_file_language(path: &Path) -> Option<PromptPreset> {
    PromptPreset::detect_from_path(path)
}

/// Composes a tailored system prompt with optional Termux and custom notes.
pub fn compose_system_prompt(
    preset: PromptPreset,
    is_termux: bool,
    extra_instructions: Option<&str>,
) -> String {
    let mut builder = SystemPromptBuilder::new()
        .with_preset(preset)
        .with_termux(is_termux);

    if let Some(extra) = extra_instructions {
        builder = builder.with_custom_instructions(extra);
    }

    builder.build()
}

// ---------------------------------------------------------------------------
// Domain Prompt Templates (Structured Task Prompts per Domain)
// ---------------------------------------------------------------------------

/// Kinds of structured task prompts that can be generated per domain preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DomainTask {
    /// Structured code review task.
    Review,
    /// Behavior-preserving refactoring task.
    Refactor,
    /// Comprehensive test generation task.
    TestGeneration,
    /// Root cause debugging task.
    Debug,
    /// Security / threat-model audit task.
    SecurityAudit,
    /// Documentation / docstring authoring task.
    Documentation,
}

impl DomainTask {
    /// Returns a short human-readable label of the task.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Review => "Code Review",
            Self::Refactor => "Refactoring",
            Self::TestGeneration => "Test Generation",
            Self::Debug => "Debugging",
            Self::SecurityAudit => "Security Audit",
            Self::Documentation => "Documentation",
        }
    }

    /// Domain-specific acceptance criteria emphasized for this task under the preset.
    pub fn domain_criteria(&self, preset: &PromptPreset) -> &'static str {
        match self {
            Self::Review => match preset {
                PromptPreset::Rust => {
                    "- Verify borrow checker compliance and absence of needless clones\n- Confirm no unverified unwrap/expect in library paths\n- Inspect unsafe blocks for documented SAFETY invariants"
                }
                PromptPreset::TypeScript => {
                    "- Confirm zero `any` leaks and exhaustive nullability handling\n- Verify discriminated union state machines are closed\n- Check floating promises are all awaited or handled"
                }
                PromptPreset::Python => {
                    "- Confirm full type annotations pass mypy/pyright strict\n- Verify exception hierarchy derives from Exception\n- Check no blocking calls inside the asyncio loop"
                }
                PromptPreset::Go => {
                    "- Confirm every error is checked and wrapped with %w\n- Verify goroutine lifecycles terminate deterministically\n- Inspect context propagation through I/O boundaries"
                }
                PromptPreset::Termux => {
                    "- Confirm memory streaming with no unbounded buffers\n- Verify Termux $PREFIX path conventions respected\n- Check output stays within mobile column widths"
                }
                PromptPreset::General | PromptPreset::Custom(_) => {
                    "- Verify correctness across edge cases and error paths\n- Confirm API contracts and invariants hold\n- Check style and conventions consistency"
                }
            },
            Self::Refactor => match preset {
                PromptPreset::Rust => {
                    "- Preserve ownership/borrowing structure; no behavior drift\n- Prefer zero-cost iterator pipelines over manual loops\n- Keep error enums and ?-propagation idiomatic"
                }
                PromptPreset::TypeScript => {
                    "- Preserve discriminated unions and strict narrowing\n- Maintain named exports for tree-shaking\n- Keep ESM module boundaries intact"
                }
                PromptPreset::Python => {
                    "- Preserve type annotations and protocol shapes\n- Prefer comprehensions and pathlib over legacy patterns\n- Keep dataclass/Pydantic boundaries clean"
                }
                PromptPreset::Go => {
                    "- Preserve explicit error wrapping semantics\n- Keep interfaces small and consumer-defined\n- Respect zero-value usability of structs"
                }
                PromptPreset::Termux => {
                    "- Preserve streaming/chunked processing\n- Maintain battery-friendly event-driven loops\n- Keep resource footprint unchanged or smaller"
                }
                PromptPreset::General | PromptPreset::Custom(_) => {
                    "- Preserve external behavior and API contracts\n- Eliminate duplication without premature abstraction\n- Maintain or improve test coverage"
                }
            },
            Self::TestGeneration => match preset {
                PromptPreset::Rust => "Use #[test] with assert!/assert_eq!, cover error variants with Result-returning tests, include doctests where meaningful",
                PromptPreset::TypeScript => "Use vitest/jest describe/it blocks with discriminated-union fixtures, test both resolve and reject promise paths",
                PromptPreset::Python => "Use pytest with parametrize and fixtures, cover async paths with pytest-asyncio, test exception raising with pytest.raises",
                PromptPreset::Go => "Use table-driven tests with t.Run subtests, cover error branches, use httptest for network boundaries",
                PromptPreset::Termux => "Keep tests lightweight and fast (no heavy polling), assert memory-bounded behavior, run under Termux $PREFIX tooling",
                PromptPreset::General | PromptPreset::Custom(_) => "Cover happy path, boundary values, empty inputs, overflow, and error scenarios deterministically",
            },
            Self::Debug => match preset {
                PromptPreset::Rust => "Diagnose against borrow checker output, backtrace with RUST_BACKTRACE=1, distinguish panic vs error-path failure",
                PromptPreset::TypeScript => "Reproduce under strict tsconfig, inspect promise rejection chains, verify runtime-specific (Node/Bun/Deno) behavior",
                PromptPreset::Python => "Reproduce with full traceback preserved, distinguish asyncio cancellation from exceptions, check mypy diagnostics",
                PromptPreset::Go => "Diagnose with go vet findings, inspect goroutine dumps, verify error wrapping chains with errors.Is/As",
                PromptPreset::Termux => "Reproduce under Termux environment ($PREFIX paths, Bionic libc), account for Android phantom process kills and Doze",
                PromptPreset::General | PromptPreset::Custom(_) => "Form hypothesis, construct minimal reproduction, apply fix eliminating root cause",
            },
            Self::SecurityAudit => match preset {
                PromptPreset::Rust => "Audit unsafe blocks for soundness, check integer overflow arithmetic, inspect FFI boundaries for C string validity",
                PromptPreset::TypeScript => "Audit untrusted input validation schemas, check prototype pollution in object merges, inspect XSS sinks",
                PromptPreset::Python => "Audit deserialization (pickle/yaml.load), check injection in SQL/eval paths, inspect subprocess shell=True usage",
                PromptPreset::Go => "Audit path traversal in file handling, check command injection via exec.Command, inspect race conditions with -race",
                PromptPreset::Termux => "Audit permission boundaries in Android user-space, inspect secret storage in $HOME, check network resilience to cellular MITM",
                PromptPreset::General | PromptPreset::Custom(_) => "Threat-model injection, auth bypass, DoS vectors, and secret exposure",
            },
            Self::Documentation => match preset {
                PromptPreset::Rust => "Write /// doc comments with runnable doctests, document panics, errors, and SAFETY preconditions",
                PromptPreset::TypeScript => "Write TSDoc with @param/@returns/@throws, include usage examples with explicit types",
                PromptPreset::Python => "Write PEP 257 docstrings with type annotations, document raises and async semantics",
                PromptPreset::Go => "Write idiomatic doc comments starting with the identifier name, document error semantics",
                PromptPreset::Termux => "Document Termux-specific setup ($PREFIX, pkg), memory constraints, and mobile usage caveats",
                PromptPreset::General | PromptPreset::Custom(_) => "Document purpose, parameters, returns, errors, and provide runnable examples",
            },
        }
    }
}

/// Generates a complete, structured task prompt tailored to the domain preset.
///
/// Produces a ready-to-send prompt combining the task objective, the domain
/// acceptance criteria, and the supplied subject matter (code, diff, errors...).
pub fn domain_task_prompt(preset: &PromptPreset, task: DomainTask, subject: &str) -> String {
    let mut out = String::with_capacity(subject.len() + 1024);
    out.push_str(&format!("## Task: {}\n", task.label()));
    out.push_str(&format!("Domain: {}\n\n", preset));
    out.push_str(&format!(
        "You are operating as: {}\n{}\n\n",
        preset.name(),
        preset.description()
    ));
    out.push_str("### Acceptance Criteria\n");
    out.push_str(task.domain_criteria(preset));
    out.push_str("\n\n### Subject Matter\n");
    out.push_str("```\n");
    out.push_str(subject);
    out.push_str("\n```\n");
    out
}

/// Builds a structured task prompt and returns it with its estimated token cost.
/// Returns `(prompt, estimated_tokens)`.
pub fn domain_task_prompt_with_tokens(
    preset: &PromptPreset,
    task: DomainTask,
    subject: &str,
) -> (String, usize) {
    let prompt = domain_task_prompt(preset, task, subject);
    let tokens = crate::agent::tokens::estimate_text_tokens(&prompt);
    (prompt, tokens)
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn test_prompt_presets_from_str() {
        assert_eq!("rust".parse::<PromptPreset>().unwrap(), PromptPreset::Rust);
        assert_eq!("rs".parse::<PromptPreset>().unwrap(), PromptPreset::Rust);
        assert_eq!(
            "typescript".parse::<PromptPreset>().unwrap(),
            PromptPreset::TypeScript
        );
        assert_eq!(
            "ts".parse::<PromptPreset>().unwrap(),
            PromptPreset::TypeScript
        );
        assert_eq!(
            "python".parse::<PromptPreset>().unwrap(),
            PromptPreset::Python
        );
        assert_eq!("py".parse::<PromptPreset>().unwrap(), PromptPreset::Python);
        assert_eq!("go".parse::<PromptPreset>().unwrap(), PromptPreset::Go);
        assert_eq!("golang".parse::<PromptPreset>().unwrap(), PromptPreset::Go);
        assert_eq!(
            "termux".parse::<PromptPreset>().unwrap(),
            PromptPreset::Termux
        );
        assert_eq!(
            "android".parse::<PromptPreset>().unwrap(),
            PromptPreset::Termux
        );
        assert_eq!(
            "general".parse::<PromptPreset>().unwrap(),
            PromptPreset::General
        );
        assert_eq!(
            "special".parse::<PromptPreset>().unwrap(),
            PromptPreset::Custom("special".into())
        );
    }

    #[test]
    fn test_prompt_preset_display() {
        assert_eq!(PromptPreset::Rust.to_string(), "rust");
        assert_eq!(PromptPreset::TypeScript.to_string(), "typescript");
        assert_eq!(PromptPreset::Python.to_string(), "python");
        assert_eq!(PromptPreset::Go.to_string(), "go");
        assert_eq!(PromptPreset::Termux.to_string(), "termux");
        assert_eq!(PromptPreset::General.to_string(), "general");
        assert_eq!(
            PromptPreset::Custom("embedded".into()).to_string(),
            "custom:embedded"
        );
    }
    #[test]
    fn test_general_system_prompt_content() {
        let prompt = general_system_prompt();
        assert!(prompt.contains("Universal Terminal Diagrams"));
        assert!(prompt.contains("ASCII/Unicode box art"));
        assert!(prompt.contains("```text or ```ascii"));
        assert!(prompt.contains("Avoid raw Mermaid syntax unless explicitly requested."));
    }

    #[test]
    fn test_rust_system_prompt_content() {
        let prompt = rust_system_prompt();
        assert!(prompt.contains("expert Rust systems and application engineer"));
        assert!(prompt.contains("Ownership, Borrowing & Lifetimes"));
        assert!(prompt.contains("thiserror"));
        assert!(prompt.contains("anyhow"));
        assert!(prompt.contains("tokio"));
        assert!(prompt.contains("SAFETY:"));
        assert!(prompt.contains("clippy"));
    }

    #[test]
    fn test_typescript_system_prompt_content() {
        let prompt = typescript_system_prompt();
        assert!(prompt.contains("expert TypeScript and modern full-stack architect"));
        assert!(prompt.contains("Strict Type Safety"));
        assert!(prompt.contains("Discriminated Unions"));
        assert!(prompt.contains("as const"));
        assert!(prompt.contains("Zod"));
        assert!(prompt.contains("ESM"));
    }

    #[test]
    fn test_python_system_prompt_content() {
        let prompt = python_system_prompt();
        assert!(prompt.contains("expert Python systems and backend engineer"));
        assert!(prompt.contains("Python 3.10+"));
        assert!(prompt.contains("mypy"));
        assert!(prompt.contains("dataclass"));
        assert!(prompt.contains("pathlib.Path"));
        assert!(prompt.contains("TaskGroup"));
    }

    #[test]
    fn test_go_system_prompt_content() {
        let prompt = go_system_prompt();
        assert!(prompt.contains("expert Go systems engineer"));
        assert!(prompt.contains("Effective Go"));
        assert!(prompt.contains("if err != nil"));
        assert!(prompt.contains("context.Context"));
        assert!(prompt.contains("goroutine"));
        assert!(prompt.contains("sync.Mutex"));
    }

    #[test]
    fn test_termux_system_prompt_content() {
        let prompt = termux_system_prompt();
        assert!(prompt.contains("resource-constrained mobile and Android/Termux"));
        assert!(prompt.contains("$PREFIX"));
        assert!(prompt.contains("Bionic libc"));
        assert!(prompt.contains("40-80 columns"));
    }

    #[test]
    fn test_file_extension_detection() {
        assert_eq!(
            PromptPreset::from_file_extension("rs"),
            Some(PromptPreset::Rust)
        );
        assert_eq!(
            PromptPreset::from_file_extension(".ts"),
            Some(PromptPreset::TypeScript)
        );
        assert_eq!(
            PromptPreset::from_file_extension("py"),
            Some(PromptPreset::Python)
        );
        assert_eq!(
            PromptPreset::from_file_extension("go"),
            Some(PromptPreset::Go)
        );
        assert_eq!(PromptPreset::from_file_extension("unknown"), None);
    }

    #[test]
    fn test_detect_from_path() {
        assert_eq!(
            PromptPreset::detect_from_path(Path::new("src/main.rs")),
            Some(PromptPreset::Rust)
        );
        assert_eq!(
            PromptPreset::detect_from_path(Path::new("frontend/app.tsx")),
            Some(PromptPreset::TypeScript)
        );
        assert_eq!(
            PromptPreset::detect_from_path(Path::new("scripts/eval.py")),
            Some(PromptPreset::Python)
        );
        assert_eq!(
            PromptPreset::detect_from_path(Path::new("server/main.go")),
            Some(PromptPreset::Go)
        );
        assert_eq!(
            PromptPreset::detect_from_path(Path::new("Cargo.toml")),
            Some(PromptPreset::Rust)
        );
        assert_eq!(
            PromptPreset::detect_from_path(Path::new("package.json")),
            Some(PromptPreset::TypeScript)
        );
        assert_eq!(
            PromptPreset::detect_from_path(Path::new("go.mod")),
            Some(PromptPreset::Go)
        );
    }

    #[test]
    fn test_system_prompt_builder_basic() {
        let prompt = SystemPromptBuilder::new()
            .with_preset(PromptPreset::Rust)
            .with_termux(false)
            .with_custom_instructions("Always write unit tests.")
            .build();

        assert!(prompt.contains("expert Rust systems"));
        assert!(prompt.contains("Always write unit tests."));
        assert!(!prompt.contains("Environment Note (Android / Termux)"));
    }

    #[test]
    fn test_system_prompt_builder_with_termux_addendum() {
        let prompt = SystemPromptBuilder::new()
            .with_preset(PromptPreset::Python)
            .with_termux(true)
            .build();

        assert!(prompt.contains("expert Python systems"));
        assert!(prompt.contains("Environment Note (Android / Termux)"));
        assert!(prompt.contains("$PREFIX"));
    }

    #[test]
    fn test_system_prompt_builder_with_advisor_and_tools() {
        let prompt = SystemPromptBuilder::new()
            .with_preset(PromptPreset::Go)
            .with_termux(false)
            .with_tool_instructions("Tool available: bash, read, write.")
            .with_workspace_context("Project: Distributed Key-Value Store")
            .with_advisor_critiques("[SecurityAdvisor]: Avoid hardcoded secrets.")
            .build();

        assert!(prompt.contains("expert Go systems"));
        assert!(prompt.contains("Tool available: bash, read, write."));
        assert!(prompt.contains("Distributed Key-Value Store"));
        assert!(prompt.contains("[SecurityAdvisor]: Avoid hardcoded secrets."));
    }

    #[test]
    fn test_compose_system_prompt_helper() {
        let prompt = compose_system_prompt(
            PromptPreset::TypeScript,
            false,
            Some("Use strict React hook conventions."),
        );
        assert!(prompt.contains("TypeScript"));
        assert!(prompt.contains("Use strict React hook conventions."));
    }

    #[test]
    fn test_detect_project_language_cargo() {
        let temp_dir = std::env::temp_dir().join(format!(
            "fusion_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        File::create(temp_dir.join("Cargo.toml")).unwrap();

        let detected = detect_project_language(&temp_dir);
        assert_eq!(detected, PromptPreset::Rust);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_system_prompt_builder_memory_and_skills() {
        let mut store = crate::agent::memory::MemoryStore::new();
        store.remember_preference("indentation", "Always use tabs over spaces.");

        let prompt = SystemPromptBuilder::new()
            .with_preset(PromptPreset::General)
            .with_termux(false)
            .with_memory_store(&store, None)
            .build();

        assert!(prompt.contains("Persistent Memory"));
        assert!(prompt.contains("tabs over spaces"));
    }

    #[test]
    fn test_system_prompt_builder_tool_definitions() {
        let tools = vec![
            crate::provider::types::ToolDefinition {
                name: "bash".to_string(),
                description: "Run a shell command.\nMore details...".to_string(),
                parameters: serde_json::json!({}),
            },
            crate::provider::types::ToolDefinition {
                name: "read".to_string(),
                description: "Read file contents".to_string(),
                parameters: serde_json::json!({}),
            },
        ];

        let prompt = SystemPromptBuilder::new()
            .with_preset(PromptPreset::General)
            .with_termux(false)
            .with_tool_definitions(&tools)
            .build();

        assert!(prompt.contains("Available Tools:"));
        assert!(prompt.contains("- bash: Run a shell command."));
        assert!(prompt.contains("- read: Read file contents"));
        assert!(!prompt.contains("More details..."));
    }

    #[test]
    fn test_system_prompt_builder_empty_tool_definitions_noop() {
        let prompt = SystemPromptBuilder::new()
            .with_preset(PromptPreset::Rust)
            .with_termux(false)
            .with_tool_definitions(&[])
            .build();

        assert!(!prompt.contains("Available Tools:"));
    }

    #[test]
    fn test_system_prompt_builder_token_estimation_and_len() {
        let builder = SystemPromptBuilder::new()
            .with_preset(PromptPreset::Rust)
            .with_termux(false)
            .with_custom_instructions("Extra guidance paragraph.");

        let tokens = builder.estimate_tokens();
        let len = builder.len();

        assert!(tokens > 0, "compiled prompt must have nonzero token estimate");
        assert!(len > 0);
        // Rust prompt is several paragraphs; rough sanity bounds.
        assert!(tokens > 100 && tokens < 20_000);
        assert!(!builder.is_empty());
    }

    #[test]
    fn test_domain_task_label_and_criteria_by_preset() {
        assert_eq!(DomainTask::Review.label(), "Code Review");
        assert_eq!(DomainTask::Refactor.label(), "Refactoring");
        assert_eq!(DomainTask::TestGeneration.label(), "Test Generation");
        assert_eq!(DomainTask::Debug.label(), "Debugging");
        assert_eq!(DomainTask::SecurityAudit.label(), "Security Audit");
        assert_eq!(DomainTask::Documentation.label(), "Documentation");

        let rust_review = DomainTask::Review.domain_criteria(&PromptPreset::Rust);
        assert!(rust_review.contains("borrow checker"));
        assert!(rust_review.contains("SAFETY"));

        let ts_review = DomainTask::Review.domain_criteria(&PromptPreset::TypeScript);
        assert!(ts_review.contains("`any`"));

        let go_review = DomainTask::Review.domain_criteria(&PromptPreset::Go);
        assert!(go_review.contains("%w"));

        let py_review = DomainTask::Review.domain_criteria(&PromptPreset::Python);
        assert!(py_review.contains("mypy"));

        let termux_review = DomainTask::Review.domain_criteria(&PromptPreset::Termux);
        assert!(termux_review.contains("$PREFIX"));

        let general_review = DomainTask::Review.domain_criteria(&PromptPreset::General);
        assert!(general_review.contains("edge cases"));
    }

    #[test]
    fn test_domain_task_prompt_structure() {
        let subject = "fn add(a: u32, b: u32) -> u32 { a + b }";
        let prompt = domain_task_prompt(&PromptPreset::Rust, DomainTask::Review, subject);

        assert!(prompt.contains("## Task: Code Review"));
        assert!(prompt.contains("Domain: rust"));
        assert!(prompt.contains("Rust Systems Engineer"));
        assert!(prompt.contains("### Acceptance Criteria"));
        assert!(prompt.contains("borrow checker"));
        assert!(prompt.contains("### Subject Matter"));
        assert!(prompt.contains(subject));
    }

    #[test]
    fn test_domain_task_prompt_with_tokens() {
        let (prompt, tokens) = domain_task_prompt_with_tokens(
            &PromptPreset::TypeScript,
            DomainTask::SecurityAudit,
            "const apiKey = location.hash;",
        );

        assert!(prompt.contains("Security Audit"));
        assert!(prompt.contains("XSS"));
        assert!(tokens > 0);
        // Token estimate should be roughly proportional to prompt length.
        assert!(tokens * 2 <= prompt.len() + 16);
    }

    #[test]
    fn test_domain_task_prompts_differ_by_preset() {
        let subject = "SELECT * FROM users WHERE id = $1";
        let rust_prompt = domain_task_prompt(&PromptPreset::Rust, DomainTask::SecurityAudit, subject);
        let go_prompt = domain_task_prompt(&PromptPreset::Go, DomainTask::SecurityAudit, subject);

        assert!(rust_prompt.contains("unsafe"));
        assert!(go_prompt.contains("race"));
        assert_ne!(rust_prompt, go_prompt);
    }
}
