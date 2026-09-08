//! Comprehensive unit and integration tests for `VectorSearch` semantic codebase search.
//!
//! Validates:
//! 1. Subword tokenization (camelCase, snake_case, punctuation, lowercasing).
//! 2. Code chunking (line splitting, 5-line overlap, 1-based line bounds, symbol extraction).
//! 3. BM25 statistical relevance ranking (term frequency, IDF weighting, doc length normalization).
//! 4. TF-IDF vector cosine similarity (orthogonal, identical, and partial matches).
//! 5. Multi-chunk semantic query search accuracy across realistic code domains.
//! 6. `VectorSearchTool` metadata, JSON schema, and execution against workspace files.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use fusion::tools::types::{Tool, ToolContext};
use fusion::tools::vector_search::{
    cosine_similarity, extract_symbol_name, tokenize, Chunker, CodeChunk, LexicalVectorIndex,
    SearchResult, VectorSearchTool, DEFAULT_BM25_B, DEFAULT_BM25_K1,
};
use serde_json::json;

// ============================================================================
// 1. Tokenizer Tests
// ============================================================================

#[test]
fn test_tokenizer_camel_case_variations() {
    // Standard camelCase
    assert_eq!(tokenize("camelCase"), vec!["camel", "case"]);
    assert_eq!(tokenize("getUserById"), vec!["get", "user", "by", "id"]);

    // PascalCase
    assert_eq!(
        tokenize("VectorSearchTool"),
        vec!["vector", "search", "tool"]
    );
    assert_eq!(
        tokenize("LexicalVectorIndex"),
        vec!["lexical", "vector", "index"]
    );

    // Consecutive uppercase (acronyms) followed by lowercase
    assert_eq!(tokenize("XMLParser"), vec!["xml", "parser"]);
    assert_eq!(
        tokenize("parseHTMLContent"),
        vec!["parse", "html", "content"]
    );
    assert_eq!(tokenize("XMLHttpRequest"), vec!["xml", "http", "request"]);
    assert_eq!(tokenize("ASTNode"), vec!["ast", "node"]);

    // Digits adjacent to uppercase
    assert_eq!(tokenize("v2Api"), vec!["v2", "api"]);
    assert_eq!(tokenize("sha256Hash"), vec!["sha256", "hash"]);
}

#[test]
fn test_tokenizer_snake_case_variations() {
    // Standard snake_case
    assert_eq!(tokenize("snake_case"), vec!["snake", "case"]);
    assert_eq!(tokenize("my_variable_name"), vec!["my", "variable", "name"]);
    assert_eq!(
        tokenize("user_id_generator"),
        vec!["user", "id", "generator"]
    );

    // SCREAMING_SNAKE_CASE
    assert_eq!(tokenize("MAX_BUFFER_SIZE"), vec!["max", "buffer", "size"]);
    assert_eq!(tokenize("DEFAULT_BM25_K1"), vec!["default", "bm25", "k1"]);

    // Leading and trailing underscores
    assert_eq!(tokenize("__init__"), vec!["init"]);
    assert_eq!(tokenize("_private_field_"), vec!["private", "field"]);
}

#[test]
fn test_tokenizer_mixed_and_punctuation() {
    // Mixed camelCase and snake_case
    assert_eq!(
        tokenize("my_variableName_withCamel"),
        vec!["my", "variable", "name", "with", "camel"]
    );

    // Punctuation and symbols
    assert_eq!(
        tokenize("foo.bar(baz, qux);"),
        vec!["foo", "bar", "baz", "qux"]
    );
    assert_eq!(
        tokenize("let x: Option<String> = None;"),
        vec!["let", "x", "option", "string", "none"]
    );
    assert_eq!(
        tokenize("pub async fn execute(&self, args: Value) -> Result<String>"),
        vec!["pub", "async", "fn", "execute", "self", "args", "value", "result", "string"]
    );

    // Kebab-case
    assert_eq!(tokenize("kebab-case-name"), vec!["kebab", "case", "name"]);

    // Whitespace handling
    assert!(tokenize("   \n\t  ").is_empty());
    assert!(tokenize("").is_empty());
}

// ============================================================================
// 2. Chunker Tests
// ============================================================================

