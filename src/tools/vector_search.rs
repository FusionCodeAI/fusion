//! In-memory semantic codebase indexer and vector search tool.
//!
//! Provides pure-Rust lightweight semantic search over codebase files:
//! - **Chunker**: Walks workspace files (respecting `.gitignore`), splits files into
//!   chunks of 30-50 lines with 5-line overlap, and extracts identifier tokens and symbols.
//! - **LexicalVectorIndex**:
//!   - Subword and identifier tokenizer splitting on camelCase, snake_case, and punctuation,
//!     lowercasing tokens.
//!   - Okapi BM25 statistical relevance ranking with Robertson-Spärck Jones IDF and
//!     document length normalization ($k_1 = 1.2$, $b = 0.75$).
//!   - Cosine similarity matching over TF-IDF vector embeddings.
//! - **VectorSearchTool**: `Tool` trait wrapper exposing semantic code search
//!   with query, directory path filter, and result limit.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::tools::types::{Tool, ToolContext};

// ============================================================================
// Constants & Configuration Defaults
// ============================================================================

/// Default BM25 term frequency saturation parameter ($k_1$).
pub const DEFAULT_BM25_K1: f64 = 1.2;

/// Default BM25 document length normalization parameter ($b$).
pub const DEFAULT_BM25_B: f64 = 0.75;

/// Default chunk size in lines (within 30-50 lines).
pub const DEFAULT_CHUNK_SIZE: usize = 40;

/// Default chunk overlap in lines.
pub const DEFAULT_CHUNK_OVERLAP: usize = 5;

/// Default maximum file size to scan (5 MB).
pub const DEFAULT_MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;

// ============================================================================
// Tokenizer
// ============================================================================

/// Splits input text on camelCase, snake_case, and punctuation, returning lowercase tokens.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current_segment = String::new();

    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current_segment.push(ch);
        } else {
            if !current_segment.is_empty() {
                split_camel_case(&current_segment, &mut tokens);
                current_segment.clear();
            }
        }
    }

    if !current_segment.is_empty() {
        split_camel_case(&current_segment, &mut tokens);
    }

    tokens
}

fn split_camel_case(s: &str, out: &mut Vec<String>) {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return;
    }

    let mut start = 0;
    for i in 1..chars.len() {
        let prev = chars[i - 1];
        let curr = chars[i];

        // Transition 1: lowercase or digit followed by uppercase (e.g. "camelCase", "v2Api")
        let lower_to_upper = (prev.is_lowercase() || prev.is_ascii_digit()) && curr.is_uppercase();

        // Transition 2: sequence of uppercase followed by lowercase (e.g. "XMLParser" -> "XML", "Parser")
        let upper_to_lower = prev.is_uppercase()
            && curr.is_uppercase()
            && i + 1 < chars.len()
            && chars[i + 1].is_lowercase();

        if lower_to_upper || upper_to_lower {
            let word: String = chars[start..i].iter().collect();
            let lower = word.to_lowercase();
            if !lower.is_empty() {
                out.push(lower);
            }
            start = i;
        }
    }

    let word: String = chars[start..].iter().collect();
    let lower = word.to_lowercase();
    if !lower.is_empty() {
        out.push(lower);
    }
}

// ============================================================================
// Symbol Extraction
// ============================================================================

/// Extracts a prominent symbol or declaration name from a slice of code lines.
pub fn extract_symbol_name(lines: &[&str]) -> Option<String> {
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
            || trimmed.starts_with('#')
        {
            continue;
        }

        // Functions: fn, def, func, function
        if let Some(name) = extract_fn_name(trimmed) {
            return Some(name);
        }

        // Types/Structures: struct, enum, trait, class, interface, type, impl
        if let Some(name) = extract_type_name(trimmed) {
            return Some(name);
        }
    }
    None
}

