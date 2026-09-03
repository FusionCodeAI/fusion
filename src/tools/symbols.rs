//! Symbol and declaration scanner tool.
//!
//! Fast regex-based code symbol extractor for functions, structs, classes,
//! traits, interfaces, enums, type aliases, modules, constants, and macros
//! across workspace files in all major programming languages.

use async_trait::async_trait;
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use crate::tools::file::resolve_path;
use crate::tools::types::{Tool, ToolContext};

// ---------------------------------------------------------------------------
// Symbol Kind
// ---------------------------------------------------------------------------

/// Categorization of discovered code symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Struct,
    Class,
    Trait,
    Interface,
    Enum,
    TypeAlias,
    Module,
    Constant,
    Macro,
    Variable,
}

impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Struct => "struct",
            SymbolKind::Class => "class",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::Enum => "enum",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Module => "module",
            SymbolKind::Constant => "constant",
            SymbolKind::Macro => "macro",
            SymbolKind::Variable => "variable",
        }
    }

    pub fn display_tag(&self) -> &'static str {
        match self {
            SymbolKind::Function => "fn",
            SymbolKind::Struct => "struct",
            SymbolKind::Class => "class",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "iface",
            SymbolKind::Enum => "enum",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Module => "mod",
            SymbolKind::Constant => "const",
            SymbolKind::Macro => "macro",
            SymbolKind::Variable => "var",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        let clean = s.trim().to_lowercase();
        match clean.as_str() {
            "fn" | "func" | "function" | "def" | "method" | "fun" => Some(SymbolKind::Function),
            "struct" | "structure" | "record" => Some(SymbolKind::Struct),
            "class" | "object" | "actor" => Some(SymbolKind::Class),
            "trait" | "protocol" | "mixin" => Some(SymbolKind::Trait),
            "interface" | "iface" => Some(SymbolKind::Interface),
            "enum" | "enumeration" => Some(SymbolKind::Enum),
            "type" | "typedef" | "alias" | "typealias" => Some(SymbolKind::TypeAlias),
            "mod" | "module" | "namespace" | "ns" | "package" | "pkg" => Some(SymbolKind::Module),
            "const" | "constant" | "static" => Some(SymbolKind::Constant),
            "macro" | "macro_rules" => Some(SymbolKind::Macro),
            "var" | "variable" | "let" => Some(SymbolKind::Variable),
            _ => None,
        }
    }

    pub fn matches_filter(&self, filter: &str) -> bool {
        let f = filter.trim().to_lowercase();
        if f.is_empty() || f == "all" || f == "*" {
            return true;
        }

        if let Some(target) = Self::from_str_loose(&f) {
            if *self == target {
                return true;
            }
            // Group interface and trait together if searching for either
            if (target == SymbolKind::Trait || target == SymbolKind::Interface)
                && (*self == SymbolKind::Trait || *self == SymbolKind::Interface)
            {
                return true;
            }
            // Group class and struct together if searching for "type" or "class"
            if target == SymbolKind::TypeAlias
                && (*self == SymbolKind::TypeAlias
                    || *self == SymbolKind::Struct
                    || *self == SymbolKind::Enum)
            {
                return true;
            }
        }

        self.as_str().contains(&f) || self.display_tag().contains(&f)
    }
}

// ---------------------------------------------------------------------------
// Symbol Data Model
// ---------------------------------------------------------------------------

/// A discovered symbol with file location, signature, and metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub signature: String,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_comment: Option<String>,
}

impl Symbol {
    /// Full qualified name including container (e.g. `User::validate` or `GrepTool.execute`).
    pub fn qualified_name(&self) -> String {
        if let Some(parent) = &self.container {
            if self.language == "rust" || self.language == "cpp" {
                format!("{}::{}", parent, self.name)
            } else {
                format!("{}.{}", parent, self.name)
            }
        } else {
            self.name.clone()
        }
    }
}

// ---------------------------------------------------------------------------
// Supported Language Definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Cpp,
    C,
    Java,
    CSharp,
    Kotlin,
    Scala,
    Swift,
    Ruby,
    Php,
    Zig,
    Dart,
    Lua,
    Shell,
    Elixir,
    Sql,
    Generic,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::Cpp => "cpp",
            Language::C => "c",
            Language::Java => "java",
            Language::CSharp => "csharp",
            Language::Kotlin => "kotlin",
            Language::Scala => "scala",
            Language::Swift => "swift",
            Language::Ruby => "ruby",
            Language::Php => "php",
            Language::Zig => "zig",
            Language::Dart => "dart",
            Language::Lua => "lua",
            Language::Shell => "shell",
            Language::Elixir => "elixir",
            Language::Sql => "sql",
            Language::Generic => "generic",
        }
    }

    pub fn from_extension(ext: &str) -> Self {
        let e = ext.to_lowercase();
        match e.as_str() {
            "rs" => Language::Rust,
            "ts" | "tsx" | "mts" | "cts" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "py" | "pyi" | "pyw" => Language::Python,
            "go" => Language::Go,
            "cpp" | "cxx" | "cc" | "hpp" | "hxx" | "hh" => Language::Cpp,
            "c" | "h" => Language::C,
            "java" => Language::Java,
            "cs" => Language::CSharp,
            "kt" | "kts" => Language::Kotlin,
            "scala" | "sc" => Language::Scala,
            "swift" => Language::Swift,
            "rb" | "rake" | "gemspec" => Language::Ruby,
            "php" | "phtml" => Language::Php,
            "zig" => Language::Zig,
            "dart" => Language::Dart,
            "lua" => Language::Lua,
            "sh" | "bash" | "zsh" => Language::Shell,
            "ex" | "exs" | "erl" | "hrl" => Language::Elixir,
            "sql" => Language::Sql,
            _ => Language::Generic,
        }
    }

    pub fn from_name_or_ext(s: &str) -> Option<Self> {
        let clean = s.trim().to_lowercase();
        let stripped = clean.strip_prefix('.').unwrap_or(&clean);
        match stripped {
            "rs" | "rust" => Some(Language::Rust),
            "ts" | "tsx" | "typescript" => Some(Language::TypeScript),
            "js" | "jsx" | "javascript" | "node" => Some(Language::JavaScript),
            "py" | "pyi" | "python" => Some(Language::Python),
            "go" | "golang" => Some(Language::Go),
            "cpp" | "cxx" | "cc" | "c++" => Some(Language::Cpp),
            "c" | "h" => Some(Language::C),
            "java" => Some(Language::Java),
            "cs" | "csharp" | "c#" => Some(Language::CSharp),
            "kt" | "kts" | "kotlin" => Some(Language::Kotlin),
            "scala" => Some(Language::Scala),
            "swift" => Some(Language::Swift),
            "rb" | "ruby" => Some(Language::Ruby),
            "php" => Some(Language::Php),
            "zig" => Some(Language::Zig),
            "dart" => Some(Language::Dart),
            "lua" => Some(Language::Lua),
            "sh" | "bash" | "zsh" | "shell" => Some(Language::Shell),
            "ex" | "exs" | "elixir" | "erl" | "erlang" => Some(Language::Elixir),
            "sql" => Some(Language::Sql),
            "all" | "*" => None,
            _ => Some(Self::from_extension(stripped)),
        }
    }
}

