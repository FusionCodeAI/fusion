use fusion::tools::lsp::config::{find_server_for_file, get_default_servers};
use std::path::Path;

#[test]
fn test_default_servers_parse() {
    let servers = get_default_servers();
    assert!(
        servers.len() >= 50,
        "expected at least 50 servers from defaults.json, got {}",
        servers.len()
    );

    // Verify rust-analyzer
    let rust_server = servers
        .get("rust-analyzer")
        .expect("rust-analyzer should be present");
    assert_eq!(rust_server.command, "rust-analyzer");
    assert!(rust_server.file_types.contains(&".rs".to_string()));
    assert!(rust_server.root_markers.contains(&"Cargo.toml".to_string()));
    assert!(rust_server.settings.get("rust-analyzer").is_some());

    // Verify gopls
    let go_server = servers.get("gopls").expect("gopls should be present");
    assert_eq!(go_server.command, "gopls");
    assert!(go_server.args.contains(&"serve".to_string()));
    assert!(go_server.file_types.contains(&".go".to_string()));
    assert!(go_server.root_markers.contains(&"go.mod".to_string()));

    // Verify typescript-language-server
    let ts_server = servers
        .get("typescript-language-server")
        .expect("typescript-language-server should be present");
    assert_eq!(ts_server.command, "typescript-language-server");
    assert!(ts_server.args.contains(&"--stdio".to_string()));
    assert!(ts_server.file_types.contains(&".ts".to_string()));
    assert!(ts_server.root_markers.contains(&"package.json".to_string()));

    // Verify pyright
    let py_server = servers.get("pyright").expect("pyright should be present");
    assert_eq!(py_server.command, "pyright-langserver");
    assert!(py_server.args.contains(&"--stdio".to_string()));
    assert!(py_server.file_types.contains(&".py".to_string()));
    assert!(py_server
        .root_markers
        .contains(&"pyproject.toml".to_string()));
}

#[test]
fn test_find_server_for_file_rust() {
    let server_dot = find_server_for_file(Path::new(".rs")).expect("should find server for .rs");
    assert_eq!(server_dot.command, "rust-analyzer");

    let server_path =
        find_server_for_file(Path::new("src/main.rs")).expect("should find server for src/main.rs");
    assert_eq!(server_path.command, "rust-analyzer");
}

#[test]
fn test_find_server_for_file_go() {
    let server_dot = find_server_for_file(Path::new(".go")).expect("should find server for .go");
    assert_eq!(server_dot.command, "gopls");

    let server_path = find_server_for_file(Path::new("cmd/server/main.go"))
        .expect("should find server for main.go");
    assert_eq!(server_path.command, "gopls");
}

#[test]
fn test_find_server_for_file_typescript() {
    let server_dot = find_server_for_file(Path::new(".ts")).expect("should find server for .ts");
    assert_eq!(server_dot.command, "typescript-language-server");

    let server_path = find_server_for_file(Path::new("packages/web/index.ts"))
        .expect("should find server for index.ts");
    assert_eq!(server_path.command, "typescript-language-server");
}

#[test]
fn test_find_server_for_file_python() {
    let server_dot = find_server_for_file(Path::new(".py")).expect("should find server for .py");
    assert_eq!(server_dot.command, "pyright-langserver");

    let server_path =
        find_server_for_file(Path::new("scripts/run.py")).expect("should find server for run.py");
    assert_eq!(server_path.command, "pyright-langserver");
}

#[test]
fn test_find_server_for_file_unknown() {
    let server = find_server_for_file(Path::new("unknown.nonexistentextension123"));
    assert!(server.is_none());
}
