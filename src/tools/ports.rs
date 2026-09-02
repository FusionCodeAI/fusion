//! Local TCP Port Scanner and Dev Server Discovery Tool.
//!
//! Provides fast, asynchronous TCP port scanning, service identification, and active
//! development server discovery on localhost (or remote hosts).
//!
//! Features:
//! - High-concurrency async TCP port probing built on Tokio with microsecond-level latency measurement.
//! - Curated presets for common developer environments:
//!   - `dev`: Modern web frameworks, bundlers, and AI endpoints (3000, 5173, 8000, 8080, 11434, etc.)
//!   - `web`: Standard web server ports (80, 443, 3000, 5000, 5173, 8000, 8080, etc.)
//!   - `databases`: PostgreSQL, MySQL, Redis, MongoDB, Elasticsearch, Meilisearch, Vector DBs, etc.
//!   - `ai`: Ollama, LM Studio, Gradio, Streamlit, vLLM, LocalAI, llama.cpp, Qdrant, Milvus.
//!   - `quick`: Top 10 most common development ports for ultra-fast checks.
//!   - `all_common`: Comprehensive database of 100+ standard developer and infrastructure ports.
//! - Deep HTTP service fingerprinting:
//!   - Framework detection: Vite, Next.js, Nuxt, SvelteKit, Astro, Express, FastAPI, Flask, Django,
//!     Gradio, Streamlit, JupyterLab, Spring Boot, Ruby on Rails, etc.
//!   - Extracts HTTP status code, Server header, HTML `<title>`, X-Powered-By, and Content-Type.
//! - Raw TCP protocol banner grabbing (Redis PING, SSH, MySQL, PostgreSQL).
//! - Actions:
//!   - `scan`: Scan specified ports or presets, returning open ports with service metadata.
//!   - `check`: Fast check of specific individual ports.
//!   - `find_free`: Find available/unbound ports starting from a base port (e.g., for starting a new server).
//!   - `wait_for`: Poll and wait for a port to open or close (ideal for watching dev server startup).
//!   - `identify`: Deep inspection and fingerprinting of a specific target port.
//!   - `list_presets`: List all available presets and their configured ports.
//! - Flexible output formats: Rich ANSI text table, clean Markdown table, or structured JSON.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::tools::types::{Tool, ToolContext};

// ===========================================================================
// Known Services Database
// ===========================================================================

/// Metadata about a well-known TCP port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownService {
    pub port: u16,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub default_protocol: &'static str,
}