fn extract_fn_name(line: &str) -> Option<String> {
    let keywords = [
        "pub async fn ",
        "async fn ",
        "pub fn ",
        "fn ",
        "def ",
        "func ",
        "function ",
    ];
    for kw in &keywords {
        if let Some(pos) = line.find(kw) {
            let after = &line[pos + kw.len()..];
            let after = after.trim_start();
            // Handle Go receiver: func (r *Receiver) MethodName(...)
            let after = if after.starts_with('(') {
                if let Some(close_paren) = after.find(')') {
                    after[close_paren + 1..].trim_start()
                } else {
                    after
                }
            } else {
                after
            };
            let ident: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !ident.is_empty() {
                return Some(ident);
            }
        }
    }
    None
}

fn extract_type_name(line: &str) -> Option<String> {
    let keywords = [
        "pub struct ",
        "struct ",
        "pub enum ",
        "enum ",
        "pub trait ",
        "trait ",
        "pub interface ",
        "interface ",
        "class ",
        "pub type ",
        "type ",
        "impl ",
    ];
    for kw in &keywords {
        if let Some(pos) = line.find(kw) {
            let after = &line[pos + kw.len()..];
            let after = after.trim_start();
            // Strip generics on impl, e.g. impl<T> Foo
            let after = if after.starts_with('<') {
                if let Some(close_bracket) = after.find('>') {
                    after[close_bracket + 1..].trim_start()
                } else {
                    after
                }
            } else {
                after
            };
            let ident: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !ident.is_empty() {
                return Some(ident);
            }
        }
    }
    None
}

// ============================================================================
// CodeChunk & SearchResult Models
// ============================================================================

/// A contiguous chunk of code from a file with line bounds, content, and tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeChunk {
    pub file_path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub symbol_name: Option<String>,
    pub tokens: Vec<String>,
}

impl CodeChunk {
    /// Creates a new `CodeChunk`.
    pub fn new(
        file_path: PathBuf,
        start_line: usize,
        end_line: usize,
        content: String,
        symbol_name: Option<String>,
        tokens: Vec<String>,
    ) -> Self {
        Self {
            file_path,
            start_line,
            end_line,
            content,
            symbol_name,
            tokens,
        }
    }

    /// Number of lines spanned by this chunk.
    pub fn line_count(&self) -> usize {
        if self.end_line >= self.start_line {
            self.end_line - self.start_line + 1
        } else {
            0
        }
    }

    /// Number of tokens in this chunk.
    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }
}

/// A search match associating a `CodeChunk` with its relevance score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub chunk: CodeChunk,
    pub score: f64,
}

impl SearchResult {
    pub fn new(chunk: CodeChunk, score: f64) -> Self {
        Self { chunk, score }
    }
}

// ============================================================================
// Chunker
// ============================================================================

/// Chunker splits files into overlapping code chunks (30-50 lines) and extracts tokens.
#[derive(Debug, Clone)]
pub struct Chunker {
    pub chunk_size: usize,
    pub overlap: usize,
    pub max_file_size: u64,
}

impl Default for Chunker {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            overlap: DEFAULT_CHUNK_OVERLAP,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
        }
    }
}

impl Chunker {
    /// Creates a new `Chunker` with default 40-line chunk size and 5-line overlap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the target chunk size in lines (typically 30-50).
    pub fn with_chunk_size(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size.max(1);
        self
    }

    /// Sets the overlap between consecutive chunks in lines (default: 5).
    pub fn with_overlap(mut self, overlap: usize) -> Self {
        self.overlap = overlap;
        self
    }

    /// Sets the maximum file size in bytes to scan.
    pub fn with_max_file_size(mut self, max_size: u64) -> Self {
        self.max_file_size = max_size;
        self
    }

    /// Splits string content from a given file path into overlapping `CodeChunk`s.
    pub fn chunk_text(&self, file_path: &Path, content: &str) -> Vec<CodeChunk> {
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return Vec::new();
        }

        let total_lines = lines.len();
        let mut chunks = Vec::new();
        let mut start = 0;

