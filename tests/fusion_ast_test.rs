//! Integration tests for `fusion-ast`.
//!
//! Comprehensive test suite verifying:
//! 1. Syntactic block resolution via `fusion_ast::block::block_range_at` for Rust, Go, and TypeScript
//! 2. Language detection by file extension and special filenames via `SupportLang::from_path`
//! 3. Extension-based language resolution wired through `block_range_at`
//! 4. Robust handling of edge cases (blank lines, closing delimiters, out-of-bounds lines, syntax errors)

use std::path::Path;

use fusion_ast::block::{block_range_at, BlockRange, BlockRangeOptions};
use fusion_ast::SupportLang;

/// Helper function to resolve block range given code, file path, and 1-indexed line.
fn resolve_by_path(code: &str, path: &str, line: u32) -> Option<BlockRange> {
    block_range_at(BlockRangeOptions {
        code: code.to_string(),
        lang: None,
        path: Some(path.to_string()),
        line,
    })
    .expect("block_range_at should execute without internal error")
}

/// Helper function to resolve block range given code, explicit language alias, and 1-indexed line.
fn resolve_by_lang(code: &str, lang: &str, line: u32) -> Option<BlockRange> {
    block_range_at(BlockRangeOptions {
        code: code.to_string(),
        lang: Some(lang.to_string()),
        path: None,
        line,
    })
    .expect("block_range_at should execute without internal error")
}

// ============================================================================
// 1. Rust Block Range Tests (`block_range_at`)
// ============================================================================

const RUST_SAMPLE: &str = r#"fn calculate_total(items: &[u32]) -> u32 {
    let mut sum = 0;
    for item in items {
        if *item > 0 {
            sum += item;
        }
    }
    sum
}

struct Point {
    x: f64,
    y: f64,
}

enum Status {
    Pending,
    Active { started_at: u64 },
    Completed,
}

impl Point {
    fn distance(&self, other: &Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}
"#;

#[test]
fn test_rust_function_block_range() {
    // Line 1: `fn calculate_total(...) -> u32 {` spans lines 1 through 9
    let range = resolve_by_path(RUST_SAMPLE, "src/math.rs", 1);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 1,
            end_line: 9,
        })
    );

    // Line 3: `for item in items {` spans lines 3 through 7
    let range = resolve_by_path(RUST_SAMPLE, "src/math.rs", 3);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 3,
            end_line: 7,
        })
    );

    // Line 4: `if *item > 0 {` spans lines 4 through 6
    let range = resolve_by_path(RUST_SAMPLE, "src/math.rs", 4);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 4,
            end_line: 6,
        })
    );
}

#[test]
fn test_rust_struct_and_enum_block_range() {
    // Line 11: `struct Point {` spans lines 11 through 14
    let range = resolve_by_path(RUST_SAMPLE, "src/math.rs", 11);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 11,
            end_line: 14,
        })
    );

    // Line 16: `enum Status {` spans lines 16 through 20
    let range = resolve_by_path(RUST_SAMPLE, "src/math.rs", 16);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 16,
            end_line: 20,
        })
    );
}

#[test]
fn test_rust_impl_block_and_method() {
    // Line 22: `impl Point {` spans lines 22 through 28
    let range = resolve_by_path(RUST_SAMPLE, "src/math.rs", 22);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 22,
            end_line: 28,
        })
    );

    // Line 23: `fn distance(&self, other: &Point) -> f64 {` spans lines 23 through 27
    let range = resolve_by_path(RUST_SAMPLE, "src/math.rs", 23);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 23,
            end_line: 27,
        })
    );
}

#[test]
fn test_rust_continuations_and_delimiters_return_none() {
    // Line 2: `let mut sum = 0;` - single statement inside fn, not a multi-line block opener
    // In block_range_at, single-line statements resolve start=2, end=2
    let stmt = resolve_by_path(RUST_SAMPLE, "src/math.rs", 2);
    assert_eq!(
        stmt,
        Some(BlockRange {
            start_line: 2,
            end_line: 2
        })
    );

    // Line 9: `}` closing brace of fn - no block begins here
    let closing = resolve_by_path(RUST_SAMPLE, "src/math.rs", 9);
    assert_eq!(closing, None);

    // Line 10: Blank line - no block begins here
    let blank = resolve_by_path(RUST_SAMPLE, "src/math.rs", 10);
    assert_eq!(blank, None);
}