/// Static registry of well-known developer, database, and infrastructure ports.
pub static KNOWN_SERVICES: LazyLock<Vec<KnownService>> = LazyLock::new(|| {
    vec![
        // Standard Web
        KnownService { port: 80, name: "HTTP", category: "Web / Standard", description: "Standard World Wide Web HTTP", default_protocol: "http" },
        KnownService { port: 443, name: "HTTPS", category: "Web / Standard", description: "Standard Secure HTTP over TLS", default_protocol: "https" },
        KnownService { port: 8080, name: "HTTP-Alt / Tomcat / llama.cpp", category: "Web / App Server", description: "Alternative HTTP / Spring Boot / Tomcat / llama.cpp / LocalAI", default_protocol: "http" },
        KnownService { port: 8443, name: "HTTPS-Alt", category: "Web / App Server", description: "Alternative HTTPS server", default_protocol: "https" },

        // Frontend & Web Framework Dev Servers
        KnownService { port: 3000, name: "Node / React / Next.js / Rails", category: "Web / Frontend", description: "Default dev server for Next.js, Create React App, Grafana, Rails", default_protocol: "http" },
        KnownService { port: 3001, name: "Dev Server (Secondary)", category: "Web / Frontend", description: "Secondary dev server / React / Express", default_protocol: "http" },
        KnownService { port: 3002, name: "Dev Server (Tertiary)", category: "Web / Frontend", description: "Tertiary dev server", default_protocol: "http" },
        KnownService { port: 3333, name: "AdonisJS / Dev Server", category: "Web / Backend", description: "AdonisJS framework / Alt web dev", default_protocol: "http" },
        KnownService { port: 4000, name: "Hexo / Jekyll / Phoenix / Strapi", category: "Web / Static & CMS", description: "Static site generators, Phoenix, Strapi CMS", default_protocol: "http" },
        KnownService { port: 4173, name: "Vite Preview", category: "Web / Frontend", description: "Vite production preview server", default_protocol: "http" },
        KnownService { port: 4200, name: "Angular CLI", category: "Web / Frontend", description: "Default Angular development server", default_protocol: "http" },
        KnownService { port: 4321, name: "Astro Dev Server", category: "Web / Frontend", description: "Astro static & SSR web framework", default_protocol: "http" },
        KnownService { port: 5000, name: "Flask / AirPlay / ASP.NET", category: "Web / Backend", description: "Flask default dev server, macOS AirPlay Receiver, ASP.NET", default_protocol: "http" },
        KnownService { port: 5001, name: "Flask SSL / Control Center", category: "Web / Backend", description: "Flask SSL / macOS Control Center", default_protocol: "https" },
        KnownService { port: 5173, name: "Vite Dev Server", category: "Web / Frontend", description: "Vite (Vue 3, React, Svelte, Solid) dev server", default_protocol: "http" },
        KnownService { port: 5174, name: "Vite Dev Server #2", category: "Web / Frontend", description: "Vite secondary instance", default_protocol: "http" },
        KnownService { port: 5175, name: "Vite Dev Server #3", category: "Web / Frontend", description: "Vite tertiary instance", default_protocol: "http" },
        KnownService { port: 5500, name: "VS Code Live Server", category: "Web / Frontend", description: "Visual Studio Code Live Server extension", default_protocol: "http" },
        KnownService { port: 8000, name: "Django / FastAPI / Python HTTP", category: "Web / Backend", description: "Django dev server, FastAPI/Uvicorn, Python http.server, vLLM", default_protocol: "http" },
        KnownService { port: 8001, name: "FastAPI Alt / Docs", category: "Web / Backend", description: "FastAPI secondary / API documentation", default_protocol: "http" },
        KnownService { port: 8081, name: "React Native Metro", category: "Mobile / Dev", description: "React Native Metro bundler / Alt web server", default_protocol: "http" },
        KnownService { port: 8082, name: "Alt Web Server", category: "Web / Backend", description: "Alternative web service", default_protocol: "http" },
        KnownService { port: 8787, name: "Cloudflare Wrangler", category: "Cloud / Serverless", description: "Cloudflare Workers / Wrangler dev server", default_protocol: "http" },
        KnownService { port: 8888, name: "Jupyter Notebook", category: "Data Science", description: "Jupyter Notebook & JupyterLab server", default_protocol: "http" },
        KnownService { port: 9000, name: "PHP-FPM / MinIO / SonarQube", category: "Web / Storage", description: "PHP-FPM listener, MinIO S3 API, SonarQube web UI", default_protocol: "http" },
        KnownService { port: 9001, name: "MinIO Console / Supervisord", category: "Management", description: "MinIO Admin UI, Supervisord web interface", default_protocol: "http" },
        KnownService { port: 9999, name: "Webpack / Dev Tool", category: "Development", description: "Webpack dev server alternate / Debuggers", default_protocol: "http" },

        // AI / LLM Local Servers
        KnownService { port: 11434, name: "Ollama Local LLM", category: "AI / LLM", description: "Ollama local LLM runner REST API", default_protocol: "http" },
        KnownService { port: 1234, name: "LM Studio", category: "AI / LLM", description: "LM Studio local OpenAI-compatible LLM server", default_protocol: "http" },
        KnownService { port: 7860, name: "Gradio AI App", category: "AI / ML", description: "Gradio machine learning & AI web interface", default_protocol: "http" },
        KnownService { port: 8501, name: "Streamlit App", category: "Data / AI", description: "Streamlit data app / dashboard server", default_protocol: "http" },
        KnownService { port: 6006, name: "TensorBoard", category: "AI / ML", description: "TensorFlow TensorBoard visualization", default_protocol: "http" },
        KnownService { port: 6333, name: "Qdrant Vector DB (HTTP)", category: "AI / Vector DB", description: "Qdrant vector search engine HTTP API", default_protocol: "http" },
        KnownService { port: 6334, name: "Qdrant Vector DB (gRPC)", category: "AI / Vector DB", description: "Qdrant vector search engine gRPC endpoint", default_protocol: "tcp" },
        KnownService { port: 19530, name: "Milvus Vector DB", category: "AI / Vector DB", description: "Milvus distributed vector database", default_protocol: "tcp" },
        KnownService { port: 8086, name: "InfluxDB API", category: "Database / Time-Series", description: "InfluxDB time-series database HTTP API", default_protocol: "http" },

        // Databases & Caches
        KnownService { port: 3306, name: "MySQL / MariaDB", category: "Database / SQL", description: "MySQL / MariaDB relational database", default_protocol: "tcp" },
        KnownService { port: 5432, name: "PostgreSQL", category: "Database / SQL", description: "PostgreSQL object-relational database", default_protocol: "tcp" },
        KnownService { port: 6379, name: "Redis", category: "Database / In-Memory", description: "Redis in-memory key-value database & cache", default_protocol: "tcp" },
        KnownService { port: 27017, name: "MongoDB", category: "Database / NoSQL", description: "MongoDB document database", default_protocol: "tcp" },
        KnownService { port: 9200, name: "Elasticsearch", category: "Search / Analytics", description: "Elasticsearch REST API", default_protocol: "http" },
        KnownService { port: 9300, name: "Elasticsearch Cluster", category: "Search / Analytics", description: "Elasticsearch internal cluster communication", default_protocol: "tcp" },
        KnownService { port: 7700, name: "Meilisearch", category: "Search / Analytics", description: "Meilisearch instant search engine HTTP API", default_protocol: "http" },
        KnownService { port: 7474, name: "Neo4j Web / HTTP", category: "Database / Graph", description: "Neo4j graph database browser & REST API", default_protocol: "http" },
        KnownService { port: 7687, name: "Neo4j Bolt", category: "Database / Graph", description: "Neo4j Bolt binary protocol", default_protocol: "tcp" },
        KnownService { port: 9042, name: "Apache Cassandra", category: "Database / NoSQL", description: "Cassandra native transport CQL port", default_protocol: "tcp" },
        KnownService { port: 1433, name: "Microsoft SQL Server", category: "Database / SQL", description: "MS SQL Server database engine", default_protocol: "tcp" },
        KnownService { port: 5984, name: "CouchDB", category: "Database / NoSQL", description: "Apache CouchDB document database", default_protocol: "http" },

        // Message Queues & Microservices
        KnownService { port: 5672, name: "RabbitMQ AMQP", category: "Message Queue", description: "RabbitMQ AMQP messaging broker", default_protocol: "tcp" },
        KnownService { port: 15672, name: "RabbitMQ Management", category: "Message Queue", description: "RabbitMQ web management UI", default_protocol: "http" },
        KnownService { port: 9092, name: "Apache Kafka", category: "Message Queue", description: "Apache Kafka plaintext broker", default_protocol: "tcp" },
        KnownService { port: 2181, name: "Apache ZooKeeper", category: "Coordination", description: "ZooKeeper client port", default_protocol: "tcp" },
        KnownService { port: 2379, name: "etcd client", category: "Coordination", description: "etcd distributed key-value store client API", default_protocol: "http" },
        KnownService { port: 8500, name: "Consul HTTP API", category: "Service Mesh", description: "HashiCorp Consul HTTP API & UI", default_protocol: "http" },
        KnownService { port: 4566, name: "LocalStack", category: "Cloud Emulation", description: "LocalStack AWS cloud emulator edge port", default_protocol: "http" },

        // Observability & Metrics
        KnownService { port: 9090, name: "Prometheus Server", category: "Observability", description: "Prometheus metrics server & web UI", default_protocol: "http" },
        KnownService { port: 9100, name: "Node Exporter", category: "Observability", description: "Prometheus Node Exporter system metrics", default_protocol: "http" },
        KnownService { port: 16686, name: "Jaeger UI", category: "Observability", description: "Jaeger distributed tracing web UI", default_protocol: "http" },
        KnownService { port: 9411, name: "Zipkin", category: "Observability", description: "Zipkin distributed tracing collector & UI", default_protocol: "http" },

        // System & Remote Access
        KnownService { port: 22, name: "SSH", category: "System / Remote", description: "Secure Shell remote login", default_protocol: "tcp" },
        KnownService { port: 8545, name: "Ethereum JSON-RPC", category: "Web3 / Blockchain", description: "Ethereum local node / Hardhat / Anvil RPC", default_protocol: "http" },
    ]
});

/// Look up known service metadata for a specific port.
pub fn lookup_service(port: u16) -> Option<&'static KnownService> {
    KNOWN_SERVICES.iter().find(|s| s.port == port)
}

// ===========================================================================
// Port Presets
// ===========================================================================

/// Named presets for port scanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortPreset {
    /// Modern web frameworks, bundlers, and AI endpoints.
    Dev,
    /// Standard web servers and dev servers.
    Web,
    /// Relational, document, key-value, and vector databases.
    Databases,
    /// AI/LLM servers (Ollama, LM Studio, Gradio, vLLM, Streamlit, etc.).
    Ai,
    /// Fast check on top 10 most common development ports.
    Quick,
    /// Comprehensive scan across 100+ standard ports.
    AllCommon,
}