#[test]
fn test_chunker_small_file_single_chunk() {
    let chunker = Chunker::new().with_chunk_size(40).with_overlap(5);
    let code = "fn main() {\n    println!(\"Hello World!\");\n}\n";
    let chunks = chunker.chunk_text(Path::new("src/main.rs"), code);

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].start_line, 1);
    assert_eq!(chunks[0].end_line, 3);
    assert_eq!(chunks[0].file_path, PathBuf::from("src/main.rs"));
    assert_eq!(chunks[0].symbol_name, Some("main".to_string()));
    assert!(chunks[0].tokens.contains(&"main".to_string()));
    assert!(chunks[0].tokens.contains(&"println".to_string()));
}

#[test]
fn test_chunker_large_file_with_overlap() {
    let chunker = Chunker::new().with_chunk_size(40).with_overlap(5);
    // Create 100 lines: step is 40 - 5 = 35
    let lines: Vec<String> = (1..=100)
        .map(|i| format!("let var_{} = {};", i, i))
        .collect();
    let content = lines.join("\n");

    let chunks = chunker.chunk_text(Path::new("test.rs"), &content);
    // Chunk 0: lines 1..=40
    // Chunk 1: lines 36..=75
    // Chunk 2: lines 71..=100
    assert_eq!(chunks.len(), 3);

    assert_eq!(chunks[0].start_line, 1);
    assert_eq!(chunks[0].end_line, 40);

    assert_eq!(chunks[1].start_line, 36);
    assert_eq!(chunks[1].end_line, 75);

    assert_eq!(chunks[2].start_line, 71);
    assert_eq!(chunks[2].end_line, 100);

    // Overlap verification: line 36 is present in both chunk 0 and chunk 1
    assert!(chunks[0].content.contains("let var_36 = 36;"));
    assert!(chunks[1].content.contains("let var_36 = 36;"));

    // Line 71 is present in both chunk 1 and chunk 2
    assert!(chunks[1].content.contains("let var_71 = 71;"));
    assert!(chunks[2].content.contains("let var_71 = 71;"));
}

#[test]
fn test_chunker_symbol_extraction() {
    // Rust function
    let rust_fn = vec!["// comments", "pub fn calculate_hash(data: &[u8]) -> u64 {"];
    assert_eq!(
        extract_symbol_name(&rust_fn),
        Some("calculate_hash".to_string())
    );

    // Rust struct
    let rust_struct = vec!["#[derive(Debug)]", "pub struct ConnectionPool {"];
    assert_eq!(
        extract_symbol_name(&rust_struct),
        Some("ConnectionPool".to_string())
    );

    // Python function
    let py_def = vec!["# Python handler", "def process_request(req):"];
    assert_eq!(
        extract_symbol_name(&py_def),
        Some("process_request".to_string())
    );

    // Go method
    let go_func = vec!["func (s *Server) ListenAndServe() error {"];
    assert_eq!(
        extract_symbol_name(&go_func),
        Some("ListenAndServe".to_string())
    );

    // TypeScript class
    let ts_class = vec!["export class SemanticIndex {"];
    assert_eq!(
        extract_symbol_name(&ts_class),
        Some("SemanticIndex".to_string())
    );

    // No symbol found
    let no_sym = vec!["let a = 10;", "let b = 20;"];
    assert_eq!(extract_symbol_name(&no_sym), None);
}

// ============================================================================
// 3. BM25 Ranking Tests
// ============================================================================

#[test]
fn test_bm25_term_frequency_ranking() {
    // Document A: mentions "deadlock" multiple times
    let chunk_a = CodeChunk::new(
        PathBuf::from("deadlock_heavy.rs"),
        1,
        20,
        "deadlock deadlock deadlock deadlock in mutex lock".to_string(),
        Some("check_deadlock".to_string()),
        tokenize("deadlock deadlock deadlock deadlock in mutex lock"),
    );

    // Document B: mentions "deadlock" once
    let chunk_b = CodeChunk::new(
        PathBuf::from("deadlock_light.rs"),
        1,
        20,
        "prevent deadlock using timeout on channel".to_string(),
        Some("timeout_handler".to_string()),
        tokenize("prevent deadlock using timeout on channel"),
    );

    // Document C: does not mention "deadlock"
    let chunk_c = CodeChunk::new(
        PathBuf::from("unrelated.rs"),
        1,
        20,
        "file buffer reader allocation buffer".to_string(),
        Some("read_file".to_string()),
        tokenize("file buffer reader allocation buffer"),
    );

    let index = LexicalVectorIndex::build_from_chunks(vec![chunk_a, chunk_b, chunk_c]);
    let results = index.search_bm25("deadlock", 10);

    assert_eq!(results.len(), 2, "Only matching chunks should be returned");
    assert_eq!(
        results[0].chunk.file_path,
        PathBuf::from("deadlock_heavy.rs")
    );
    assert_eq!(
        results[1].chunk.file_path,
        PathBuf::from("deadlock_light.rs")
    );
    assert!(
        results[0].score > results[1].score,
        "Document with higher term frequency should rank higher"
    );
}