// ============================================================================
// 2. Go Block Range Tests (`block_range_at`)
// ============================================================================

const GO_SAMPLE: &str = r#"package main

import "fmt"

func ProcessItems(items []string) int {
	count := 0
	for idx, item := range items {
		if len(item) > 0 {
			count += len(item)
			fmt.Println(idx, item)
		}
	}
	return count
}

type Server struct {
	Host string
	Port int
}

type Service interface {
	Start() error
	Stop() error
}
"#;

#[test]
fn test_go_function_and_control_flow_block_range() {
    // Line 5: `func ProcessItems(items []string) int {` spans lines 5 through 14
    let range = resolve_by_path(GO_SAMPLE, "server.go", 5);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 5,
            end_line: 14,
        })
    );

    // Line 7: `for idx, item := range items {` spans lines 7 through 12
    let range = resolve_by_path(GO_SAMPLE, "server.go", 7);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 7,
            end_line: 12,
        })
    );

    // Line 8: `if len(item) > 0 {` spans lines 8 through 11
    let range = resolve_by_path(GO_SAMPLE, "server.go", 8);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 8,
            end_line: 11,
        })
    );
}

#[test]
fn test_go_struct_and_interface_block_range() {
    // Line 16: `type Server struct {` spans lines 16 through 19
    let range = resolve_by_path(GO_SAMPLE, "server.go", 16);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 16,
            end_line: 19,
        })
    );

    // Line 21: `type Service interface {` spans lines 21 through 24
    let range = resolve_by_path(GO_SAMPLE, "server.go", 21);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 21,
            end_line: 24,
        })
    );
}

#[test]
fn test_go_package_and_delimiter_handling() {
    // Line 1: `package main` - single line package declaration
    let pkg = resolve_by_path(GO_SAMPLE, "server.go", 1);
    assert_eq!(
        pkg,
        Some(BlockRange {
            start_line: 1,
            end_line: 1,
        })
    );

    // Line 14: `}` lone closing brace of func ProcessItems
    let closing = resolve_by_path(GO_SAMPLE, "server.go", 14);
    assert_eq!(closing, None);

    // Line 15: Blank separator line
    let blank = resolve_by_path(GO_SAMPLE, "server.go", 15);
    assert_eq!(blank, None);
}

// ============================================================================
// 3. TypeScript Block Range Tests (`block_range_at`)
// ============================================================================

const TS_SAMPLE: &str = r#"export class ServiceManager {
  private name: string;

  constructor(name: string) {
    this.name = name;
  }

  async processRequest(payload: unknown): Promise<void> {
    if (payload) {
      console.log("Processing payload");
    } else {
      throw new Error("Missing payload");
    }
  }
}

interface WorkerOptions {
  concurrency: number;
  timeoutMs: number;
}

type ResultHandler = (res: Response) => void;
"#;

#[test]
fn test_typescript_class_and_methods_block_range() {
    // Line 1: `export class ServiceManager {` spans lines 1 through 15
    let range = resolve_by_path(TS_SAMPLE, "service.ts", 1);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 1,
            end_line: 15,
        })
    );

    // Line 4: `constructor(name: string) {` spans lines 4 through 6
    let range = resolve_by_path(TS_SAMPLE, "service.ts", 4);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 4,
            end_line: 6,
        })
    );

    // Line 8: `async processRequest(payload: unknown): Promise<void> {` spans lines 8 through 14
    let range = resolve_by_path(TS_SAMPLE, "service.ts", 8);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 8,
            end_line: 14,
        })
    );

    // Line 9: `if (payload) { ... } else { ... }` spans lines 9 through 13
    let range = resolve_by_path(TS_SAMPLE, "service.ts", 9);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 9,
            end_line: 13,
        })
    );
}