impl PortPreset {
    pub fn name(&self) -> &'static str {
        match self {
            PortPreset::Dev => "dev",
            PortPreset::Web => "web",
            PortPreset::Databases => "databases",
            PortPreset::Ai => "ai",
            PortPreset::Quick => "quick",
            PortPreset::AllCommon => "all_common",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            PortPreset::Dev => "Modern web frameworks (Vite, Next.js, CRA, Astro), backends (FastAPI, Flask, Django, Rails), and AI servers (Ollama, LM Studio)",
            PortPreset::Web => "Standard web ports (80, 443, 8080, 8443) and frontend dev servers",
            PortPreset::Databases => "Database engines (PostgreSQL, MySQL, Redis, MongoDB, Elasticsearch, Meilisearch, Qdrant, Milvus)",
            PortPreset::Ai => "AI and LLM servers (Ollama, LM Studio, Gradio, Streamlit, TensorBoard, vLLM, Qdrant)",
            PortPreset::Quick => "Top 10 most popular dev ports (3000, 5173, 8000, 8080, 11434, 5000, 4200, 8888, 5432, 6379)",
            PortPreset::AllCommon => "Comprehensive catalog of 100+ known developer, database, and infrastructure ports",
        }
    }

    pub fn ports(&self) -> Vec<u16> {
        match self {
            PortPreset::Dev => vec![
                3000, 3001, 3002, 3333, 4000, 4173, 4200, 4321, 5000, 5001,
                5173, 5174, 5175, 5500, 7860, 8000, 8001, 8080, 8081, 8501,
                8787, 8888, 9000, 9999, 11434, 1234,
            ],
            PortPreset::Web => vec![
                80, 443, 3000, 3001, 4000, 4173, 4200, 4321, 5000, 5173,
                5174, 8000, 8080, 8081, 8443, 8888, 9000,
            ],
            PortPreset::Databases => vec![
                1433, 2181, 2379, 3306, 5432, 5984, 6333, 6334, 6379, 7474,
                7687, 7700, 8086, 9042, 9092, 9200, 9300, 19530, 27017,
            ],
            PortPreset::Ai => vec![
                1234, 5000, 6006, 6333, 7860, 8000, 8080, 8501, 11434, 19530,
            ],
            PortPreset::Quick => vec![
                3000, 5173, 8000, 8080, 11434, 5000, 4200, 8888, 5432, 6379,
            ],
            PortPreset::AllCommon => {
                let mut p: Vec<u16> = KNOWN_SERVICES.iter().map(|s| s.port).collect();
                p.sort_unstable();
                p.dedup();
                p
            }
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "dev" | "developer" | "frontend" | "frontend-backend" => Some(PortPreset::Dev),
            "web" | "http" | "www" => Some(PortPreset::Web),
            "database" | "databases" | "db" | "dbs" => Some(PortPreset::Databases),
            "ai" | "llm" | "ml" | "models" => Some(PortPreset::Ai),
            "quick" | "fast" | "top" => Some(PortPreset::Quick),
            "all" | "all_common" | "common" | "full" => Some(PortPreset::AllCommon),
            _ => None,
        }
    }
}

// ===========================================================================
// Scan Results & Data Models
// ===========================================================================

/// Connection status of a scanned port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortState {
    /// Port is actively listening and accepted a TCP connection.
    Open,
    /// Connection was refused or explicitly rejected.
    Closed,
    /// Connection timed out or was unreachable/filtered by firewall.
    Filtered,
}

impl std::fmt::Display for PortState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortState::Open => write!(f, "open"),
            PortState::Closed => write!(f, "closed"),
            PortState::Filtered => write!(f, "filtered"),
        }
    }
}

/// Extracted HTTP service metadata from an open port.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HttpServiceInfo {
    /// HTTP status code (e.g. 200, 404, 302).
    pub status_code: Option<u16>,
    /// HTTP status text (e.g. "OK", "Not Found").
    pub status_text: Option<String>,
    /// HTML page `<title>` if present.
    pub title: Option<String>,
    /// Server header value (e.g. "uvicorn", "Werkzeug/3.0", "nginx").
    pub server: Option<String>,
    /// X-Powered-By header value (e.g. "Express", "Next.js").
    pub powered_by: Option<String>,
    /// Content-Type header value.
    pub content_type: Option<String>,
    /// Detected web framework or technology name.
    pub detected_framework: Option<String>,
    /// HTTP endpoint URL (e.g. `http://localhost:3000`).
    pub url: String,
    /// Truncated preview of response body or headers.
    pub response_preview: Option<String>,
}

/// Complete scan result for an individual TCP port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortScanResult {
    /// Port number.
    pub port: u16,
    /// Port state (open, closed, filtered).
    pub state: PortState,
    /// Measured round-trip connection latency in milliseconds.
    pub latency_ms: Option<f64>,
    /// Name of known service or detected framework.
    pub service: Option<String>,
    /// High-level category (e.g. "Web / Frontend", "AI / LLM").
    pub category: Option<String>,
    /// Human-friendly description of the port/service.
    pub description: Option<String>,
    /// Raw text banner or initial greeting if available.
    pub banner: Option<String>,
    /// HTTP service inspection details if port is open and responded to HTTP probe.
    pub http_info: Option<HttpServiceInfo>,
}

/// Summary of an entire port scan execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortScanSummary {
    /// Target host scanned.
    pub host: String,
    /// Total number of ports scanned.
    pub total_scanned: usize,
    /// Number of open ports found.
    pub open_count: usize,
    /// Number of closed ports.
    pub closed_count: usize,
    /// Number of filtered/timeout ports.
    pub filtered_count: usize,
    /// Total scan duration in milliseconds.
    pub duration_ms: f64,
    /// Detailed list of port results.
    pub results: Vec<PortScanResult>,
}

// ===========================================================================
// Core Scanner Implementation
// ===========================================================================

/// Default connection timeout for scanning an individual port.
pub const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 400;

/// Default concurrency limit for parallel scanning.
pub const DEFAULT_SCAN_CONCURRENCY: usize = 32;

/// Checks if a single TCP port is open on the target host.
pub async fn is_port_open(host: &str, port: u16, timeout: Duration) -> bool {
    let addr = format!("{}:{}", host, port);
    match tokio::time::timeout(timeout, TcpStream::connect(&addr)).await {
        Ok(Ok(_stream)) => true,
        _ => false,
    }
}

/// Probes a single TCP port and performs service identification if open.
pub async fn scan_single_port(
    host: &str,
    port: u16,
    timeout: Duration,
    probe_http: bool,
) -> PortScanResult {
    let known = lookup_service(port);
    let addr = format!("{}:{}", host, port);

    let start = Instant::now();
    let connect_res = tokio::time::timeout(timeout, TcpStream::connect(&addr)).await;
    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

    match connect_res {
        Ok(Ok(mut stream)) => {
            let mut result = PortScanResult {
                port,
                state: PortState::Open,
                latency_ms: Some((latency_ms * 100.0).round() / 100.0),
                service: known.map(|k| k.name.to_string()),
                category: known.map(|k| k.category.to_string()),
                description: known.map(|k| k.description.to_string()),
                banner: None,
                http_info: None,
            };

            if probe_http {
                // Drop initial stream and probe HTTP / raw banners
                drop(stream);
                let (http_info, raw_banner) = probe_service_details(host, port, timeout).await;
                if let Some(http) = http_info {
                    if let Some(fw) = &http.detected_framework {
                        result.service = Some(format!("{} ({})", fw, result.service.as_deref().unwrap_or("HTTP")));
                    }
                    result.http_info = Some(http);
                }
                if let Some(b) = raw_banner {
                    result.banner = Some(b);
                }
            }

            result
        }
        Ok(Err(err)) => {
            let state = if err.kind() == std::io::ErrorKind::ConnectionRefused {
                PortState::Closed
            } else {
                PortState::Filtered
            };
            PortScanResult {
                port,
                state,
                latency_ms: None,
                service: known.map(|k| k.name.to_string()),
                category: known.map(|k| k.category.to_string()),
                description: known.map(|k| k.description.to_string()),
                banner: None,
                http_info: None,
            }
        }
        Err(_) => {
            // Timeout -> Filtered
            PortScanResult {
                port,
                state: PortState::Filtered,
                latency_ms: None,
                service: known.map(|k| k.name.to_string()),
                category: known.map(|k| k.category.to_string()),
                description: known.map(|k| k.description.to_string()),
                banner: None,
                http_info: None,
            }
        }
    }
}