#[test]
fn test_bm25_idf_weighting() {
    // "common" appears in all 3 documents
    // "rare_term" appears only in Document 1
    let chunk1 = CodeChunk::new(
        PathBuf::from("doc1.rs"),
        1,
        10,
        "common rare_term feature".to_string(),
        None,
        tokenize("common rare_term feature"),
    );
    let chunk2 = CodeChunk::new(
        PathBuf::from("doc2.rs"),
        1,
        10,
        "common system runner".to_string(),
        None,
        tokenize("common system runner"),
    );
    let chunk3 = CodeChunk::new(
        PathBuf::from("doc3.rs"),
        1,
        10,
        "common network worker".to_string(),
        None,
        tokenize("common network worker"),
    );

    let index = LexicalVectorIndex::build_from_chunks(vec![chunk1, chunk2, chunk3]);

    let idf_rare = index.idf("rare_term");
    let idf_common = index.idf("common");

    assert!(
        idf_rare > idf_common,
        "Rare term must have significantly higher IDF than common term (rare: {}, common: {})",
        idf_rare,
        idf_common
    );
}

#[test]
fn test_bm25_document_length_normalization() {
    // Short document containing "quantum"
    let short_doc = CodeChunk::new(
        PathBuf::from("short.rs"),
        1,
        5,
        "quantum algorithm".to_string(),
        None,
        tokenize("quantum algorithm"),
    );

    // Long document containing "quantum" only once among many other tokens
    let long_tokens: Vec<String> = (0..50)
        .map(|i| format!("filler_word_{}", i))
        .chain(std::iter::once("quantum".to_string()))
        .collect();
    let long_content = long_tokens.join(" ");
    let long_doc = CodeChunk::new(
        PathBuf::from("long.rs"),
        1,
        50,
        long_content,
        None,
        long_tokens,
    );

    let index = LexicalVectorIndex::build_from_chunks(vec![short_doc, long_doc]);
    let results = index.search_bm25("quantum", 10);

    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].chunk.file_path,
        PathBuf::from("short.rs"),
        "Shorter document with higher term density should rank above longer document"
    );
}

// ============================================================================
// 4. Cosine Similarity Tests
// ============================================================================

#[test]
fn test_cosine_similarity_properties() {
    let mut v1 = HashMap::new();
    v1.insert("tokio".to_string(), 2.5);
    v1.insert("async".to_string(), 1.8);
    v1.insert("mutex".to_string(), 3.1);

    // Identical vector gives 1.0
    let sim_identical = cosine_similarity(&v1, &v1);
    assert!(
        (sim_identical - 1.0).abs() < 1e-6,
        "Identical vectors must have cosine similarity = 1.0"
    );

    // Orthogonal vector gives 0.0
    let mut v2 = HashMap::new();
    v2.insert("database".to_string(), 4.0);
    v2.insert("sql".to_string(), 2.0);
    let sim_orthogonal = cosine_similarity(&v1, &v2);
    assert_eq!(
        sim_orthogonal, 0.0,
        "Disjoint vectors must have cosine similarity = 0.0"
    );

    // Partial overlap gives value between 0.0 and 1.0
    let mut v3 = HashMap::new();
    v3.insert("tokio".to_string(), 1.0);
    v3.insert("spawn".to_string(), 2.0);
    let sim_partial = cosine_similarity(&v1, &v3);
    assert!(
        sim_partial > 0.0 && sim_partial < 1.0,
        "Partially overlapping vectors must have similarity in (0, 1), got {}",
        sim_partial
    );

    // Symmetry: sim(A, B) == sim(B, A)
    assert!(
        (cosine_similarity(&v1, &v3) - cosine_similarity(&v3, &v1)).abs() < 1e-6,
        "Cosine similarity must be symmetric"
    );

    // Empty vector gives 0.0
    let empty: HashMap<String, f64> = HashMap::new();
    assert_eq!(cosine_similarity(&v1, &empty), 0.0);
    assert_eq!(cosine_similarity(&empty, &empty), 0.0);
}