        while start < total_lines {
            let end = (start + self.chunk_size).min(total_lines);
            let chunk_lines = &lines[start..end];
            let chunk_content = chunk_lines.join("\n");
            let tokens = tokenize(&chunk_content);
            let symbol_name = extract_symbol_name(chunk_lines);

            chunks.push(CodeChunk {
                file_path: file_path.to_path_buf(),
                start_line: start + 1,
                end_line: end,
                content: chunk_content,
                symbol_name,
                tokens,
            });

            if end >= total_lines {
                break;
            }

            let step = if self.chunk_size > self.overlap {
                self.chunk_size - self.overlap
            } else {
                1
            };
            start += step;
        }

        chunks
    }

    /// Reads and chunks a single file from disk.
    pub fn chunk_file(&self, path: &Path) -> std::io::Result<Vec<CodeChunk>> {
        let content = std::fs::read_to_string(path)?;
        Ok(self.chunk_text(path, &content))
    }

    /// Walks workspace files respecting `.gitignore`, chunking all readable code files.
    pub fn walk_and_chunk(
        &self,
        root: &Path,
        path_filter: Option<&Path>,
    ) -> anyhow::Result<Vec<CodeChunk>> {
        let target_path = match path_filter {
            Some(filter) => {
                if filter.is_absolute() {
                    filter.to_path_buf()
                } else {
                    root.join(filter)
                }
            }
            None => root.to_path_buf(),
        };

        if !target_path.exists() {
            return Ok(Vec::new());
        }

        if target_path.is_file() {
            return match self.chunk_file(&target_path) {
                Ok(chunks) => Ok(chunks),
                Err(_) => Ok(Vec::new()),
            };
        }

        let mut builder = WalkBuilder::new(&target_path);
        builder
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .parents(true);

        let mut all_chunks = Vec::new();

        for entry_result in builder.build() {
            let entry = match entry_result {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if let Ok(meta) = entry.metadata() {
                if meta.len() > self.max_file_size {
                    continue;
                }
            }

            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };

            let chunks = self.chunk_text(path, &content);
            all_chunks.extend(chunks);
        }

        Ok(all_chunks)
    }
}

// ============================================================================
// Vector Math
// ============================================================================

/// Computes cosine similarity between two sparse vector representations.
///
/// $$\text{CosineSim}(\mathbf{u}, \mathbf{v}) = \frac{\sum_i u_i \cdot v_i}{\|\mathbf{u}\|_2 \cdot \|\mathbf{v}\|_2}$$
pub fn cosine_similarity(v1: &HashMap<String, f64>, v2: &HashMap<String, f64>) -> f64 {
    if v1.is_empty() || v2.is_empty() {
        return 0.0;
    }

    let mut dot_product = 0.0;
    let mut norm1_sq = 0.0;
    let mut norm2_sq = 0.0;

    for (k, &w1) in v1 {
        norm1_sq += w1 * w1;
        if let Some(&w2) = v2.get(k) {
            dot_product += w1 * w2;
        }
    }

    for &w2 in v2.values() {
        norm2_sq += w2 * w2;
    }

    if norm1_sq <= 0.0 || norm2_sq <= 0.0 {
        return 0.0;
    }

    let sim = dot_product / (norm1_sq.sqrt() * norm2_sq.sqrt());
    sim.clamp(0.0, 1.0)
}

// ============================================================================
// LexicalVectorIndex
// ============================================================================

/// In-memory lexical and vector search index combining Okapi BM25 and TF-IDF Cosine Similarity.
#[derive(Debug, Clone)]
pub struct LexicalVectorIndex {
    chunks: Vec<CodeChunk>,
    doc_lengths: Vec<usize>,
    avg_doc_len: f64,
    doc_frequencies: HashMap<String, usize>,
    term_frequencies: Vec<HashMap<String, usize>>,
    tfidf_vectors: Vec<HashMap<String, f64>>,
    vector_norms: Vec<f64>,
    k1: f64,
    b: f64,
}