#[test]
fn test_typescript_interface_and_type_alias_block_range() {
    // Line 17: `interface WorkerOptions {` spans lines 17 through 20
    let range = resolve_by_path(TS_SAMPLE, "service.ts", 17);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 17,
            end_line: 20,
        })
    );

    // Line 22: `type ResultHandler = (res: Response) => void;` spans line 22
    let range = resolve_by_path(TS_SAMPLE, "service.ts", 22);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 22,
            end_line: 22,
        })
    );
}

#[test]
fn test_typescript_explicit_lang_option() {
    // Using `lang: Some("typescript")` without path
    let range = resolve_by_lang(TS_SAMPLE, "typescript", 1);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 1,
            end_line: 15,
        })
    );

    // Alias "ts" is also recognized
    let range = resolve_by_lang(TS_SAMPLE, "ts", 8);
    assert_eq!(
        range,
        Some(BlockRange {
            start_line: 8,
            end_line: 14,
        })
    );
}

// ============================================================================
// 4. Language Detection by Extension & Special Filenames
// ============================================================================

#[test]
fn test_language_detection_by_standard_extension() {
    // Core systems and backend languages
    assert_eq!(SupportLang::from_path(Path::new("src/main.rs")), Some(SupportLang::Rust));
    assert_eq!(SupportLang::from_path(Path::new("cmd/api/main.go")), Some(SupportLang::Go));
    assert_eq!(SupportLang::from_path(Path::new("app/index.ts")), Some(SupportLang::TypeScript));
    assert_eq!(SupportLang::from_path(Path::new("components/App.tsx")), Some(SupportLang::Tsx));
    assert_eq!(SupportLang::from_path(Path::new("lib/util.js")), Some(SupportLang::JavaScript));
    assert_eq!(SupportLang::from_path(Path::new("scripts/build.py")), Some(SupportLang::Python));
    assert_eq!(SupportLang::from_path(Path::new("native/core.c")), Some(SupportLang::C));
    assert_eq!(SupportLang::from_path(Path::new("native/engine.cpp")), Some(SupportLang::Cpp));
    assert_eq!(SupportLang::from_path(Path::new("native/engine.cc")), Some(SupportLang::Cpp));
    assert_eq!(SupportLang::from_path(Path::new("native/engine.cxx")), Some(SupportLang::Cpp));
    assert_eq!(SupportLang::from_path(Path::new("App.java")), Some(SupportLang::Java));
    assert_eq!(SupportLang::from_path(Path::new("Program.cs")), Some(SupportLang::CSharp));
    assert_eq!(SupportLang::from_path(Path::new("app.rb")), Some(SupportLang::Ruby));
    assert_eq!(SupportLang::from_path(Path::new("index.php")), Some(SupportLang::Php));
    assert_eq!(SupportLang::from_path(Path::new("Main.swift")), Some(SupportLang::Swift));
    assert_eq!(SupportLang::from_path(Path::new("main.zig")), Some(SupportLang::Zig));
    assert_eq!(SupportLang::from_path(Path::new("Main.kt")), Some(SupportLang::Kotlin));
    assert_eq!(SupportLang::from_path(Path::new("script.lua")), Some(SupportLang::Lua));
    assert_eq!(SupportLang::from_path(Path::new("deploy.sh")), Some(SupportLang::Bash));

    // Data, Config, and Markup formats
    assert_eq!(SupportLang::from_path(Path::new("package.json")), Some(SupportLang::Json));
    assert_eq!(SupportLang::from_path(Path::new("config.toml")), Some(SupportLang::Toml));
    assert_eq!(SupportLang::from_path(Path::new("docker-compose.yaml")), Some(SupportLang::Yaml));
    assert_eq!(SupportLang::from_path(Path::new("ci.yml")), Some(SupportLang::Yaml));
    assert_eq!(SupportLang::from_path(Path::new("index.html")), Some(SupportLang::Html));
    assert_eq!(SupportLang::from_path(Path::new("styles.css")), Some(SupportLang::Css));
    assert_eq!(SupportLang::from_path(Path::new("README.md")), Some(SupportLang::Markdown));
    assert_eq!(SupportLang::from_path(Path::new("schema.sql")), Some(SupportLang::Sql));
    assert_eq!(SupportLang::from_path(Path::new("data.xml")), Some(SupportLang::Xml));
}