// ============================================================================
// 5. Query Search Accuracy Tests (Realistic Codebase Corpus)
// ============================================================================

#[test]
fn test_query_search_accuracy_across_domains() {
    let chunk_db = CodeChunk::new(
        PathBuf::from("src/db/pool.rs"),
        1,
        40,
        r#"
pub struct ConnectionPool {
    max_connections: u32,
    idle_timeout: Duration,
    connections: Vec<PostgresConnection>,
}

impl ConnectionPool {
    pub async fn acquire_connection(&mut self) -> Result<PostgresConnection, PoolError> {
        // Acquire pooled postgres connection
    }
}
"#
        .to_string(),
        Some("ConnectionPool".to_string()),
        tokenize(
            "pub struct ConnectionPool max_connections idle_timeout PostgresConnection acquire_connection postgres pool"
        ),
    );

    let chunk_router = CodeChunk::new(
        PathBuf::from("src/net/router.rs"),
        1,
        40,
        r#"
pub struct HttpRouter {
    routes: HashMap<String, RouteHandler>,
}

impl HttpRouter {
    pub fn route_get(&mut self, path: &str, handler: RouteHandler) {
        self.routes.insert(path.to_string(), handler);
    }

    pub async fn dispatch_request(&self, req: HttpRequest) -> HttpResponse {
        // Dispatch incoming HTTP request to matching route handler middleware
    }
}
"#
        .to_string(),
        Some("HttpRouter".to_string()),
        tokenize(
            "pub struct HttpRouter routes RouteHandler route_get dispatch_request HttpRequest HttpResponse middleware"
        ),
    );

    let chunk_auth = CodeChunk::new(
        PathBuf::from("src/auth/jwt.rs"),
        1,
        40,
        r#"
pub struct JwtValidator {
    secret_key: Vec<u8>,
    issuer: String,
}

impl JwtValidator {
    pub fn validate_bearer_token(&self, token_str: &str) -> Result<UserClaims, AuthError> {
        // Validate JWT bearer token signature and expiration claims
    }
}
"#
        .to_string(),
        Some("JwtValidator".to_string()),
        tokenize(
            "pub struct JwtValidator secret_key issuer validate_bearer_token UserClaims AuthError jwt signature expiration claims"
        ),
    );

    let chunk_cache = CodeChunk::new(
        PathBuf::from("src/cache/lru.rs"),
        1,
        40,
        r#"
pub struct LruCache<K, V> {
    capacity: usize,
    entries: HashMap<K, Node<V>>,
}

impl<K, V> LruCache<K, V> {
    pub fn evict_oldest(&mut self) -> Option<(K, V)> {
        // Eviction policy for least recently used entries
    }
}
"#
        .to_string(),
        Some("LruCache".to_string()),
        tokenize("pub struct LruCache capacity entries evict_oldest eviction policy least recently used entries"),
    );

    let index = LexicalVectorIndex::build_from_chunks(vec![
        chunk_db.clone(),
        chunk_router.clone(),
        chunk_auth.clone(),
        chunk_cache.clone(),
    ]);

    // Query 1: Database connection pool
    let res_db = index.search("postgres database connection acquire pool", 5);
    assert!(!res_db.is_empty());
    assert_eq!(res_db[0].chunk.file_path, PathBuf::from("src/db/pool.rs"));

    // Query 2: JWT bearer token validation
    let res_auth = index.search("jwt bearer token validate authentication claims", 5);
    assert!(!res_auth.is_empty());
    assert_eq!(
        res_auth[0].chunk.file_path,
        PathBuf::from("src/auth/jwt.rs")
    );

    // Query 3: HTTP router dispatch
    let res_router = index.search("http router dispatch request middleware route", 5);
    assert!(!res_router.is_empty());
    assert_eq!(
        res_router[0].chunk.file_path,
        PathBuf::from("src/net/router.rs")
    );

    // Query 4: LRU eviction policy
    let res_cache = index.search("lru cache eviction policy capacity", 5);
    assert!(!res_cache.is_empty());
    assert_eq!(
        res_cache[0].chunk.file_path,
        PathBuf::from("src/cache/lru.rs")
    );
}