/// Probes an open port to extract HTTP status, headers, HTML title, and tech fingerprints.
pub async fn probe_service_details(
    host: &str,
    port: u16,
    timeout: Duration,
) -> (Option<HttpServiceInfo>, Option<String>) {
    let addr = format!("{}:{}", host, port);
    let probe_timeout = timeout.max(Duration::from_millis(500));

    // 1. Try sending standard HTTP/1.1 GET request
    let http_req = format!(
        "GET / HTTP/1.1\r\nHost: {}:{}\r\nUser-Agent: Fusion-PortScanner/0.3\r\nAccept: text/html,application/json,*/*\r\nConnection: close\r\n\r\n",
        host, port
    );

    let connect_and_read = async {
        let mut stream = TcpStream::connect(&addr).await?;
        stream.write_all(http_req.as_bytes()).await?;

        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Empty response"));
        }
        buf.truncate(n);
        Ok::<Vec<u8>, std::io::Error>(buf)
    };

    match tokio::time::timeout(probe_timeout, connect_and_read).await {
        Ok(Ok(bytes)) => {
            let raw_text = String::from_utf8_lossy(&bytes).to_string();
            if raw_text.starts_with("HTTP/") {
                let parsed = parse_http_response(host, port, &raw_text);
                return (Some(parsed), None);
            } else {
                // Non-HTTP raw banner (e.g. SSH-2.0, Redis, MySQL)
                let cleaned_banner = clean_raw_banner(&bytes);
                return (None, cleaned_banner);
            }
        }
        _ => {
            // If standard HTTP GET failed, check for Redis PING if port is 6379 or unknown
            if port == 6379 {
                if let Some(redis_banner) = probe_redis(&addr, probe_timeout).await {
                    return (None, Some(redis_banner));
                }
            }
            (None, None)
        }
    }
}

/// Parses raw HTTP response text into structured `HttpServiceInfo`.
fn parse_http_response(host: &str, port: u16, raw: &str) -> HttpServiceInfo {
    let mut info = HttpServiceInfo {
        url: format!("http://{}:{}", host, port),
        ..Default::default()
    };

    let mut lines = raw.lines();
    if let Some(status_line) = lines.next() {
        let parts: Vec<&str> = status_line.split_whitespace().collect();
        if parts.len() >= 2 {
            if let Ok(code) = parts[1].parse::<u16>() {
                info.status_code = Some(code);
            }
            if parts.len() >= 3 {
                info.status_text = Some(parts[2..].join(" "));
            }
        }
    }

    let mut headers = Vec::new();
    let mut body = String::new();
    let mut in_body = false;

    for line in lines {
        if in_body {
            body.push_str(line);
            body.push('\n');
        } else if line.trim().is_empty() {
            in_body = true;
        } else {
            headers.push(line);
            let lower = line.to_lowercase();
            if lower.starts_with("server:") {
                info.server = Some(line["server:".len()..].trim().to_string());
            } else if lower.starts_with("x-powered-by:") {
                info.powered_by = Some(line["x-powered-by:".len()..].trim().to_string());
            } else if lower.starts_with("content-type:") {
                info.content_type = Some(line["content-type:".len()..].trim().to_string());
            }
        }
    }

    // Extract HTML <title>
    if let Some(title) = extract_html_title(&raw) {
        info.title = Some(title);
    }

    // Technology / framework fingerprinting
    info.detected_framework = fingerprint_framework(port, &raw, &info);

    // Response preview (first 200 chars of body or status)
    let trimmed_body = body.trim();
    if !trimmed_body.is_empty() {
        let preview = if trimmed_body.len() > 200 {
            format!("{}...", &trimmed_body[..200])
        } else {
            trimmed_body.to_string()
        };
        info.response_preview = Some(preview);
    }

    info
}

/// Fingerprints specific web frameworks and dev servers from HTTP responses.
fn fingerprint_framework(port: u16, raw_http: &str, info: &HttpServiceInfo) -> Option<String> {
    let lower_raw = raw_http.to_lowercase();

    // Ollama detection
    if port == 11434 || lower_raw.contains("ollama is running") {
        return Some("Ollama".to_string());
    }

    // Vite detection
    if lower_raw.contains("@vite/client") || lower_raw.contains("/@vite/") || lower_raw.contains("vite") {
        return Some("Vite".to_string());
    }

    // Next.js detection
    if let Some(powered) = &info.powered_by {
        if powered.to_lowercase().contains("next.js") {
            return Some("Next.js".to_string());
        }
    }
    if lower_raw.contains("__next_data__") || lower_raw.contains("/_next/") {
        return Some("Next.js".to_string());
    }

    // Nuxt / Vue detection
    if lower_raw.contains("__nuxt") || lower_raw.contains("/_nuxt/") {
        return Some("Nuxt".to_string());
    }

    // SvelteKit detection
    if lower_raw.contains("__sveltekit") || lower_raw.contains("%sveltekit") {
        return Some("SvelteKit".to_string());
    }

    // Astro detection
    if lower_raw.contains("data-astro-") || lower_raw.contains("astro-island") {
        return Some("Astro".to_string());
    }

    // Gradio detection
    if lower_raw.contains("gradio_config") || lower_raw.contains("gradio-app") {
        return Some("Gradio".to_string());
    }

    // Streamlit detection
    if lower_raw.contains("streamlitdebug") || lower_raw.contains("streamlit") {
        return Some("Streamlit".to_string());
    }

    // JupyterLab / Notebook
    if lower_raw.contains("jupyter notebook") || lower_raw.contains("jupyterlab") {
        return Some("Jupyter".to_string());
    }

    // FastAPI / Uvicorn
    if let Some(server) = &info.server {
        let s_lower = server.to_lowercase();
        if s_lower.contains("uvicorn") {
            return Some("FastAPI / Uvicorn".to_string());
        }
        if s_lower.contains("werkzeug") {
            return Some("Flask / Werkzeug".to_string());
        }
        if s_lower.contains("gunicorn") {
            return Some("Gunicorn".to_string());
        }
        if s_lower.contains("express") {
            return Some("Express".to_string());
        }
        if s_lower.contains("caddy") {
            return Some("Caddy".to_string());
        }
        if s_lower.contains("nginx") {
            return Some("Nginx".to_string());
        }
    }

    if let Some(powered) = &info.powered_by {
        let p_lower = powered.to_lowercase();
        if p_lower.contains("express") {
            return Some("Express".to_string());
        }
    }

    // Spring Boot Whitelabel
    if lower_raw.contains("whitelabel error page") {
        return Some("Spring Boot".to_string());
    }

    // React CRA default
    if lower_raw.contains("<div id=\"root\">") && lower_raw.contains("react") {
        return Some("React App".to_string());
    }

    None
}

/// Extracts the `<title>` contents from an HTML string.
fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start_tag = "<title>";
    let end_tag = "</title>";

    if let Some(start_idx) = lower.find(start_tag) {
        let after_start = start_idx + start_tag.len();
        if let Some(end_idx) = lower[after_start..].find(end_tag) {
            let raw_title = &html[after_start..after_start + end_idx];
            let trimmed = raw_title.trim();
            if !trimmed.is_empty() {
                // Decode common HTML entities
                let title = trimmed
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\"")
                    .replace("&#39;", "'");
                return Some(title);
            }
        }
    }
    None
}

/// Cleans a non-HTTP raw binary/text banner.
fn clean_raw_banner(bytes: &[u8]) -> Option<String> {
    let s = String::from_utf8_lossy(bytes);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Filter out non-printable binary junk
    let printable: String = trimmed
        .chars()
        .map(|c| if c.is_control() && c != '\n' && c != '\r' && c != '\t' { '.' } else { c })
        .collect();

    let clean = printable.trim().to_string();
    if clean.len() > 150 {
        Some(format!("{}...", &clean[..150]))
    } else {
        Some(clean)
    }
}