impl Default for LexicalVectorIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl LexicalVectorIndex {
    /// Creates a new empty `LexicalVectorIndex` with default BM25 parameters ($k_1=1.2, b=0.75$).
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            doc_lengths: Vec::new(),
            avg_doc_len: 0.0,
            doc_frequencies: HashMap::new(),
            term_frequencies: Vec::new(),
            tfidf_vectors: Vec::new(),
            vector_norms: Vec::new(),
            k1: DEFAULT_BM25_K1,
            b: DEFAULT_BM25_B,
        }
    }

    /// Creates an index with custom BM25 parameters $k_1$ and $b$.
    pub fn with_params(k1: f64, b: f64) -> Self {
        Self {
            k1,
            b,
            ..Self::new()
        }
    }

    /// Tokenizes input text into subwords (splitting camelCase, snake_case, punctuation).
    pub fn tokenize(text: &str) -> Vec<String> {
        tokenize(text)
    }

    /// Number of chunks currently indexed.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Returns true if the index contains no chunks.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Returns a slice of all indexed code chunks.
    pub fn chunks(&self) -> &[CodeChunk] {
        &self.chunks
    }

    /// Adds a chunk to the index and recomputes corpus statistics.
    pub fn add_chunk(&mut self, chunk: CodeChunk) {
        self.chunks.push(chunk);
        self.rebuild();
    }

    /// Adds multiple chunks to the index and recomputes corpus statistics.
    pub fn add_chunks(&mut self, chunks: impl IntoIterator<Item = CodeChunk>) {
        self.chunks.extend(chunks);
        self.rebuild();
    }

    /// Builds a new index from a vector of chunks.
    pub fn build_from_chunks(chunks: Vec<CodeChunk>) -> Self {
        let mut index = Self {
            chunks,
            ..Self::new()
        };
        index.rebuild();
        index
    }

    /// Rebuilds inverted index, BM25 corpus stats, and TF-IDF vectors.
    fn rebuild(&mut self) {
        let n = self.chunks.len();
        self.doc_lengths.clear();
        self.doc_frequencies.clear();
        self.term_frequencies.clear();
        self.tfidf_vectors.clear();
        self.vector_norms.clear();

        if n == 0 {
            self.avg_doc_len = 0.0;
            return;
        }

        let mut total_tokens = 0;

        for chunk in &self.chunks {
            let mut term_counts = HashMap::new();
            for token in &chunk.tokens {
                *term_counts.entry(token.clone()).or_insert(0usize) += 1;
            }

            for term in term_counts.keys() {
                *self.doc_frequencies.entry(term.clone()).or_insert(0usize) += 1;
            }

            let doc_len = chunk.tokens.len();
            total_tokens += doc_len;
            self.doc_lengths.push(doc_len);
            self.term_frequencies.push(term_counts);
        }

        self.avg_doc_len = total_tokens as f64 / n as f64;

        // Build TF-IDF vectors and norms
        for i in 0..n {
            let mut tfidf = HashMap::new();
            let mut norm_sq = 0.0;
            let term_counts = &self.term_frequencies[i];

            for (term, &tf) in term_counts {
                let idf = self.calc_idf(term);
                let weight = (tf as f64) * idf;
                norm_sq += weight * weight;
                tfidf.insert(term.clone(), weight);
            }

            let norm = norm_sq.sqrt();
            self.tfidf_vectors.push(tfidf);
            self.vector_norms.push(norm);
        }
    }

    /// Calculates Lucene-smoothed Robertson-Spärck Jones Inverse Document Frequency (IDF).
    ///
    /// $$\text{IDF}(t) = \ln\left( 1.0 + \frac{N - n(t) + 0.5}{n(t) + 0.5} \right)$$
    pub fn idf(&self, term: &str) -> f64 {
        self.calc_idf(term)
    }

    fn calc_idf(&self, term: &str) -> f64 {
        let n = self.chunks.len();
        if n == 0 {
            return 0.0;
        }
        let df = self.doc_frequencies.get(term).copied().unwrap_or(0) as f64;
        let n_f = n as f64;
        let idf = ((n_f - df + 0.5) / (df + 0.5) + 1.0).ln();
        idf.max(0.0)
    }

    /// Computes BM25 score of a chunk against query tokens.
    ///
    /// $$\text{score}(D, Q) = \sum_{t \in Q} \text{IDF}(t) \cdot \frac{f(t, D) \cdot (k_1 + 1)}{f(t, D) + k_1 \cdot (1 - b + b \cdot \frac{|D|}{\text{avgdl}})}$$
    pub fn score_bm25(&self, query_tokens: &[String], chunk_idx: usize) -> f64 {
        if chunk_idx >= self.chunks.len() || self.chunks.is_empty() {
            return 0.0;
        }

        let term_freqs = &self.term_frequencies[chunk_idx];
        let doc_len = self.doc_lengths[chunk_idx];
        let len_ratio = if self.avg_doc_len > 0.0 {
            doc_len as f64 / self.avg_doc_len
        } else {
            1.0
        };

        let mut score = 0.0;
        let unique_tokens: HashSet<&str> = query_tokens.iter().map(|s| s.as_str()).collect();

        for q_term in unique_tokens {
            if let Some(&tf) = term_freqs.get(q_term) {
                let idf = self.calc_idf(q_term);
                let tf_f = tf as f64;
                let numerator = tf_f * (self.k1 + 1.0);
                let denominator = tf_f + self.k1 * (1.0 - self.b + self.b * len_ratio);
                if denominator > 0.0 {
                    score += idf * (numerator / denominator);
                }
            }
        }

        score
    }

    /// Computes Cosine Similarity of an indexed chunk's TF-IDF vector against query tokens.
    pub fn score_cosine(&self, query_tokens: &[String], chunk_idx: usize) -> f64 {
        if chunk_idx >= self.chunks.len() || self.chunks.is_empty() {
            return 0.0;
        }

        let query_vector = self.build_query_vector(query_tokens);
        let chunk_vector = &self.tfidf_vectors[chunk_idx];
        cosine_similarity(&query_vector, chunk_vector)
    }

    /// Builds a TF-IDF sparse vector for query tokens using index IDF statistics.
    pub fn build_query_vector(&self, query_tokens: &[String]) -> HashMap<String, f64> {
        let mut vector = HashMap::new();
        let mut term_counts = HashMap::new();
        for t in query_tokens {
            *term_counts.entry(t).or_insert(0usize) += 1;
        }
        for (t, &tf) in &term_counts {
            let idf = self.calc_idf(t);
            let weight = (tf as f64) * idf;
            vector.insert((*t).clone(), weight);
        }
        vector
    }

    /// Searches using pure BM25 ranking algorithm.
    pub fn search_bm25(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() || self.chunks.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();
        for (idx, chunk) in self.chunks.iter().enumerate() {
            let score = self.score_bm25(&query_tokens, idx);
            if score > 0.0 {
                results.push(SearchResult {
                    chunk: chunk.clone(),
                    score,
                });
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if results.len() > limit {
            results.truncate(limit);
        }

        results
    }

    /// Searches using TF-IDF Cosine Similarity.
    pub fn search_cosine(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() || self.chunks.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();
        for (idx, chunk) in self.chunks.iter().enumerate() {
            let score = self.score_cosine(&query_tokens, idx);
            if score > 0.0 {
                results.push(SearchResult {
                    chunk: chunk.clone(),
                    score,
                });
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if results.len() > limit {
            results.truncate(limit);
        }

        results
    }

    /// Hybrid search combining BM25 and Cosine Similarity scores.
    pub fn search_hybrid(
        &self,
        query: &str,
        bm25_weight: f64,
        cosine_weight: f64,
        limit: usize,
    ) -> Vec<SearchResult> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() || self.chunks.is_empty() {
            return Vec::new();
        }

        let mut results = Vec::new();
        for (idx, chunk) in self.chunks.iter().enumerate() {
            let bm25 = self.score_bm25(&query_tokens, idx);
            let cosine = self.score_cosine(&query_tokens, idx);
            let score = bm25 * bm25_weight + cosine * cosine_weight;
            if score > 0.0 {
                results.push(SearchResult {
                    chunk: chunk.clone(),
                    score,
                });
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if results.len() > limit {
            results.truncate(limit);
        }

        results
    }

    /// Primary search entry point (using BM25 relevance ranking).
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        self.search_bm25(query, limit)
    }

    /// Searches chunks scoped by an optional path filter.
    pub fn search_filtered(
        &self,
        query: &str,
        path_filter: Option<&Path>,
        limit: usize,
    ) -> Vec<SearchResult> {
        let all_matches = self.search_bm25(query, self.chunks.len());
        match path_filter {
            Some(filter) => all_matches
                .into_iter()
                .filter(|res| {
                    res.chunk.file_path.starts_with(filter)
                        || res.chunk.file_path == filter
                        || res
                            .chunk
                            .file_path
                            .to_string_lossy()
                            .contains(&filter.to_string_lossy().to_string())
                })
                .take(limit)
                .collect(),
            None => all_matches.into_iter().take(limit).collect(),
        }
    }
}

// ============================================================================
// VectorSearchTool Implementation
// ============================================================================

/// Tool for performing semantic and lexical search across workspace code chunks.
#[derive(Debug, Clone, Default)]
pub struct VectorSearchTool;

impl VectorSearchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for VectorSearchTool {
    fn name(&self) -> &str {
        "vector_search"
    }

    fn description(&self) -> &str {
        "Semantic search across codebase without knowing exact file names or symbols."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language or keyword query to search across the codebase."
                },
                "path": {
                    "type": "string",
                    "description": "Optional subdirectory or file path to scope the search."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of code chunks to return (default: 10)."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter 'query'"))?
            .trim();

        if query.is_empty() {
            anyhow::bail!("Parameter 'query' cannot be empty");
        }

        let path_filter = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10) as usize;
        let limit = limit.clamp(1, 100);

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let cwd = ctx.cwd.clone();
        let query_owned = query.to_string();

        let (results, total_chunks) = tokio::task::spawn_blocking(move || {
            let chunker = Chunker::default();
            let chunks = chunker.walk_and_chunk(&cwd, path_filter.as_deref())?;
            let total = chunks.len();
            let index = LexicalVectorIndex::build_from_chunks(chunks);
            let search_results = index.search(&query_owned, limit);
            Ok::<_, anyhow::Error>((search_results, total))
        })
        .await
        .map_err(|e| anyhow::anyhow!("Vector search execution failed: {e}"))??;

        if format == "json" {
            return Ok(serde_json::to_string_pretty(&results)?);
        }

        if results.is_empty() {
            return Ok(format!(
                "No matching code chunks found for query '{}' across {} chunks scanned.",
                query, total_chunks
            ));
        }

        let mut output = String::new();
        output.push_str(&format!(
            "Found {} matching code chunk(s) for query '{}' (scanned {} chunks):\n\n",
            results.len(),
            query,
            total_chunks
        ));

        for (i, res) in results.iter().enumerate() {
            let rel_path = res
                .chunk
                .file_path
                .strip_prefix(&ctx.cwd)
                .unwrap_or(&res.chunk.file_path);
            let symbol_info = match &res.chunk.symbol_name {
                Some(sym) => format!(" [symbol: {}]", sym),
                None => String::new(),
            };

            output.push_str(&format!(
                "{}. {}:{}-{}{}\n   Score: {:.3}\n",
                i + 1,
                rel_path.display(),
                res.chunk.start_line,
                res.chunk.end_line,
                symbol_info,
                res.score
            ));

            output.push_str("   ------------------------------------------------------------\n");
            let content_lines: Vec<&str> = res.chunk.content.lines().collect();
            let display_lines = &content_lines[..content_lines.len().min(8)];
            for line in display_lines {
                output.push_str(&format!("   {}\n", line));
            }
            if content_lines.len() > 8 {
                output.push_str(&format!(
                    "   ... ({} lines total)\n",
                    content_lines.len()
                ));
            }
            output.push_str("   ------------------------------------------------------------\n\n");
        }

        Ok(output)
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_camel_case() {
        assert_eq!(tokenize("camelCase"), vec!["camel", "case"]);
        assert_eq!(
            tokenize("getUserById"),
            vec!["get", "user", "by", "id"]
        );
        assert_eq!(tokenize("XMLParser"), vec!["xml", "parser"]);
        assert_eq!(
            tokenize("XMLHttpRequest"),
            vec!["xml", "http", "request"]
        );
        assert_eq!(tokenize("v2Api"), vec!["v2", "api"]);
    }

    #[test]
    fn test_tokenize_snake_case() {
        assert_eq!(tokenize("snake_case"), vec!["snake", "case"]);
        assert_eq!(
            tokenize("my_variable_name"),
            vec!["my", "variable", "name"]
        );
        assert_eq!(
            tokenize("SCREAMING_SNAKE_CASE"),
            vec!["screaming", "snake", "case"]
        );
        assert_eq!(tokenize("__init__"), vec!["init"]);
    }

    #[test]
    fn test_tokenize_punctuation() {
        assert_eq!(
            tokenize("foo.bar(baz, qux);"),
            vec!["foo", "bar", "baz", "qux"]
        );
        assert_eq!(
            tokenize("let x: Option<String> = None;"),
            vec!["let", "x", "option", "string", "none"]
        );
    }

    #[test]
    fn test_chunker_basic() {
        let chunker = Chunker::new().with_chunk_size(5).with_overlap(2);
        let content = (1..=10)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");

        let chunks = chunker.chunk_text(Path::new("test.rs"), &content);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 5);
        assert_eq!(chunks[1].start_line, 4);
        assert_eq!(chunks[1].end_line, 8);
        assert_eq!(chunks[2].start_line, 7);
        assert_eq!(chunks[2].end_line, 10);
    }

    #[test]
    fn test_chunker_symbol_extraction() {
        let code = "pub async fn handle_request(req: Request) -> Response {\n    todo!()\n}\n";
        let chunker = Chunker::new();
        let chunks = chunker.chunk_text(Path::new("server.rs"), code);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].symbol_name, Some("handle_request".to_string()));
    }

    #[test]
    fn test_bm25_ranking() {
        let chunk1 = CodeChunk::new(
            PathBuf::from("file1.rs"),
            1,
            10,
            "database connection pool acquire postgresql".to_string(),
            None,
            tokenize("database connection pool acquire postgresql"),
        );
        let chunk2 = CodeChunk::new(
            PathBuf::from("file2.rs"),
            1,
            10,
            "http router route get post request".to_string(),
            None,
            tokenize("http router route get post request"),
        );
        let chunk3 = CodeChunk::new(
            PathBuf::from("file3.rs"),
            1,
            10,
            "database query connection execute statement sql".to_string(),
            None,
            tokenize("database query connection execute statement sql"),
        );

        let index = LexicalVectorIndex::build_from_chunks(vec![chunk1, chunk2, chunk3]);
        let results = index.search("database connection postgresql", 5);

        assert!(!results.is_empty());
        // file1.rs has both connection, database, and postgresql (rare term)
        assert_eq!(results[0].chunk.file_path, PathBuf::from("file1.rs"));
        assert!(results[0].score > 0.0);
    }

    #[test]
    fn test_cosine_similarity() {
        let mut v1 = HashMap::new();
        v1.insert("a".to_string(), 1.0);
        v1.insert("b".to_string(), 2.0);

        let mut v2 = HashMap::new();
        v2.insert("a".to_string(), 1.0);
        v2.insert("b".to_string(), 2.0);

        let sim = cosine_similarity(&v1, &v2);
        assert!((sim - 1.0).abs() < 1e-6);

        let mut v3 = HashMap::new();
        v3.insert("c".to_string(), 3.0);
        let sim_orthogonal = cosine_similarity(&v1, &v3);
        assert_eq!(sim_orthogonal, 0.0);
    }
}