// ---------------------------------------------------------------------------
// Compiled Regex Rule Set
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct SymbolPattern {
    kind: SymbolKind,
    regex: Regex,
    name_group: usize,
    vis_group: Option<usize>,
    container_group: Option<usize>,
}

struct LanguageRules {
    patterns: Vec<SymbolPattern>,
    container_open: Option<Regex>,
    container_name_group: usize,
}

static RULES: LazyLock<HashMap<Language, LanguageRules>> = LazyLock::new(init_all_rules);

fn get_rules() -> &'static HashMap<Language, LanguageRules> {
    &RULES
}

fn make_pattern(
    kind: SymbolKind,
    pattern: &str,
    name_group: usize,
    vis_group: Option<usize>,
) -> SymbolPattern {
    make_pattern_full(kind, pattern, name_group, vis_group, None)
}

fn make_pattern_full(
    kind: SymbolKind,
    pattern: &str,
    name_group: usize,
    vis_group: Option<usize>,
    container_group: Option<usize>,
) -> SymbolPattern {
    let regex = RegexBuilder::new(pattern)
        .multi_line(false)
        .dot_matches_new_line(false)
        .build()
        .unwrap_or_else(|e| panic!("Failed to compile regex '{}': {}", pattern, e));
    SymbolPattern {
        kind,
        regex,
        name_group,
        vis_group,
        container_group,
    }
}