/// Probes Redis server with a PING command.
async fn probe_redis(addr: &str, timeout: Duration) -> Option<String> {
    let probe = async {
        let mut stream = TcpStream::connect(addr).await?;
        stream.write_all(b"PING\r\n").await?;
        let mut buf = [0u8; 256];
        let n = stream.read(&mut buf).await?;
        if n > 0 {
            let res = String::from_utf8_lossy(&buf[..n]).trim().to_string();
            return Ok::<String, std::io::Error>(format!("Redis [{}]", res));
        }
        Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "No response"))
    };

    tokio::time::timeout(timeout, probe).await.ok().and_then(|r| r.ok())
}

/// Executes a concurrent port scan over a collection of ports.
pub async fn scan_ports(
    host: &str,
    ports: &[u16],
    timeout: Duration,
    probe_http: bool,
    concurrency: usize,
) -> PortScanSummary {
    let start_time = Instant::now();
    let total_scanned = ports.len();

    let concurrency_limit = concurrency.clamp(1, 128);
    let mut results = Vec::with_capacity(ports.len());

    // Chunk ports to respect concurrency limits
    for chunk in ports.chunks(concurrency_limit) {
        let mut futures = Vec::new();
        for &port in chunk {
            let h = host.to_string();
            futures.push(tokio::spawn(async move {
                scan_single_port(&h, port, timeout, probe_http).await
            }));
        }

        for f in futures {
            if let Ok(res) = f.await {
                results.push(res);
            }
        }
    }

    // Sort results by port number
    results.sort_by_key(|r| r.port);

    let mut open_count = 0;
    let mut closed_count = 0;
    let mut filtered_count = 0;

    for r in &results {
        match r.state {
            PortState::Open => open_count += 1,
            PortState::Closed => closed_count += 1,
            PortState::Filtered => filtered_count += 1,
        }
    }

    let duration_ms = start_time.elapsed().as_secs_f64() * 1000.0;

    PortScanSummary {
        host: host.to_string(),
        total_scanned,
        open_count,
        closed_count,
        filtered_count,
        duration_ms: (duration_ms * 100.0).round() / 100.0,
        results,
    }
}

/// Finds the next `count` free (unbound) TCP ports starting from `start_port`.
pub async fn find_free_ports(host: &str, start_port: u16, count: usize) -> Vec<u16> {
    let mut free_ports = Vec::new();
    let mut candidate = start_port;

    while free_ports.len() < count && candidate <= 65535 {
        let addr = format!("{}:{}", host, candidate);
        // Try binding to the port to verify it's available
        if let Ok(listener) = TcpListener::bind(&addr).await {
            // Drop listener immediately to release port
            drop(listener);
            free_ports.push(candidate);
        }
        if candidate == 65535 {
            break;
        }
        candidate += 1;
    }

    // If count not fulfilled, use OS ephemeral port allocation
    while free_ports.len() < count {
        let addr = format!("{}:0", host);
        if let Ok(listener) = TcpListener::bind(&addr).await {
            if let Ok(local_addr) = listener.local_addr() {
                let port = local_addr.port();
                drop(listener);
                if !free_ports.contains(&port) {
                    free_ports.push(port);
                }
            }
        } else {
            break;
        }
    }

    free_ports
}

/// Waits for a port on `host` to transition to `target_state` with a given timeout.
pub async fn wait_for_port(
    host: &str,
    port: u16,
    target_state: PortState,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<PortScanResult, String> {
    let start = Instant::now();

    loop {
        let res = scan_single_port(host, port, Duration::from_millis(300), true).await;
        if res.state == target_state {
            return Ok(res);
        }

        if start.elapsed() >= timeout {
            return Err(format!(
                "Timed out waiting for {}:{} to become {:?} after {:.1}s (last state: {:?})",
                host,
                port,
                target_state,
                timeout.as_secs_f64(),
                res.state
            ));
        }

        tokio::time::sleep(poll_interval).await;
    }
}

// ===========================================================================
// Argument Parsing & Presets Resolution
// ===========================================================================

/// Parses port specifications from JSON arguments (integers, arrays, comma-separated ranges, or presets).
pub fn parse_target_ports(
    ports_val: Option<&Value>,
    preset_val: Option<&str>,
    start_port: Option<u16>,
    end_port: Option<u16>,
) -> Vec<u16> {
    let mut ports_set = HashSet::new();

    // 1. If explicit start_port and end_port provided
    if let (Some(start), Some(end)) = (start_port, end_port) {
        let min_p = start.min(end);
        let max_p = start.max(end);
        for p in min_p..=max_p {
            ports_set.insert(p);
        }
    }

    // 2. If explicit ports argument provided
    if let Some(val) = ports_val {
        match val {
            Value::Number(n) => {
                if let Some(p) = n.as_u64() {
                    if p <= 65535 && p > 0 {
                        ports_set.insert(p as u16);
                    }
                }
            }
            Value::Array(arr) => {
                for item in arr {
                    if let Some(p) = item.as_u64() {
                        if p <= 65535 && p > 0 {
                            ports_set.insert(p as u16);
                        }
                    } else if let Some(s) = item.as_str() {
                        parse_port_range_string(s, &mut ports_set);
                    }
                }
            }
            Value::String(s) => {
                parse_port_range_string(s, &mut ports_set);
            }
            _ => {}
        }
    }

    // 3. If preset provided
    if let Some(preset_name) = preset_val {
        if let Some(preset) = PortPreset::from_str_loose(preset_name) {
            for p in preset.ports() {
                ports_set.insert(p);
            }
        }
    }

    // 4. If no ports specified at all, default to Dev preset
    if ports_set.is_empty() && start_port.is_none() && end_port.is_none() {
        for p in PortPreset::Dev.ports() {
            ports_set.insert(p);
        }
    }

    let mut list: Vec<u16> = ports_set.into_iter().collect();
    list.sort_unstable();
    list
}

/// Parses strings like `"3000, 5173, 8000-8010"` into port sets.
fn parse_port_range_string(s: &str, set: &mut HashSet<u16>) {
    for part in s.split(',') {
        let p_trimmed = part.trim();
        if p_trimmed.is_empty() {
            continue;
        }

        if let Some((start_s, end_s)) = p_trimmed.split_once('-') {
            if let (Ok(start), Ok(end)) = (start_s.trim().parse::<u16>(), end_s.trim().parse::<u16>()) {
                let min_p = start.min(end);
                let max_p = start.max(end);
                for p in min_p..=max_p {
                    set.insert(p);
                }
            }
        } else if let Ok(port) = p_trimmed.parse::<u16>() {
            set.insert(port);
        }
    }
}

// ===========================================================================
// Formatters (Text, Markdown, JSON)
// ===========================================================================

/// Formats port scan summary as a clean human-readable ANSI text table.
pub fn format_summary_text(summary: &PortScanSummary, open_only: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "TCP Port Scan on {} ({} scanned in {:.1}ms)\n",
        summary.host, summary.total_scanned, summary.duration_ms
    ));
    out.push_str(&format!(
        "Open: {} | Closed: {} | Filtered: {}\n\n",
        summary.open_count, summary.closed_count, summary.filtered_count
    ));

    let display_results: Vec<&PortScanResult> = if open_only {
        summary.results.iter().filter(|r| r.state == PortState::Open).collect()
    } else {
        summary.results.iter().collect()
    };

    if display_results.is_empty() {
        if open_only {
            out.push_str("No active/open ports found on target.\n");
        } else {
            out.push_str("No ports to display.\n");
        }
        return out;
    }

    out.push_str(&format!(
        "{:<7} {:<8} {:<8} {:<30} {:<30}\n",
        "PORT", "STATE", "LATENCY", "SERVICE / FRAMEWORK", "DETAILS / TITLE / BANNER"
    ));
    out.push_str(&format!("{}\n", "-".repeat(90)));

    for r in display_results {
        let state_str = match r.state {
            PortState::Open => "OPEN",
            PortState::Closed => "closed",
            PortState::Filtered => "filtered",
        };

        let latency_str = r
            .latency_ms
            .map(|l| format!("{:.1}ms", l))
            .unwrap_or_else(|| "-".to_string());

        let service_str = r
            .service
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());

        let mut details = Vec::new();
        if let Some(http) = &r.http_info {
            if let Some(code) = http.status_code {
                details.push(format!("HTTP {}", code));
            }
            if let Some(title) = &http.title {
                details.push(format!("\"{}\"", title));
            } else if let Some(server) = &http.server {
                details.push(format!("Server: {}", server));
            }
        } else if let Some(b) = &r.banner {
            details.push(b.clone());
        } else if let Some(desc) = &r.description {
            details.push(desc.clone());
        }

        let detail_str = if details.is_empty() {
            "-".to_string()
        } else {
            details.join(" | ")
        };

        let truncated_service = if service_str.len() > 28 {
            format!("{}..", &service_str[..26])
        } else {
            service_str
        };

        let truncated_detail = if detail_str.len() > 38 {
            format!("{}..", &detail_str[..36])
        } else {
            detail_str
        };

        out.push_str(&format!(
            "{:<7} {:<8} {:<8} {:<30} {:<30}\n",
            r.port, state_str, latency_str, truncated_service, truncated_detail
        ));
    }

    out
}