#[test]
fn test_language_detection_by_special_filenames() {
    // Exact filenames without standard extensions
    assert_eq!(SupportLang::from_path(Path::new("Makefile")), Some(SupportLang::Make));
    assert_eq!(SupportLang::from_path(Path::new("makefile")), Some(SupportLang::Make));
    assert_eq!(SupportLang::from_path(Path::new("GNUmakefile")), Some(SupportLang::Make));
    assert_eq!(SupportLang::from_path(Path::new("Justfile")), Some(SupportLang::Just));
    assert_eq!(SupportLang::from_path(Path::new("justfile")), Some(SupportLang::Just));
    assert_eq!(SupportLang::from_path(Path::new("CMakeLists.txt")), Some(SupportLang::Cmake));
    assert_eq!(SupportLang::from_path(Path::new("Dockerfile")), Some(SupportLang::Dockerfile));
    assert_eq!(SupportLang::from_path(Path::new("dockerfile")), Some(SupportLang::Dockerfile));
    assert_eq!(SupportLang::from_path(Path::new("Dockerfile.prod")), Some(SupportLang::Dockerfile));
    assert_eq!(SupportLang::from_path(Path::new("Containerfile")), Some(SupportLang::Dockerfile));

    // Shell profile / rc files
    assert_eq!(SupportLang::from_path(Path::new(".bashrc")), Some(SupportLang::Bash));
    assert_eq!(SupportLang::from_path(Path::new(".bash_profile")), Some(SupportLang::Bash));
    assert_eq!(SupportLang::from_path(Path::new(".zshrc")), Some(SupportLang::Bash));
    assert_eq!(SupportLang::from_path(Path::new(".zshenv")), Some(SupportLang::Bash));
    assert_eq!(SupportLang::from_path(Path::new(".emacs")), Some(SupportLang::EmacsLisp));
}

#[test]
fn test_language_detection_unknown_extensions() {
    assert_eq!(SupportLang::from_path(Path::new("file.unknown_ext")), None);
    assert_eq!(SupportLang::from_path(Path::new("file.xyz123")), None);
    assert_eq!(SupportLang::from_path(Path::new("file.bin")), None);
    assert_eq!(SupportLang::from_path(Path::new("some_random_file")), None);
}

// ============================================================================
// 5. Edge Cases & Error Boundaries
// ============================================================================

#[test]
fn test_block_range_unrecognized_extension_returns_none() {
    let code = "fn test() { println!(\"hello\"); }";
    // .unrecognized has no tree-sitter grammar mapped
    let res = resolve_by_path(code, "file.unrecognized", 1);
    assert_eq!(res, None);
}

#[test]
fn test_block_range_out_of_bounds_lines() {
    let code = "fn hello() {\n    println!(\"world\");\n}\n";
    // 0 is invalid 1-indexed line
    assert_eq!(resolve_by_path(code, "test.rs", 0), None);
    // Line 99 exceeds line count
    assert_eq!(resolve_by_path(code, "test.rs", 99), None);
}

#[test]
fn test_block_range_empty_code() {
    assert_eq!(resolve_by_path("", "test.rs", 1), None);
    assert_eq!(resolve_by_lang("", "rust", 1), None);
}

#[test]
fn test_block_range_syntax_error_handling() {
    // Unclosed brace causes syntax error in subtree; block_range_at returns None safely
    let broken_code = "fn broken() {\n    let x = \n";
    let res = resolve_by_path(broken_code, "test.rs", 1);
    assert_eq!(res, None);
}