fn init_all_rules() -> HashMap<Language, LanguageRules> {
    let mut map = HashMap::new();

    // 1. Rust
    let rust_patterns = vec![
        // Functions: [pub] [async] [unsafe] [extern "C"] [const] fn name
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(pub(?:\([^)]+\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern(?:\s+"[^"]+")?\s+)?(?:const\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Structs
        make_pattern(
            SymbolKind::Struct,
            r#"^\s*(pub(?:\([^)]+\))?\s+)?struct\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Enums
        make_pattern(
            SymbolKind::Enum,
            r#"^\s*(pub(?:\([^)]+\))?\s+)?enum\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Traits
        make_pattern(
            SymbolKind::Trait,
            r#"^\s*(pub(?:\([^)]+\))?\s+)?(?:unsafe\s+)?trait\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Type aliases
        make_pattern(
            SymbolKind::TypeAlias,
            r#"^\s*(pub(?:\([^)]+\))?\s+)?type\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Modules
        make_pattern(
            SymbolKind::Module,
            r#"^\s*(pub(?:\([^)]+\))?\s+)?mod\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Macros
        make_pattern(
            SymbolKind::Macro,
            r#"^\s*macro_rules!\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        // Constants & Statics
        make_pattern(
            SymbolKind::Constant,
            r#"^\s*(pub(?:\([^)]+\))?\s+)?(?:const|static)\s+(?:mut\s+)?([A-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
    ];
    let rust_container = Regex::new(
        r#"^\s*impl(?:\s*<[^>]*>)?\s+(?:[a-zA-Z_][a-zA-Z0-9_:<>\s,]*\s+for\s+)?([a-zA-Z_][a-zA-Z0-9_]*)"#,
    )
    .ok();
    map.insert(
        Language::Rust,
        LanguageRules {
            patterns: rust_patterns,
            container_open: rust_container,
            container_name_group: 1,
        },
    );

    // 2. TypeScript / JavaScript
    let ts_patterns = vec![
        // Named functions
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:export\s+(?:default\s+)?)?(?:async\s+)?function\s*\*?\s+([a-zA-Z_$][a-zA-Z0-9_$]*)"#,
            1,
            None,
        ),
        // Arrow functions / function expressions assigned to const/let/var
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(export\s+)?(?:const|let|var)\s+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*(?::\s*[^=]+)?\s*=\s*(?:async\s+)?(?:\([^)]*\)|[a-zA-Z_$][a-zA-Z0-9_$]*)\s*=>"#,
            2,
            Some(1),
        ),
        // Classes
        make_pattern(
            SymbolKind::Class,
            r#"^\s*(export\s+(?:default\s+|abstract\s+)?)?(?:abstract\s+)?class\s+([a-zA-Z_$][a-zA-Z0-9_$]*)"#,
            2,
            Some(1),
        ),
        // Interfaces
        make_pattern(
            SymbolKind::Interface,
            r#"^\s*(export\s+)?interface\s+([a-zA-Z_$][a-zA-Z0-9_$]*)"#,
            2,
            Some(1),
        ),
        // Type aliases
        make_pattern(
            SymbolKind::TypeAlias,
            r#"^\s*(export\s+)?type\s+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*(?:<[^>]+>)?\s*="#,
            2,
            Some(1),
        ),
        // Enums
        make_pattern(
            SymbolKind::Enum,
            r#"^\s*(export\s+(?:const\s+)?)?enum\s+([a-zA-Z_$][a-zA-Z0-9_$]*)"#,
            2,
            Some(1),
        ),
        // Class methods
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:(public|private|protected|static|readonly|override|async)\s+)+([a-zA-Z_$][a-zA-Z0-9_$]*)\s*(?:<[^>]+>)?\s*\("#,
            2,
            Some(1),
        ),
        // Constants / Exports
        make_pattern(
            SymbolKind::Constant,
            r#"^\s*(export\s+)?const\s+([A-Z_][a-zA-Z0-9_]*)\s*(?::\s*[^=]+)?\s*="#,
            2,
            Some(1),
        ),
    ];
    let ts_container = Regex::new(
        r#"^\s*(?:export\s+)?(?:abstract\s+)?(?:class|interface)\s+([a-zA-Z_$][a-zA-Z0-9_$]*)"#,
    )
    .ok();
    map.insert(
        Language::TypeScript,
        LanguageRules {
            patterns: ts_patterns.clone(),
            container_open: ts_container.clone(),
            container_name_group: 1,
        },
    );
    map.insert(
        Language::JavaScript,
        LanguageRules {
            patterns: ts_patterns,
            container_open: ts_container,
            container_name_group: 1,
        },
    );

    // 3. Python
    let py_patterns = vec![
        // Functions and methods: def name(...)
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:async\s+)?def\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        // Classes: class Name(...)
        make_pattern(
            SymbolKind::Class,
            r#"^\s*class\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
    ];
    let py_container = Regex::new(r#"^\s*class\s+([a-zA-Z_][a-zA-Z0-9_]*)"#).ok();
    map.insert(
        Language::Python,
        LanguageRules {
            patterns: py_patterns,
            container_open: py_container,
            container_name_group: 1,
        },
    );

    // 4. Go
    let go_patterns = vec![
        // Methods: func (r *Receiver) MethodName(...)
        make_pattern_full(
            SymbolKind::Function,
            r#"^\s*func\s+\((?:[a-zA-Z_][a-zA-Z0-9_]*\s+)?(?:\*)?([a-zA-Z_][a-zA-Z0-9_]*)\)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            None,
            Some(1),
        ),
        // Functions: func Name(...)
        make_pattern(
            SymbolKind::Function,
            r#"^\s*func\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        // Structs: type Name struct
        make_pattern(
            SymbolKind::Struct,
            r#"^\s*type\s+([a-zA-Z_][a-zA-Z0-9_]*)\s+struct\b"#,
            1,
            None,
        ),
        // Interfaces: type Name interface
        make_pattern(
            SymbolKind::Interface,
            r#"^\s*type\s+([a-zA-Z_][a-zA-Z0-9_]*)\s+interface\b"#,
            1,
            None,
        ),
        // Type aliases
        make_pattern(
            SymbolKind::TypeAlias,
            r#"^\s*type\s+([a-zA-Z_][a-zA-Z0-9_]*)\b"#,
            1,
            None,
        ),
        // Constants: const Name = ...
        make_pattern(
            SymbolKind::Constant,
            r#"^\s*const\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
    ];
    map.insert(
        Language::Go,
        LanguageRules {
            patterns: go_patterns,
            container_open: None,
            container_name_group: 0,
        },
    );

    // 5. C & C++
    let cpp_patterns = vec![
        // Classes
        make_pattern(
            SymbolKind::Class,
            r#"^\s*(?:template\s*<[^>]*>\s*)?class\s+(?:[a-zA-Z_][a-zA-Z0-9_]*_API\s+)?([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        // Structs
        make_pattern(
            SymbolKind::Struct,
            r#"^\s*(?:typedef\s+)?struct\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        // Enums
        make_pattern(
            SymbolKind::Enum,
            r#"^\s*enum\s+(?:class\s+|struct\s+)?([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        // Type aliases (using / typedef)
        make_pattern(
            SymbolKind::TypeAlias,
            r#"^\s*using\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*="#,
            1,
            None,
        ),
        // Macros
        make_pattern(
            SymbolKind::Macro,
            r#"^\s*#\s*define\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        // Functions / Methods (simplified definition matching)
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:(?:inline|static|virtual|explicit|constexpr|consteval|friend)\s+)*(?:[a-zA-Z_][a-zA-Z0-9_:*&<>,]*\s+)+([a-zA-Z_][a-zA-Z0-9_:]*)\s*\([^)]*\)\s*(?:const|noexcept|override|final|\s)*[;{]"#,
            1,
            None,
        ),
    ];
    let cpp_container = Regex::new(r#"^\s*(?:class|struct)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#).ok();
    map.insert(
        Language::Cpp,
        LanguageRules {
            patterns: cpp_patterns.clone(),
            container_open: cpp_container.clone(),
            container_name_group: 1,
        },
    );
    map.insert(
        Language::C,
        LanguageRules {
            patterns: cpp_patterns,
            container_open: cpp_container,
            container_name_group: 1,
        },
    );

    // 6. Java & C#
    let java_patterns = vec![
        // Classes
        make_pattern(
            SymbolKind::Class,
            r#"^\s*(?:(public|protected|private|static|final|abstract|sealed|open|data|internal)\s+)*class\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Records
        make_pattern(
            SymbolKind::Struct,
            r#"^\s*(?:(public|protected|private|static|final)\s+)*record\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Interfaces
        make_pattern(
            SymbolKind::Interface,
            r#"^\s*(?:(public|protected|private|static|abstract|sealed|internal)\s+)*interface\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Enums
        make_pattern(
            SymbolKind::Enum,
            r#"^\s*(?:(public|protected|private|static|internal)\s+)*(?:enum|enum\s+class)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Structs (C#)
        make_pattern(
            SymbolKind::Struct,
            r#"^\s*(?:(public|protected|private|static|readonly|ref|internal)\s+)*struct\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        // Methods
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:(public|protected|private|static|final|abstract|synchronized|override|virtual|async)\s+)+(?:[a-zA-Z0-9_<>,\[\]?]+\s+)+([a-zA-Z_][a-zA-Z0-9_]*)\s*\([^)]*\)"#,
            2,
            Some(1),
        ),
    ];
    let java_container = Regex::new(r#"^\s*(?:(?:public|protected|private)\s+)*(?:class|interface|enum|record|struct)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#).ok();
    map.insert(
        Language::Java,
        LanguageRules {
            patterns: java_patterns.clone(),
            container_open: java_container.clone(),
            container_name_group: 1,
        },
    );
    map.insert(
        Language::CSharp,
        LanguageRules {
            patterns: java_patterns,
            container_open: java_container,
            container_name_group: 1,
        },
    );

    // 7. Kotlin
    let kt_patterns = vec![
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:(public|private|protected|internal|open|abstract|override|inline|suspend)\s+)*fun\s+(?:<[^>]+>\s+)?([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Class,
            r#"^\s*(?:(public|private|protected|internal|open|abstract|final|data|sealed)\s+)*(?:class|object)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Interface,
            r#"^\s*(?:(public|private|protected|internal)\s+)*interface\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Enum,
            r#"^\s*(?:(public|private|protected|internal)\s+)*enum\s+class\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
    ];
    let kt_container =
        Regex::new(r#"^\s*(?:class|object|interface)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#).ok();
    map.insert(
        Language::Kotlin,
        LanguageRules {
            patterns: kt_patterns,
            container_open: kt_container,
            container_name_group: 1,
        },
    );

    // 8. Scala
    let scala_patterns = vec![
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:(override|private|protected|final|implicit)\s+)*def\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Class,
            r#"^\s*(?:(case|abstract|sealed|final)\s+)*(?:class|object)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Trait,
            r#"^\s*(?:(sealed)\s+)*trait\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
    ];
    map.insert(
        Language::Scala,
        LanguageRules {
            patterns: scala_patterns,
            container_open: None,
            container_name_group: 0,
        },
    );

    // 9. Swift
    let swift_patterns = vec![
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:(public|private|fileprivate|internal|open|final|static|class|mutating|nonmutating|async|throws)\s+)*func\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Class,
            r#"^\s*(?:(public|private|fileprivate|internal|open|final)\s+)*class\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Struct,
            r#"^\s*(?:(public|private|fileprivate|internal)\s+)*struct\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Enum,
            r#"^\s*(?:(public|private|fileprivate|internal)\s+)*enum\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Trait,
            r#"^\s*(?:(public|private|fileprivate|internal)\s+)*protocol\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Class,
            r#"^\s*(?:(public|private|fileprivate|internal)\s+)*actor\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::TypeAlias,
            r#"^\s*(?:(public|private|fileprivate|internal)\s+)*typealias\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
    ];
    let swift_container =
        Regex::new(r#"^\s*(?:class|struct|enum|protocol|actor)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#).ok();
    map.insert(
        Language::Swift,
        LanguageRules {
            patterns: swift_patterns,
            container_open: swift_container,
            container_name_group: 1,
        },
    );

    // 10. Ruby
    let ruby_patterns = vec![
        make_pattern(
            SymbolKind::Function,
            r#"^\s*def\s+(?:self\.)?([a-zA-Z_][a-zA-Z0-9_?!]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Class,
            r#"^\s*class\s+([a-zA-Z_][a-zA-Z0-9_:]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Module,
            r#"^\s*module\s+([a-zA-Z_][a-zA-Z0-9_:]*)"#,
            1,
            None,
        ),
    ];
    let ruby_container = Regex::new(r#"^\s*(?:class|module)\s+([a-zA-Z_][a-zA-Z0-9_:]*)"#).ok();
    map.insert(
        Language::Ruby,
        LanguageRules {
            patterns: ruby_patterns,
            container_open: ruby_container,
            container_name_group: 1,
        },
    );

    // 11. PHP
    let php_patterns = vec![
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:(public|protected|private|static|abstract|final)\s+)*function\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Class,
            r#"^\s*(?:(abstract|final|readonly)\s+)*class\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Interface,
            r#"^\s*interface\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Trait,
            r#"^\s*trait\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Enum,
            r#"^\s*enum\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
    ];
    let php_container = Regex::new(r#"^\s*class\s+([a-zA-Z_][a-zA-Z0-9_]*)"#).ok();
    map.insert(
        Language::Php,
        LanguageRules {
            patterns: php_patterns,
            container_open: php_container,
            container_name_group: 1,
        },
    );

    // 12. Zig
    let zig_patterns = vec![
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(pub\s+)?fn\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Struct,
            r#"^\s*(pub\s+)?const\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*=\s*struct\b"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Enum,
            r#"^\s*(pub\s+)?const\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*=\s*enum\b"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::TypeAlias,
            r#"^\s*(pub\s+)?const\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*="#,
            2,
            Some(1),
        ),
    ];
    map.insert(
        Language::Zig,
        LanguageRules {
            patterns: zig_patterns,
            container_open: None,
            container_name_group: 0,
        },
    );

    // 13. Dart
    let dart_patterns = vec![
        make_pattern(
            SymbolKind::Class,
            r#"^\s*(?:(abstract|sealed|base|interface|final|mixin)\s+)*class\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            2,
            Some(1),
        ),
        make_pattern(
            SymbolKind::Enum,
            r#"^\s*enum\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Trait,
            r#"^\s*mixin\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:[a-zA-Z0-9_<>?]+\s+)+([a-zA-Z_][a-zA-Z0-9_]*)\s*\([^)]*\)\s*(?:async\s*)?[{;]"#,
            1,
            None,
        ),
    ];
    map.insert(
        Language::Dart,
        LanguageRules {
            patterns: dart_patterns,
            container_open: None,
            container_name_group: 0,
        },
    );

    // 14. Lua
    let lua_patterns = vec![make_pattern(
        SymbolKind::Function,
        r#"^\s*(?:local\s+)?function\s+([a-zA-Z_][a-zA-Z0-9_.:]*)"#,
        1,
        None,
    )];
    map.insert(
        Language::Lua,
        LanguageRules {
            patterns: lua_patterns,
            container_open: None,
            container_name_group: 0,
        },
    );

    // 15. Shell
    let sh_patterns = vec![
        make_pattern(
            SymbolKind::Function,
            r#"^\s*function\s+([a-zA-Z_][a-zA-Z0-9_-]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Function,
            r#"^\s*([a-zA-Z_][a-zA-Z0-9_-]*)\s*\(\s*\)\s*\{"#,
            1,
            None,
        ),
    ];
    map.insert(
        Language::Shell,
        LanguageRules {
            patterns: sh_patterns,
            container_open: None,
            container_name_group: 0,
        },
    );

    // 16. Elixir
    let ex_patterns = vec![
        make_pattern(
            SymbolKind::Module,
            r#"^\s*defmodule\s+([a-zA-Z_][a-zA-Z0-9_.]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Function,
            r#"^\s*defp?\s+([a-zA-Z_][a-zA-Z0-9_?!]*)"#,
            1,
            None,
        ),
    ];
    map.insert(
        Language::Elixir,
        LanguageRules {
            patterns: ex_patterns,
            container_open: None,
            container_name_group: 0,
        },
    );

    // 17. SQL
    let sql_patterns = vec![
        make_pattern(
            SymbolKind::Struct,
            r#"(?i)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?(?:[`"\[]?([a-zA-Z0-9_]+)[`"\]]?)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Function,
            r#"(?i)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?(?:FUNCTION|PROCEDURE)\s+(?:[`"\[]?([a-zA-Z0-9_]+)[`"\]]?)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Struct,
            r#"(?i)^\s*CREATE\s+(?:OR\s+REPLACE\s+)?VIEW\s+(?:[`"\[]?([a-zA-Z0-9_]+)[`"\]]?)"#,
            1,
            None,
        ),
    ];
    map.insert(
        Language::Sql,
        LanguageRules {
            patterns: sql_patterns,
            container_open: None,
            container_name_group: 0,
        },
    );

    // 18. Generic Fallback
    let generic_patterns = vec![
        make_pattern(
            SymbolKind::Function,
            r#"^\s*(?:pub\s+|export\s+)?(?:def|fn|func|function)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Class,
            r#"^\s*(?:pub\s+|export\s+)?class\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Struct,
            r#"^\s*(?:pub\s+|export\s+)?struct\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Interface,
            r#"^\s*(?:pub\s+|export\s+)?(?:interface|trait)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
        make_pattern(
            SymbolKind::Enum,
            r#"^\s*(?:pub\s+|export\s+)?enum\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
            1,
            None,
        ),
    ];
    map.insert(
        Language::Generic,
        LanguageRules {
            patterns: generic_patterns,
            container_open: None,
            container_name_group: 0,
        },
    );

    map
}

// ---------------------------------------------------------------------------
// Scanner Implementation
// ---------------------------------------------------------------------------

/// Fast scanner that inspects file content and extracts symbols line by line.
#[derive(Debug, Clone, Default)]
pub struct SymbolScanner;

impl SymbolScanner {
    pub fn new() -> Self {
        Self
    }

    /// Scan a single file's text content and return extracted symbols.
    pub fn scan_content(&self, content: &str, file_path: &str) -> Vec<Symbol> {
        let path_obj = Path::new(file_path);
        let ext = path_obj.extension().and_then(|s| s.to_str()).unwrap_or("");
        let lang = Language::from_extension(ext);
        let all_rules = get_rules();

        let rules = all_rules
            .get(&lang)
            .or_else(|| all_rules.get(&Language::Generic));

        let Some(rules) = rules else {
            return Vec::new();
        };

        let mut symbols = Vec::new();
        let mut current_container: Option<String> = None;
        let mut current_indent_or_brace: usize = 0;
        let mut recent_doc_lines: Vec<String> = Vec::new();

        let is_python = lang == Language::Python;
        let mut py_class_indent: Option<usize> = None;

        for (line_idx, line) in content.lines().enumerate() {
            let line_num = line_idx + 1;
            let trimmed = line.trim();

            if trimmed.is_empty() {
                recent_doc_lines.clear();
                continue;
            }

            // Collect doc comments
            if trimmed.starts_with("///") || trimmed.starts_with("//!") {
                let doc = trimmed
                    .trim_start_matches("///")
                    .trim_start_matches("//!")
                    .trim();
                recent_doc_lines.push(doc.to_string());
                continue;
            } else if trimmed.starts_with("/**")
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
            {
                let doc = trimmed
                    .trim_start_matches("/**")
                    .trim_start_matches("/*")
                    .trim_start_matches('*')
                    .trim();
                if !doc.is_empty() && doc != "/" {
                    recent_doc_lines.push(doc.to_string());
                }
                continue;
            } else if (is_python || lang == Language::Ruby || lang == Language::Shell)
                && trimmed.starts_with('#')
            {
                let doc = trimmed.trim_start_matches('#').trim();
                recent_doc_lines.push(doc.to_string());
                continue;
            } else if trimmed.starts_with("//") {
                recent_doc_lines.clear();
                continue;
            }

            // Container scope tracking for Python indentation
            if is_python {
                let indent = line.len() - line.trim_start().len();
                if let Some(c_indent) = py_class_indent {
                    if indent <= c_indent
                        && !trimmed.starts_with("class ")
                        && !trimmed.starts_with('@')
                        && !trimmed.starts_with('#')
                    {
                        current_container = None;
                        py_class_indent = None;
                    }
                }
            }

            // Container scope tracking for curly brace languages
            if let Some(container_rx) = &rules.container_open {
                if let Some(caps) = container_rx.captures(line) {
                    if let Some(mat) = caps.get(rules.container_name_group) {
                        current_container = Some(mat.as_str().to_string());
                        if is_python {
                            py_class_indent = Some(line.len() - line.trim_start().len());
                        }
                    }
                }
            }

            // Track brace depth across lines in brace languages
            if !is_python {
                let open_count = trimmed.chars().filter(|&c| c == '{').count();
                let close_count = trimmed.chars().filter(|&c| c == '}').count();
                if open_count > close_count {
                    current_indent_or_brace += open_count - close_count;
                } else if close_count > open_count {
                    let diff = close_count - open_count;
                    if diff >= current_indent_or_brace {
                        current_indent_or_brace = 0;
                        current_container = None;
                    } else {
                        current_indent_or_brace -= diff;
                    }
                }
            }

            // Check each pattern for matches
            for pat in &rules.patterns {
                if let Some(caps) = pat.regex.captures(line) {
                    let Some(name_match) = caps.get(pat.name_group) else {
                        continue;
                    };
                    let symbol_name = name_match.as_str().trim();
                    if symbol_name.is_empty() {
                        continue;
                    }

                    let col = name_match.start() + 1;
                    let visibility = pat.vis_group.and_then(|idx| {
                        caps.get(idx)
                            .map(|m| m.as_str().trim().to_string())
                            .filter(|s| !s.is_empty())
                    });

                    let signature = trimmed.to_string();
                    let doc_comment = if !recent_doc_lines.is_empty() {
                        Some(recent_doc_lines.join(" "))
                    } else {
                        None
                    };

                    symbols.push(Symbol {
                        name: symbol_name.to_string(),
                        kind: pat.kind,
                        path: file_path.to_string(),
                        line: line_num,
                        column: col,
                        signature,
                        language: lang.as_str().to_string(),
                        visibility,
                        container: pat
                            .container_group
                            .and_then(|idx| {
                                caps.get(idx)
                                    .map(|m| m.as_str().trim().to_string())
                                    .filter(|s| !s.is_empty())
                            })
                            .or_else(|| current_container.clone()),
                        doc_comment,
                    });

                    // Break after first matching pattern on this line
                    break;
                }
            }

            recent_doc_lines.clear();
        }

        symbols
    }
}

// ---------------------------------------------------------------------------
// Query and Search Engine
// ---------------------------------------------------------------------------

/// Query parameters for symbol search.
#[derive(Debug, Clone, Default)]
pub struct SymbolQuery {
    pub query: Option<String>,
    pub kind: Option<String>,
    pub path: Option<String>,
    pub language: Option<String>,
    pub exact: bool,
    pub case_sensitive: bool,
    pub include_docs: bool,
    pub hidden: bool,
    pub max_results: usize,
}

impl SymbolQuery {
    pub fn matches(&self, symbol: &Symbol) -> bool {
        // 1. Kind filter
        if let Some(k) = &self.kind {
            if !symbol.kind.matches_filter(k) {
                return false;
            }
        }

        // 2. Language filter
        if let Some(lang_filter) = &self.language {
            let target_lang = Language::from_name_or_ext(lang_filter);
            if let Some(tl) = target_lang {
                if symbol.language != tl.as_str() {
                    return false;
                }
            } else if !symbol.language.eq_ignore_ascii_case(lang_filter) {
                return false;
            }
        }

        // 3. Name / Query filter
        if let Some(q) = &self.query {
            let q_trimmed = q.trim();
            if !q_trimmed.is_empty() {
                if self.exact {
                    if self.case_sensitive {
                        if symbol.name != q_trimmed && symbol.qualified_name() != q_trimmed {
                            return false;
                        }
                    } else if !symbol.name.eq_ignore_ascii_case(q_trimmed)
                        && !symbol.qualified_name().eq_ignore_ascii_case(q_trimmed)
                    {
                        return false;
                    }
                } else if q_trimmed.contains('*') {
                    // Simple wildcard match
                    let regex_pattern =
                        format!("^{}$", regex::escape(q_trimmed).replace(r"\*", ".*"));
                    if let Ok(rx) = RegexBuilder::new(&regex_pattern)
                        .case_insensitive(!self.case_sensitive)
                        .build()
                    {
                        if !rx.is_match(&symbol.name) && !rx.is_match(&symbol.qualified_name()) {
                            return false;
                        }
                    }
                } else if self.case_sensitive {
                    if !symbol.name.contains(q_trimmed)
                        && !symbol.qualified_name().contains(q_trimmed)
                    {
                        return false;
                    }
                } else {
                    let q_lower = q_trimmed.to_lowercase();
                    let name_lower = symbol.name.to_lowercase();
                    let qualified_lower = symbol.qualified_name().to_lowercase();
                    if !name_lower.contains(&q_lower) && !qualified_lower.contains(&q_lower) {
                        return false;
                    }
                }
            }
        }

        true
    }
}

// ---------------------------------------------------------------------------
// Workspace Traversal
// ---------------------------------------------------------------------------

/// Check if a byte buffer appears to be binary.
fn is_binary_buffer(bytes: &[u8]) -> bool {
    bytes.iter().take(4096).any(|&b| b == 0)
}

/// Search workspace files for code symbols.
pub fn scan_workspace(
    target_path: &Path,
    workspace_root: &Path,
    query: &SymbolQuery,
) -> anyhow::Result<(Vec<Symbol>, usize)> {
    let scanner = SymbolScanner::new();
    let mut symbols = Vec::new();
    let mut files_scanned = 0;

    if target_path.is_file() {
        // Single file scan
        if let Ok(bytes) = std::fs::read(target_path) {
            if !is_binary_buffer(&bytes) {
                if let Ok(content) = std::str::from_utf8(&bytes) {
                    files_scanned += 1;
                    let rel_path = target_path
                        .strip_prefix(workspace_root)
                        .unwrap_or(target_path)
                        .to_string_lossy()
                        .to_string();

                    for sym in scanner.scan_content(content, &rel_path) {
                        if query.matches(&sym) {
                            symbols.push(sym);
                            if symbols.len() >= query.max_results {
                                break;
                            }
                        }
                    }
                }
            }
        }
        return Ok((symbols, files_scanned));
    }

    // Directory walk respecting .gitignore
    let mut builder = WalkBuilder::new(target_path);
    builder
        .hidden(!query.hidden)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .parents(true);

    for entry_res in builder.build() {
        let entry = match entry_res {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Skip files > 5MB to avoid memory pressure
        if let Ok(meta) = entry.metadata() {
            if meta.len() > 5 * 1024 * 1024 {
                continue;
            }
        }

        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };

        if is_binary_buffer(&bytes) {
            continue;
        }

        let Ok(content) = std::str::from_utf8(&bytes) else {
            continue;
        };

        files_scanned += 1;
        let rel_path = path
            .strip_prefix(workspace_root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        for sym in scanner.scan_content(content, &rel_path) {
            if query.matches(&sym) {
                symbols.push(sym);
                if symbols.len() >= query.max_results {
                    return Ok((symbols, files_scanned));
                }
            }
        }
    }

    Ok((symbols, files_scanned))
}

// ---------------------------------------------------------------------------
// Formatting Helpers
// ---------------------------------------------------------------------------

fn format_symbols_text(symbols: &[Symbol], query: &SymbolQuery, files_scanned: usize) -> String {
    if symbols.is_empty() {
        let q_str = query.query.as_deref().unwrap_or("*");
        let k_str = query.kind.as_deref().unwrap_or("all");
        return format!(
            "No symbols found matching '{}' (kind: {}, scanned {} files).",
            q_str, k_str, files_scanned
        );
    }

    let mut output = String::new();
    let q_display = query.query.as_deref().unwrap_or("all");
    output.push_str(&format!(
        "Found {} symbol(s) matching '{}' in workspace:\n\n",
        symbols.len(),
        q_display
    ));

    // Group symbols by file path
    let mut by_file: HashMap<&str, Vec<&Symbol>> = HashMap::new();
    let mut file_order: Vec<&str> = Vec::new();

    for sym in symbols {
        let entry = by_file.entry(&sym.path).or_insert_with(|| {
            file_order.push(&sym.path);
            Vec::new()
        });
        entry.push(sym);
    }

    for file in file_order {
        let file_symbols = &by_file[file];
        output.push_str(&format!("{}:\n", file));

        for sym in file_symbols {
            let qualified = sym.qualified_name();
            let kind_tag = format!("[{}]", sym.kind.as_str());
            let vis = sym
                .visibility
                .as_deref()
                .map(|v| format!("{} ", v))
                .unwrap_or_default();

            output.push_str(&format!(
                "  Line {:<5} {:<12} {}{}\n",
                sym.line, kind_tag, vis, qualified
            ));

            if query.include_docs {
                if let Some(doc) = &sym.doc_comment {
                    output.push_str(&format!("         Doc: {}\n", doc));
                }
            }
        }
        output.push('\n');
    }

    output.trim_end().to_string()
}

fn format_symbols_signatures(symbols: &[Symbol]) -> String {
    if symbols.is_empty() {
        return "No symbols found.".to_string();
    }

    let mut lines = Vec::with_capacity(symbols.len());
    for s in symbols {
        lines.push(format!(
            "{}:{}:{}: [{}] {}",
            s.path,
            s.line,
            s.column,
            s.kind.as_str(),
            s.signature
        ));
    }
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// SymbolsTool
// ---------------------------------------------------------------------------

/// Tool for looking up functions, structs, classes, traits, enums, and other symbols.
#[derive(Default, Debug, Clone)]
pub struct SymbolsTool;

impl SymbolsTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SymbolsTool {
    fn name(&self) -> &str {
        "symbols"
    }

    fn description(&self) -> &str {
        "Fast regex-based symbol and declaration scanner (functions, structs, classes, traits, enums, etc.) across workspace files."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Symbol name or pattern to search for (e.g. 'execute', 'Tool*', 'GrepTool'). Optional, returns all symbols if omitted."
                },
                "kind": {
                    "type": "string",
                    "description": "Filter by symbol kind: 'function', 'struct', 'class', 'trait', 'interface', 'enum', 'type', 'module', 'const', 'macro', 'variable', or 'all' (optional)."
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file path to search within (optional, defaults to workspace root)."
                },
                "language": {
                    "type": "string",
                    "description": "Filter by programming language or file extension (e.g. 'rust', 'ts', 'py', 'go', 'rs') (optional)."
                },
                "exact": {
                    "type": "boolean",
                    "description": "Whether to require an exact symbol name match (default: false)."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Whether symbol name search is case-sensitive (default: false)."
                },
                "include_docs": {
                    "type": "boolean",
                    "description": "Whether to include doc comments in symbol details (default: false)."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of symbols to return (default: 100)."
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "signatures", "json"],
                    "description": "Output format: 'text' (default, grouped by file), 'signatures' (single-line declarations), or 'json' (raw symbol objects)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let query_str = args
            .get("query")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("name").and_then(|v| v.as_str()))
            .or_else(|| args.get("pattern").and_then(|v| v.as_str()))
            .map(|s| s.to_string());

        let kind = args
            .get("kind")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("type").and_then(|v| v.as_str()))
            .map(|s| s.to_string());

        let language = args
            .get("language")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("lang").and_then(|v| v.as_str()))
            .or_else(|| args.get("ext").and_then(|v| v.as_str()))
            .map(|s| s.to_string());

        let exact = args.get("exact").and_then(|v| v.as_bool()).unwrap_or(false);

        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let include_docs = args
            .get("include_docs")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(100);

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| args.get("dir").and_then(|v| v.as_str()));

        let target_path = match path_str {
            Some(p) => resolve_path(p, &ctx.cwd),
            None => ctx.cwd.clone(),
        };

        if !target_path.exists() {
            anyhow::bail!("Path not found: '{}'", target_path.display());
        }

        let hidden = args
            .get("hidden")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let query = SymbolQuery {
            query: query_str,
            kind,
            path: path_str.map(|s| s.to_string()),
            language,
            exact,
            case_sensitive,
            include_docs,
            hidden,
            max_results,
        };

        let cwd = ctx.cwd.clone();
        let target = target_path.clone();
        let query_clone = query.clone();

        // Run file traversal and parsing in blocking threadpool
        let (symbols, files_scanned) =
            tokio::task::spawn_blocking(move || scan_workspace(&target, &cwd, &query_clone))
                .await
                .map_err(|e| anyhow::anyhow!("Symbol scanning task failed: {e}"))??;

        match format {
            "json" => Ok(serde_json::to_string_pretty(&symbols)?),
            "signatures" | "sig" => Ok(format_symbols_signatures(&symbols)),
            _ => Ok(format_symbols_text(&symbols, &query, files_scanned)),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_rust_symbol_extraction() {
        let code = r#"
/// A fast tool registry.
pub struct ToolRegistry {
    tools: HashMap<String, DynTool>,
}

pub enum ToolKind {
    Builtin,
    Custom,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
}

impl ToolRegistry {
    /// Create a new registry.
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub async fn execute(&self, name: &str) -> anyhow::Result<String> {
        Ok(String::new())
    }
}

pub type ToolMap = HashMap<String, DynTool>;
pub const MAX_TOOLS: usize = 128;
macro_rules! register_tool {
    ($reg:expr, $tool:expr) => {};
}
"#;

        let scanner = SymbolScanner::new();
        let symbols = scanner.scan_content(code, "src/registry.rs");

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"ToolRegistry"));
        assert!(names.contains(&"ToolKind"));
        assert!(names.contains(&"Tool"));
        assert!(names.contains(&"new"));
        assert!(names.contains(&"execute"));
        assert!(names.contains(&"ToolMap"));
        assert!(names.contains(&"MAX_TOOLS"));
        assert!(names.contains(&"register_tool"));

        // Check container scope
        let new_sym = symbols.iter().find(|s| s.name == "new").unwrap();
        assert_eq!(new_sym.container.as_deref(), Some("ToolRegistry"));
        assert_eq!(new_sym.qualified_name(), "ToolRegistry::new");
        assert_eq!(
            new_sym.doc_comment.as_deref(),
            Some("Create a new registry.")
        );
    }

    #[test]
    fn test_typescript_symbol_extraction() {
        let code = r#"
export interface UserConfig {
    name: string;
    age: number;
}

export type StringOrNumber = string | number;

export enum Status {
    Active,
    Inactive,
}

export class UserManager {
    public constructor() {}

    public async getUser(id: string): Promise<User> {
        return null;
    }
}

export const fetchUsers = async () => {
    return [];
};

export function validateInput(input: string): boolean {
    return true;
}
"#;

        let scanner = SymbolScanner::new();
        let symbols = scanner.scan_content(code, "src/user.ts");

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"UserConfig"));
        assert!(names.contains(&"StringOrNumber"));
        assert!(names.contains(&"Status"));
        assert!(names.contains(&"UserManager"));
        assert!(names.contains(&"getUser"));
        assert!(names.contains(&"fetchUsers"));
        assert!(names.contains(&"validateInput"));

        let get_user_sym = symbols.iter().find(|s| s.name == "getUser").unwrap();
        assert_eq!(get_user_sym.kind, SymbolKind::Function);
        assert_eq!(get_user_sym.container.as_deref(), Some("UserManager"));
        assert_eq!(get_user_sym.qualified_name(), "UserManager.getUser");
    }

    #[test]
    fn test_python_symbol_extraction() {
        let code = r#"
class DataProcessor:
    """Processes datasets."""

    def __init__(self, config):
        self.config = config

    async def process_batch(self, batch):
        pass

def standalone_helper(x, y):
    return x + y
"#;

        let scanner = SymbolScanner::new();
        let symbols = scanner.scan_content(code, "app/processor.py");

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"DataProcessor"));
        assert!(names.contains(&"__init__"));
        assert!(names.contains(&"process_batch"));
        assert!(names.contains(&"standalone_helper"));

        let proc_sym = symbols.iter().find(|s| s.name == "process_batch").unwrap();
        assert_eq!(proc_sym.container.as_deref(), Some("DataProcessor"));
        assert_eq!(proc_sym.qualified_name(), "DataProcessor.process_batch");
    }

    #[test]
    fn test_go_symbol_extraction() {
        let code = r#"
package service

type Server struct {
    port int
}

type Handler interface {
    Handle()
}

func (s *Server) Start() error {
    return nil
}

func NewServer(port int) *Server {
    return &Server{port: port}
}
"#;

        let scanner = SymbolScanner::new();
        let symbols = scanner.scan_content(code, "pkg/service/server.go");

        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Server"));
        assert!(names.contains(&"Handler"));
        assert!(names.contains(&"Start"));
        assert!(names.contains(&"NewServer"));

        let struct_sym = symbols.iter().find(|s| s.name == "Server").unwrap();
        assert_eq!(struct_sym.kind, SymbolKind::Struct);

        let iface_sym = symbols.iter().find(|s| s.name == "Handler").unwrap();
        assert_eq!(iface_sym.kind, SymbolKind::Interface);

        let start_sym = symbols.iter().find(|s| s.name == "Start").unwrap();
        assert_eq!(start_sym.container.as_deref(), Some("Server"));
        assert_eq!(start_sym.qualified_name(), "Server.Start");
    }

    #[test]
    fn test_cpp_and_java_symbol_extraction() {
        let cpp_code = r#"
class Engine {
public:
    void Start();
    virtual bool IsRunning() const;
};

struct Config {
    int timeout;
};

#define MAX_BUFFER_SIZE 4096
"#;
        let scanner = SymbolScanner::new();
        let cpp_symbols = scanner.scan_content(cpp_code, "src/engine.hpp");
        let cpp_names: Vec<&str> = cpp_symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(cpp_names.contains(&"Engine"));
        assert!(cpp_names.contains(&"Config"));
        assert!(cpp_names.contains(&"MAX_BUFFER_SIZE"));

        let java_code = r#"
public class OrderService {
    public record OrderItem(String id, double price) {}

    public interface OrderCallback {
        void onComplete();
    }

    public void processOrder(String orderId) {}
}
"#;
        let java_symbols = scanner.scan_content(java_code, "src/OrderService.java");
        let java_names: Vec<&str> = java_symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(java_names.contains(&"OrderService"));
        assert!(java_names.contains(&"OrderItem"));
        assert!(java_names.contains(&"OrderCallback"));
        assert!(java_names.contains(&"processOrder"));
    }

    #[test]
    fn test_symbol_query_matching() {
        let sym = Symbol {
            name: "GrepTool".to_string(),
            kind: SymbolKind::Struct,
            path: "src/tools/grep.rs".to_string(),
            line: 39,
            column: 12,
            signature: "pub struct GrepTool;".to_string(),
            language: "rust".to_string(),
            visibility: Some("pub".to_string()),
            container: None,
            doc_comment: None,
        };

        // Match all
        let q_all = SymbolQuery::default();
        assert!(q_all.matches(&sym));

        // Match substring
        let q_sub = SymbolQuery {
            query: Some("Grep".to_string()),
            ..Default::default()
        };
        assert!(q_sub.matches(&sym));

        // Match case insensitive
        let q_case_ins = SymbolQuery {
            query: Some("grep".to_string()),
            case_sensitive: false,
            ..Default::default()
        };
        assert!(q_case_ins.matches(&sym));

        // Case sensitive fail
        let q_case_sens = SymbolQuery {
            query: Some("grep".to_string()),
            case_sensitive: true,
            ..Default::default()
        };
        assert!(!q_case_sens.matches(&sym));

        // Wildcard match
        let q_wildcard = SymbolQuery {
            query: Some("Grep*".to_string()),
            ..Default::default()
        };
        assert!(q_wildcard.matches(&sym));

        // Kind filter match
        let q_kind = SymbolQuery {
            kind: Some("struct".to_string()),
            ..Default::default()
        };
        assert!(q_kind.matches(&sym));

        // Kind filter mismatch
        let q_kind_fn = SymbolQuery {
            kind: Some("function".to_string()),
            ..Default::default()
        };
        assert!(!q_kind_fn.matches(&sym));

        // Language filter match
        let q_lang = SymbolQuery {
            language: Some("rust".to_string()),
            ..Default::default()
        };
        assert!(q_lang.matches(&sym));

        // Language filter mismatch
        let q_lang_py = SymbolQuery {
            language: Some("python".to_string()),
            ..Default::default()
        };
        assert!(!q_lang_py.matches(&sym));
    }

    #[tokio::test]
    async fn test_symbols_tool_execution() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let rs_file = root.join("calculator.rs");
        fs::write(
            &rs_file,
            b"pub struct Calculator;\n\nimpl Calculator {\n    pub fn add(a: i32, b: i32) -> i32 { a + b }\n}\n",
        )
        .unwrap();

        let py_file = root.join("script.py");
        fs::write(
            &py_file,
            b"class MathUtil:\n    def multiply(x, y):\n        return x * y\n",
        )
        .unwrap();

        let tool = SymbolsTool::new();
        let ctx = ToolContext {
            cwd: root.to_path_buf(),
            env: std::collections::HashMap::new(),
        };

        // Query all symbols
        let res = tool.execute(json!({}), &ctx).await.unwrap();
        assert!(res.contains("Calculator"));
        assert!(res.contains("add"));
        assert!(res.contains("MathUtil"));
        assert!(res.contains("multiply"));

        // Query specific symbol name
        let res_calc = tool
            .execute(json!({ "query": "Calculator" }), &ctx)
            .await
            .unwrap();
        assert!(res_calc.contains("Calculator"));
        assert!(!res_calc.contains("MathUtil"));

        // Query by kind
        let res_fns = tool
            .execute(json!({ "kind": "function" }), &ctx)
            .await
            .unwrap();
        assert!(res_fns.contains("add"));
        assert!(res_fns.contains("multiply"));
        assert!(!res_fns.contains("[struct]"));

        // Query JSON format
        let res_json = tool
            .execute(json!({ "format": "json" }), &ctx)
            .await
            .unwrap();
        let parsed: Vec<Symbol> = serde_json::from_str(&res_json).unwrap();
        assert!(parsed.len() >= 4);

        // Signatures format
        let res_sig = tool
            .execute(json!({ "format": "signatures" }), &ctx)
            .await
            .unwrap();
        assert!(res_sig.contains("pub fn add"));
    }
}