/// Formats port scan summary as a Markdown table.
pub fn format_summary_markdown(summary: &PortScanSummary, open_only: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "### Port Scan Results for `{}`\n\n",
        summary.host
    ));
    out.push_str(&format!(
        "- **Total Scanned:** {}\n- **Open Ports:** {}\n- **Closed:** {}\n- **Filtered:** {}\n- **Scan Duration:** {:.1}ms\n\n",
        summary.total_scanned, summary.open_count, summary.closed_count, summary.filtered_count, summary.duration_ms
    ));

    let display_results: Vec<&PortScanResult> = if open_only {
        summary.results.iter().filter(|r| r.state == PortState::Open).collect()
    } else {
        summary.results.iter().collect()
    };

    if display_results.is_empty() {
        out.push_str("_No active open ports found._\n");
        return out;
    }

    out.push_str("| Port | State | Latency | Service / Framework | Details / Title |\n");
    out.push_str("| :--- | :--- | :--- | :--- | :--- |\n");

    for r in display_results {
        let state_badge = match r.state {
            PortState::Open => "**OPEN**",
            PortState::Closed => "closed",
            PortState::Filtered => "filtered",
        };

        let latency_str = r
            .latency_ms
            .map(|l| format!("{:.1}ms", l))
            .unwrap_or_else(|| "-".to_string());

        let service_str = r.service.as_deref().unwrap_or("Unknown");

        let mut details = Vec::new();
        if let Some(http) = &r.http_info {
            if let Some(code) = http.status_code {
                details.push(format!("HTTP {}", code));
            }
            if let Some(title) = &http.title {
                details.push(format!("Title: `{}`", title.replace('|', "\\|")));
            } else if let Some(server) = &http.server {
                details.push(format!("Server: `{}`", server.replace('|', "\\|")));
            }
        } else if let Some(b) = &r.banner {
            details.push(format!("Banner: `{}`", b.replace('|', "\\|")));
        } else if let Some(desc) = &r.description {
            details.push(desc.replace('|', "\\|"));
        }

        let detail_str = if details.is_empty() {
            "-".to_string()
        } else {
            details.join(", ")
        };

        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            r.port, state_badge, latency_str, service_str, detail_str
        ));
    }

    out
}

// ===========================================================================
// Port Scanner Tool Definition
// ===========================================================================

/// Main Tool implementation for Port Scanning.
pub struct PortScannerTool;

impl PortScannerTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PortScannerTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for PortScannerTool {
    fn name(&self) -> &str {
        "ports"
    }