// ============================================================================
// 6. VectorSearchTool Metadata & Execution Tests
// ============================================================================

#[test]
fn test_tool_metadata_and_schema() {
    let tool = VectorSearchTool::new();
    assert_eq!(tool.name(), "vector_search");
    assert!(
        tool.description().contains("Semantic search"),
        "Description should mention semantic search"
    );

    let schema = tool.parameters();
    assert_eq!(schema["type"], "object");
    let required = schema["required"].as_array().expect("required array");
    assert!(required.iter().any(|v| v.as_str() == Some("query")));

    assert!(schema["properties"]["query"].is_object());
    assert!(schema["properties"]["path"].is_object());
    assert!(schema["properties"]["limit"].is_object());
}

struct TestWorkspace {
    path: PathBuf,
}

impl TestWorkspace {
    fn new(prefix: &str) -> Self {
        let unique = uuid::Uuid::new_v4();
        let path = std::env::temp_dir().join(format!("fusion_vec_test_{}_{}", prefix, unique));
        fs::create_dir_all(&path).expect("Failed to create test workspace directory");
        Self { path }
    }

    fn write_file(&self, rel_path: &str, content: &str) -> PathBuf {
        let full_path = self.path.join(rel_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("Failed to create parent directories");
        }
        let mut file = File::create(&full_path).expect("Failed to create test file");
        file.write_all(content.as_bytes())
            .expect("Failed to write content");
        full_path
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[tokio::test]
async fn test_tool_execute_text_output() {
    let ws = TestWorkspace::new("text_exec");
    ws.write_file(
        "src/crypto/aes.rs",
        r#"
pub struct AesCipher {
    key: [u8; 32],
}

impl AesCipher {
    pub fn encrypt_block(&self, block: &[u8]) -> Vec<u8> {
        // AES symmetric block encryption
    }
}
"#,
    );

    ws.write_file(
        "src/crypto/rsa.rs",
        r#"
pub struct RsaKeyPair {
    public_exponent: u64,
}

impl RsaKeyPair {
    pub fn sign_message(&self, msg: &[u8]) -> Vec<u8> {
        // Asymmetric RSA digital signature
    }
}
"#,
    );

    let tool = VectorSearchTool::new();
    let ctx = ToolContext {
        cwd: ws.path.clone(),
        env: HashMap::new(),
    };

    let result = tool
        .execute(
            json!({
                "query": "aes cipher encrypt block symmetric",
                "limit": 5,
            }),
            &ctx,
        )
        .await
        .expect("vector_search execute should succeed");

    assert!(result.contains("src/crypto/aes.rs"));
    assert!(result.contains("Score:"));
    assert!(result.contains("encrypt_block"));
}

#[tokio::test]
async fn test_tool_execute_json_output() {
    let ws = TestWorkspace::new("json_exec");
    ws.write_file(
        "src/auth.rs",
        "pub fn authenticate_user(token: &str) -> bool { true }\n",
    );

    let tool = VectorSearchTool::new();
    let ctx = ToolContext {
        cwd: ws.path.clone(),
        env: HashMap::new(),
    };

    let result_json = tool
        .execute(
            json!({
                "query": "authenticate user token",
                "format": "json",
            }),
            &ctx,
        )
        .await
        .expect("vector_search JSON execute should succeed");

    let parsed: Vec<SearchResult> =
        serde_json::from_str(&result_json).expect("Output should be valid JSON SearchResult array");
    assert!(!parsed.is_empty());
    assert_eq!(
        parsed[0].chunk.symbol_name,
        Some("authenticate_user".to_string())
    );
}

#[tokio::test]
async fn test_tool_execute_missing_query_error() {
    let tool = VectorSearchTool::new();
    let ctx = ToolContext {
        cwd: PathBuf::from("."),
        env: HashMap::new(),
    };

    let res = tool.execute(json!({}), &ctx).await;
    assert!(res.is_err(), "Empty/missing query should return an error");
}