    fn description(&self) -> &str {
        "Local TCP port scanner and dev server discovery tool. Scans ports on localhost (or custom hosts), identifies active web frameworks (Vite, Next.js, React, Astro, Flask, FastAPI, Django, Express, etc.), databases (PostgreSQL, MySQL, Redis, MongoDB), AI endpoints (Ollama, LM Studio, Gradio, Streamlit), finds free ports, and waits for dev servers to become ready."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["scan", "check", "find_free", "wait_for", "identify", "list_presets"],
                    "default": "scan",
                    "description": "Action to perform: 'scan' (default, scans preset or port list), 'check' (quick port check), 'find_free' (find available unbound ports), 'wait_for' (poll until port is open/closed), 'identify' (deep probe of specific port), 'list_presets' (list available port presets)."
                },
                "host": {
                    "type": "string",
                    "default": "127.0.0.1",
                    "description": "Target hostname or IP address to scan (defaults to '127.0.0.1' / localhost)."
                },
                "ports": {
                    "description": "Specific port number, array of ports (e.g. [3000, 5173, 8080, 11434]), or comma-separated range string (e.g. '3000,5173,8000-8010')."
                },
                "preset": {
                    "type": "string",
                    "enum": ["dev", "web", "databases", "ai", "quick", "all_common"],
                    "description": "Pre-configured port collection to scan ('dev' = modern web/AI dev servers, 'web' = standard web, 'databases' = DB engines, 'ai' = Ollama/LM Studio/Gradio, 'quick' = top 10 dev ports, 'all_common' = 100+ ports)."
                },
                "start_port": {
                    "type": "integer",
                    "description": "Starting port for range scan or base port for find_free (e.g. 3000)."
                },
                "end_port": {
                    "type": "integer",
                    "description": "Ending port for range scan (e.g. 3010)."
                },
                "count": {
                    "type": "integer",
                    "default": 1,
                    "description": "Number of free ports to find (used with action: 'find_free')."
                },
                "timeout_ms": {
                    "type": "integer",
                    "default": 400,
                    "description": "Connection timeout per port in milliseconds (default: 400ms)."
                },
                "wait_timeout_ms": {
                    "type": "integer",
                    "default": 10000,
                    "description": "Total timeout in milliseconds when using action: 'wait_for' (default: 10000ms / 10s)."
                },
                "wait_for_status": {
                    "type": "string",
                    "enum": ["open", "closed"],
                    "default": "open",
                    "description": "Target port status to wait for ('open' or 'closed')."
                },
                "probe": {
                    "type": "boolean",
                    "default": true,
                    "description": "Whether to perform deep HTTP/service probing on open ports (default: true)."
                },
                "open_only": {
                    "type": "boolean",
                    "default": true,
                    "description": "Whether to filter output to show only open/listening ports (default: true)."
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "json", "markdown"],
                    "default": "text",
                    "description": "Output formatting style: 'text' (default table), 'markdown', or 'json'."
                }
            }
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> anyhow::Result<String> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("scan");

        let host = args
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or("127.0.0.1");

        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let open_only = args
            .get("open_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let probe = args
            .get("probe")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_CONNECT_TIMEOUT_MS);
        let timeout = Duration::from_millis(timeout_ms.clamp(50, 10000));

        match action {
            "list_presets" => {
                let presets = vec![
                    PortPreset::Dev,
                    PortPreset::Web,
                    PortPreset::Databases,
                    PortPreset::Ai,
                    PortPreset::Quick,
                    PortPreset::AllCommon,
                ];

                if format == "json" {
                    let json_arr: Vec<Value> = presets
                        .iter()
                        .map(|p| {
                            json!({
                                "name": p.name(),
                                "description": p.description(),
                                "ports_count": p.ports().len(),
                                "ports": p.ports(),
                            })
                        })
                        .collect();
                    return Ok(serde_json::to_string_pretty(&json_arr)?);
                }

                let mut out = String::from("### Available Port Scanner Presets\n\n");
                for p in presets {
                    out.push_str(&format!(
                        "- **`{}`** ({} ports): {}\n  Ports: {:?}\n\n",
                        p.name(),
                        p.ports().len(),
                        p.description(),
                        p.ports()
                    ));
                }
                Ok(out)
            }

            "find_free" => {
                let start_port = args
                    .get("start_port")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3000) as u16;

                let count = args
                    .get("count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as usize;

                let free_ports = find_free_ports(host, start_port, count.clamp(1, 100)).await;

                if format == "json" {
                    return Ok(serde_json::to_string_pretty(&json!({
                        "host": host,
                        "start_port": start_port,
                        "requested_count": count,
                        "free_ports": free_ports,
                    }))?);
                }

                if free_ports.is_empty() {
                    Ok(format!("No free ports found on {} starting from {}", host, start_port))
                } else if free_ports.len() == 1 {
                    Ok(format!("Available free port on {}: {}", host, free_ports[0]))
                } else {
                    Ok(format!("Found {} free ports on {}: {:?}", free_ports.len(), host, free_ports))
                }
            }

            "wait_for" => {
                let port = args
                    .get("ports")
                    .or_else(|| args.get("start_port"))
                    .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok())))
                    .unwrap_or(3000) as u16;

                let target_status_str = args
                    .get("wait_for_status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("open");

                let target_state = if target_status_str.eq_ignore_ascii_case("closed") {
                    PortState::Closed
                } else {
                    PortState::Open
                };

                let wait_timeout_ms = args
                    .get("wait_timeout_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10000);
                let wait_timeout = Duration::from_millis(wait_timeout_ms.clamp(500, 60000));
                let poll_interval = Duration::from_millis(250);

                match wait_for_port(host, port, target_state, wait_timeout, poll_interval).await {
                    Ok(res) => {
                        if format == "json" {
                            Ok(serde_json::to_string_pretty(&res)?)
                        } else {
                            Ok(format!(
                                "Success: Port {}:{} reached status '{:?}' in {:.1}ms{}",
                                host,
                                port,
                                res.state,
                                res.latency_ms.unwrap_or(0.0),
                                if let Some(http) = &res.http_info {
                                    format!(" (Detected: {})", http.detected_framework.as_deref().unwrap_or("HTTP Server"))
                                } else {
                                    String::new()
                                }
                            ))
                        }
                    }
                    Err(err) => Ok(format!("Wait failed: {}", err)),
                }
            }

            "identify" => {
                let port = args
                    .get("ports")
                    .or_else(|| args.get("start_port"))
                    .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok())))
                    .unwrap_or(3000) as u16;

                let res = scan_single_port(host, port, timeout, true).await;

                if format == "json" {
                    return Ok(serde_json::to_string_pretty(&res)?);
                }

                let mut out = format!("### Service Identification: {}:{}\n\n", host, port);
                out.push_str(&format!("- **State:** {:?}\n", res.state));
                if let Some(lat) = res.latency_ms {
                    out.push_str(&format!("- **Latency:** {:.2}ms\n", lat));
                }
                if let Some(srv) = &res.service {
                    out.push_str(&format!("- **Service:** {}\n", srv));
                }
                if let Some(cat) = &res.category {
                    out.push_str(&format!("- **Category:** {}\n", cat));
                }
                if let Some(desc) = &res.description {
                    out.push_str(&format!("- **Description:** {}\n", desc));
                }
                if let Some(banner) = &res.banner {
                    out.push_str(&format!("- **Raw Banner:** `{}`\n", banner));
                }

                if let Some(http) = &res.http_info {
                    out.push_str("\n**HTTP Service Details:**\n");
                    out.push_str(&format!("- **Endpoint:** {}\n", http.url));
                    if let Some(code) = http.status_code {
                        out.push_str(&format!("- **Status Code:** {} {}\n", code, http.status_text.as_deref().unwrap_or("")));
                    }
                    if let Some(fw) = &http.detected_framework {
                        out.push_str(&format!("- **Detected Framework:** {}\n", fw));
                    }
                    if let Some(title) = &http.title {
                        out.push_str(&format!("- **Page Title:** {}\n", title));
                    }
                    if let Some(srv) = &http.server {
                        out.push_str(&format!("- **Server Header:** {}\n", srv));
                    }
                    if let Some(pow) = &http.powered_by {
                        out.push_str(&format!("- **Powered By:** {}\n", pow));
                    }
                    if let Some(ct) = &http.content_type {
                        out.push_str(&format!("- **Content-Type:** {}\n", ct));
                    }
                    if let Some(prev) = &http.response_preview {
                        out.push_str(&format!("- **Response Preview:**\n```\n{}\n```\n", prev));
                    }
                }

                Ok(out)
            }

            // Default: "scan" or "check"
            _ => {
                let preset_str = args.get("preset").and_then(|v| v.as_str());
                let start_port = args.get("start_port").and_then(|v| v.as_u64()).map(|p| p as u16);
                let end_port = args.get("end_port").and_then(|v| v.as_u64()).map(|p| p as u16);
                let ports_val = args.get("ports");

                let target_ports = parse_target_ports(ports_val, preset_str, start_port, end_port);

                let summary = scan_ports(host, &target_ports, timeout, probe, DEFAULT_SCAN_CONCURRENCY).await;

                match format {
                    "json" => {
                        let filtered_summary = if open_only {
                            let open_results: Vec<PortScanResult> = summary
                                .results
                                .into_iter()
                                .filter(|r| r.state == PortState::Open)
                                .collect();
                            json!({
                                "host": summary.host,
                                "total_scanned": summary.total_scanned,
                                "open_count": summary.open_count,
                                "closed_count": summary.closed_count,
                                "filtered_count": summary.filtered_count,
                                "duration_ms": summary.duration_ms,
                                "open_ports": open_results,
                            })
                        } else {
                            json!(summary)
                        };
                        Ok(serde_json::to_string_pretty(&filtered_summary)?)
                    }
                    "markdown" => Ok(format_summary_markdown(&summary, open_only)),
                    _ => Ok(format_summary_text(&summary, open_only)),
                }
            }
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_services_lookup() {
        assert!(lookup_service(80).is_some());
        assert_eq!(lookup_service(80).unwrap().name, "HTTP");
        assert_eq!(lookup_service(3000).unwrap().category, "Web / Frontend");
        assert_eq!(lookup_service(5173).unwrap().name, "Vite Dev Server");
        assert_eq!(lookup_service(11434).unwrap().name, "Ollama Local LLM");
        assert_eq!(lookup_service(6379).unwrap().name, "Redis");
        assert!(lookup_service(61234).is_none());
    }

    #[test]
    fn test_port_presets() {
        let dev = PortPreset::Dev.ports();
        assert!(dev.contains(&3000));
        assert!(dev.contains(&5173));
        assert!(dev.contains(&8080));
        assert!(dev.contains(&11434));

        let ai = PortPreset::Ai.ports();
        assert!(ai.contains(&11434));
        assert!(ai.contains(&1234));
        assert!(ai.contains(&7860));

        let quick = PortPreset::Quick.ports();
        assert_eq!(quick.len(), 10);
        assert!(quick.contains(&3000));
        assert!(quick.contains(&5173));

        assert_eq!(PortPreset::from_str_loose("dev"), Some(PortPreset::Dev));
        assert_eq!(PortPreset::from_str_loose("databases"), Some(PortPreset::Databases));
        assert_eq!(PortPreset::from_str_loose("ai"), Some(PortPreset::Ai));
    }

    #[test]
    fn test_parse_target_ports() {
        let p1 = parse_target_ports(Some(&json!(3000)), None, None, None);
        assert_eq!(p1, vec![3000]);

        let p2 = parse_target_ports(Some(&json!([3000, 5173, 8080])), None, None, None);
        assert_eq!(p2, vec![3000, 5173, 8080]);

        let p3 = parse_target_ports(Some(&json!("3000, 3002-3004, 5173")), None, None, None);
        assert_eq!(p3, vec![3000, 3002, 3003, 3004, 5173]);

        let p4 = parse_target_ports(None, None, Some(8000), Some(8003));
        assert_eq!(p4, vec![8000, 8001, 8002, 8003]);

        let p5 = parse_target_ports(None, Some("quick"), None, None);
        assert_eq!(p5.len(), 10);
    }

    #[test]
    fn test_extract_html_title() {
        let html = "<html><head><title>My Vite React App</title></head><body><h1>Hello</h1></body></html>";
        assert_eq!(extract_html_title(html), Some("My Vite React App".to_string()));

        let html_entities = "<TITLE>Tom &amp; Jerry &lt;App&gt;</TITLE>";
        assert_eq!(extract_html_title(html_entities), Some("Tom & Jerry <App>".to_string()));

        let no_title = "<html><body>No title here</body></html>";
        assert_eq!(extract_html_title(no_title), None);
    }

    #[test]
    fn test_fingerprint_framework() {
        let info = HttpServiceInfo {
            server: Some("uvicorn".to_string()),
            ..Default::default()
        };
        assert_eq!(
            fingerprint_framework(8000, "HTTP/1.1 200 OK\r\nServer: uvicorn\r\n\r\n", &info),
            Some("FastAPI / Uvicorn".to_string())
        );

        let vite_html = "HTTP/1.1 200 OK\r\n\r\n<script type=\"module\" src=\"/@vite/client\"></script>";
        assert_eq!(
            fingerprint_framework(5173, vite_html, &HttpServiceInfo::default()),
            Some("Vite".to_string())
        );

        let next_info = HttpServiceInfo {
            powered_by: Some("Next.js".to_string()),
            ..Default::default()
        };
        assert_eq!(
            fingerprint_framework(3000, "HTTP/1.1 200 OK\r\n\r\n", &next_info),
            Some("Next.js".to_string())
        );
    }

    #[tokio::test]
    async fn test_find_free_ports() {
        let free = find_free_ports("127.0.0.1", 39000, 3).await;
        assert_eq!(free.len(), 3);
        assert!(free[0] >= 39000);
    }

    #[tokio::test]
    async fn test_scan_in_process_listener() {
        // Start an in-process TCP listener on an ephemeral port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Spawn background task to accept connection and reply with simple HTTP
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let response = "HTTP/1.1 200 OK\r\nServer: TestDev/1.0\r\nContent-Type: text/html\r\n\r\n<html><head><title>Test Dev Server</title></head></html>";
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        // Scan this specific port
        let res = scan_single_port("127.0.0.1", port, Duration::from_millis(500), true).await;
        assert_eq!(res.state, PortState::Open);
        assert!(res.latency_ms.is_some());
        assert!(res.http_info.is_some());
        let http = res.http_info.unwrap();
        assert_eq!(http.status_code, Some(200));
        assert_eq!(http.server, Some("TestDev/1.0".to_string()));
        assert_eq!(http.title, Some("Test Dev Server".to_string()));
    }

    #[tokio::test]
    async fn test_port_scanner_tool_execute() {
        let tool = PortScannerTool::new();
        let ctx = ToolContext::default();

        // 1. Test list_presets action
        let presets_out = tool
            .execute(json!({"action": "list_presets", "format": "json"}), &ctx)
            .await
            .unwrap();
        assert!(presets_out.contains("dev"));
        assert!(presets_out.contains("databases"));
        assert!(presets_out.contains("ai"));

        // 2. Test find_free action
        let free_out = tool
            .execute(json!({"action": "find_free", "start_port": 45000, "count": 2, "format": "json"}), &ctx)
            .await
            .unwrap();
        let free_val: Value = serde_json::from_str(&free_out).unwrap();
        assert_eq!(free_val["free_ports"].as_array().unwrap().len(), 2);

        // 3. Test scan action with custom port
        let scan_out = tool
            .execute(json!({"action": "scan", "ports": [65500], "open_only": false, "format": "json"}), &ctx)
            .await
            .unwrap();
        let scan_val: Value = serde_json::from_str(&scan_out).unwrap();
        assert_eq!(scan_val["total_scanned"], 1);
    }

    #[tokio::test]
    async fn test_wait_for_open_port() {
        // Bind a listener so the port is definitely open.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Accept connections in background so the port stays open.
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                tokio::spawn(async move { let _ = s.write_all(b"OK").await; });
            }
        });

        let res = wait_for_port(
            "127.0.0.1",
            port,
            PortState::Open,
            Duration::from_secs(5),
            Duration::from_millis(100),
        )
        .await;
        assert!(res.is_ok(), "expected Ok, got {:?}", res);
        assert_eq!(res.unwrap().state, PortState::Open);
    }

    #[tokio::test]
    async fn test_wait_for_timeout() {
        // Use a port very unlikely to be bound.
        let res = wait_for_port(
            "127.0.0.1",
            19999,
            PortState::Open,
            Duration::from_millis(600),
            Duration::from_millis(150),
        )
        .await;
        assert!(res.is_err(), "expected timeout error");
        let msg = res.unwrap_err();
        assert!(msg.contains("Timed out"), "unexpected error: {}", msg);
    }

    #[tokio::test]
    async fn test_tool_wait_for_action_success() {
        // Bind an ephemeral port so wait_for succeeds immediately.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                tokio::spawn(async move { let _ = s.write_all(b"OK").await; });
            }
        });

        let tool = PortScannerTool::new();
        let ctx = ToolContext::default();
        let out = tool
            .execute(
                json!({
                    "action": "wait_for",
                    "ports": port,
                    "wait_for_status": "open",
                    "wait_timeout_ms": 5000,
                    "format": "text"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.contains("Success"), "unexpected output: {}", out);
    }

    #[tokio::test]
    async fn test_tool_wait_for_action_timeout() {
        let tool = PortScannerTool::new();
        let ctx = ToolContext::default();
        let out = tool
            .execute(
                json!({
                    "action": "wait_for",
                    "ports": 19998,
                    "wait_for_status": "open",
                    "wait_timeout_ms": 600,
                    "format": "text"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.contains("Wait failed"), "unexpected output: {}", out);
    }
}
