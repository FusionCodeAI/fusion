//! Integration tests for `fusion-walker`.
//!
//! Comprehensive test suite verifying:
//! 1. `WalkRequest` with `.gitignore` filtering (basic rules, negations, nested .gitignore, toggle flag, .git and node_modules pruning)
//! 2. Directory error handling (`DirectoryErrorMode::SkipSkippable` vs `DirectoryErrorMode::Visit`, nonexistent roots, permission errors)
//! 3. Depth bounds (`depth(min, max)`, shallow walks, range bounds, root emission, inverted bounds)
//! 4. High-level filters and collectors (`collect_files`, `collect_dirs`, `WalkFilter`, `limit`)

use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

use fusion_walker::{
    CompiledWalkGlob, DirectoryError, DirectoryErrorMode, FileType, FollowLinks, WalkDecision,
    WalkError, WalkFilter, WalkOrder, WalkRequest, WalkStatus,
};

/// Helper to create parent directory and write file contents.
fn write_file(path: impl AsRef<Path>, content: impl AsRef<[u8]>) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("failed to create parent directories");
    }
    fs::write(path, content).expect("failed to write file");
}

/// Helper to get sorted relative paths from a walk outcome.
fn collect_sorted_paths(request: &WalkRequest) -> Vec<String> {
    let outcome = request.collect().expect("walk collect should succeed");
    let mut paths: Vec<String> = outcome.entries.into_iter().map(|e| e.path).collect();
    paths.sort();
    paths
}

// ============================================================================
// 1. WalkRequest with gitignore filtering
// ============================================================================

#[test]
fn test_gitignore_basic_filtering() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    // Create project structure
    write_file(root.join("src/main.rs"), "fn main() {}");
    write_file(root.join("src/lib.rs"), "pub fn lib() {}");
    write_file(root.join("README.md"), "# Fusion Project");
    write_file(root.join("target/debug/app"), "binary");
    write_file(root.join("target/release/app"), "binary");
    write_file(root.join("logs/debug.log"), "log line");
    write_file(root.join("app.log"), "root log line");

    // Create root .gitignore
    write_file(
        root.join(".gitignore"),
        "target/\n*.log\nlogs/\n",
    );

    // Walk with gitignore enabled
    let request = WalkRequest::new(root)
        .gitignore(true)
        .hidden(false)
        .order(WalkOrder::Path);

    let paths = collect_sorted_paths(&request);

    assert!(paths.contains(&"src".to_string()));
    assert!(paths.contains(&"src/main.rs".to_string()));
    assert!(paths.contains(&"src/lib.rs".to_string()));
    assert!(paths.contains(&"README.md".to_string()));

    // Ignored paths must not be present
    assert!(!paths.contains(&"target".to_string()));
    assert!(!paths.contains(&"target/debug/app".to_string()));
    assert!(!paths.contains(&"target/release/app".to_string()));
    assert!(!paths.contains(&"logs".to_string()));
    assert!(!paths.contains(&"logs/debug.log".to_string()));
    assert!(!paths.contains(&"app.log".to_string()));
}

#[test]
fn test_gitignore_toggle_disabled() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("src/main.rs"), "fn main() {}");
    write_file(root.join("target/debug/app"), "binary");
    write_file(root.join("debug.log"), "log");
    write_file(root.join(".gitignore"), "target/\n*.log\n");

    // Walk with gitignore disabled
    let request = WalkRequest::new(root)
        .gitignore(false)
        .hidden(false);

    let paths = collect_sorted_paths(&request);

    // With gitignore disabled, target and debug.log should be collected
    assert!(paths.contains(&"src/main.rs".to_string()));
    assert!(paths.contains(&"target/debug/app".to_string()));
    assert!(paths.contains(&"debug.log".to_string()));
}

#[test]
fn test_gitignore_negation_and_whitelisting() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("docs/normal.txt"), "normal doc");
    write_file(root.join("docs/important.txt"), "important doc");
    write_file(root.join("notes.txt"), "notes");
    write_file(root.join("keep.txt"), "keep");
    write_file(root.join("src/code.rs"), "fn code() {}");

    // Ignore all .txt files except keep.txt and docs/important.txt
    write_file(
        root.join(".gitignore"),
        "*.txt\n!keep.txt\n!docs/important.txt\n",
    );

    let request = WalkRequest::new(root)
        .gitignore(true)
        .hidden(false);

    let paths = collect_sorted_paths(&request);

    assert!(paths.contains(&"src/code.rs".to_string()));
    assert!(paths.contains(&"keep.txt".to_string()), "keep.txt should be whitelisted");
    assert!(
        paths.contains(&"docs/important.txt".to_string()),
        "docs/important.txt should be whitelisted"
    );

    assert!(
        !paths.contains(&"notes.txt".to_string()),
        "notes.txt should be ignored"
    );
    assert!(
        !paths.contains(&"docs/normal.txt".to_string()),
        "docs/normal.txt should be ignored"
    );
}

#[test]
fn test_nested_gitignore_hierarchical_filtering() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    // Root level files
    write_file(root.join("root.tmp"), "temp");
    write_file(root.join("root.txt"), "text");
    write_file(root.join(".gitignore"), "*.tmp\n");

    // Subpackage level files
    write_file(root.join("packages/sub/sub.tmp"), "temp in sub");
    write_file(root.join("packages/sub/sub.data"), "data in sub");
    write_file(root.join("packages/sub/special.data"), "special data in sub");
    write_file(root.join("packages/sub/sub.txt"), "text in sub");

    // Nested .gitignore in packages/sub/
    write_file(
        root.join("packages/sub/.gitignore"),
        "*.data\n!special.data\n",
    );

    let request = WalkRequest::new(root)
        .gitignore(true)
        .hidden(false);

    let paths = collect_sorted_paths(&request);

    // Root gitignore *.tmp rule applies across entire tree
    assert!(!paths.contains(&"root.tmp".to_string()));
    assert!(!paths.contains(&"packages/sub/sub.tmp".to_string()));

    // Nested gitignore rule *.data applies in packages/sub
    assert!(!paths.contains(&"packages/sub/sub.data".to_string()));

    // Nested negation !special.data keeps special.data
    assert!(paths.contains(&"packages/sub/special.data".to_string()));

    // Unignored text files are preserved
    assert!(paths.contains(&"root.txt".to_string()));
    assert!(paths.contains(&"packages/sub/sub.txt".to_string()));
}

#[test]
fn test_git_directory_pruning() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("main.rs"), "code");
    write_file(root.join(".git/config"), "core");
    write_file(root.join(".git/HEAD"), "ref");
    write_file(root.join(".git/objects/00/abc"), "blob");

    // With skip_git(true) and hidden(true), .git must be completely omitted
    let request_skip = WalkRequest::new(root)
        .hidden(true)
        .skip_git(true);

    let paths_skip = collect_sorted_paths(&request_skip);
    assert!(paths_skip.contains(&"main.rs".to_string()));
    assert!(!paths_skip.iter().any(|p| p.starts_with(".git")));

    // With skip_git(false) and hidden(true), .git contents are visited
    let request_keep = WalkRequest::new(root)
        .hidden(true)
        .skip_git(false);

    let paths_keep = collect_sorted_paths(&request_keep);
    assert!(paths_keep.contains(&"main.rs".to_string()));
    assert!(paths_keep.iter().any(|p| p.starts_with(".git")));
}

#[test]
fn test_node_modules_pruning() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("package.json"), "{}");
    write_file(root.join("index.ts"), "export {}");
    write_file(root.join("node_modules/foo/package.json"), "{}");
    write_file(root.join("node_modules/foo/index.js"), "");

    let request_pruned = WalkRequest::new(root)
        .skip_node_modules(true);

    let paths_pruned = collect_sorted_paths(&request_pruned);
    assert!(paths_pruned.contains(&"package.json".to_string()));
    assert!(paths_pruned.contains(&"index.ts".to_string()));
    assert!(!paths_pruned.iter().any(|p| p.starts_with("node_modules")));

    let request_unpruned = WalkRequest::new(root)
        .skip_node_modules(false);

    let paths_unpruned = collect_sorted_paths(&request_unpruned);
    assert!(paths_unpruned.iter().any(|p| p.starts_with("node_modules")));
}

// ============================================================================
// 2. Directory Error Handling
// ============================================================================

#[test]
fn test_directory_error_nonexistent_root() {
    let non_existent = PathBuf::from("/tmp/fusion_non_existent_dir_walker_test_99999");
    let request = WalkRequest::new(&non_existent);

    let result = request.collect();
    assert!(result.is_err(), "walking a nonexistent root directory must fail");

    match result.unwrap_err() {
        WalkError::InvalidData { path, message } => {
            assert_eq!(path, non_existent);
            assert!(!message.is_empty(), "error message should not be empty");
        }
        WalkError::Interrupted(_) => {
            panic!("expected InvalidData error for nonexistent root, got Interrupted");
        }
    }
}

#[cfg(unix)]
#[test]
fn test_directory_error_mode_skip_skippable() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("public/readme.txt"), "hello");
    let restricted_dir = root.join("restricted");
    fs::create_dir_all(&restricted_dir).expect("failed to create restricted dir");
    write_file(restricted_dir.join("secret.txt"), "secret");
    write_file(root.join("public_after/note.txt"), "world");

    // Make restricted_dir inaccessible
    let mut perms = fs::metadata(&restricted_dir).unwrap().permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&restricted_dir, perms).expect("failed to restrict permissions");

    // Guard to restore permissions on drop so tempdir cleanup succeeds
    struct RestorePerms(PathBuf);
    impl Drop for RestorePerms {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        }
    }
    let _guard = RestorePerms(restricted_dir.clone());

    // In SkipSkippable mode, the walker silently ignores permission errors on subdirectories
    let request = WalkRequest::new(root)
        .directory_errors(DirectoryErrorMode::SkipSkippable);

    let outcome = request.collect();
    assert!(
        outcome.is_ok(),
        "SkipSkippable should not abort on unreadable subdirectories"
    );

    let paths: Vec<String> = outcome.unwrap().entries.into_iter().map(|e| e.path).collect();
    assert!(paths.contains(&"public/readme.txt".to_string()));
    assert!(paths.contains(&"public_after/note.txt".to_string()));
    assert!(!paths.contains(&"restricted/secret.txt".to_string()));
}

#[cfg(unix)]
#[test]
fn test_directory_error_mode_visit_reports_error() {
    use std::cell::RefCell;
    use std::os::unix::fs::PermissionsExt;
    use std::rc::Rc;

    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("public/readme.txt"), "hello");
    let restricted_dir = root.join("restricted");
    fs::create_dir_all(&restricted_dir).expect("failed to create restricted dir");
    write_file(restricted_dir.join("secret.txt"), "secret");

    // If running as root (e.g. some CI containers), chmod 0o000 might still be readable.
    // Check if permission denial is actually enforceable.
    let mut perms = fs::metadata(&restricted_dir).unwrap().permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&restricted_dir, perms).expect("failed to restrict permissions");

    struct RestorePerms(PathBuf);
    impl Drop for RestorePerms {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        }
    }
    let _guard = RestorePerms(restricted_dir.clone());

    let is_restricted = fs::read_dir(&restricted_dir).is_err();
    if !is_restricted {
        // Running as root, permission denied cannot be tested via chmod
        return;
    }

    let request = WalkRequest::new(root)
        .directory_errors(DirectoryErrorMode::Visit);

    let recorded_errors: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(Vec::new()));
    let visited_entries: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let errors_clone = Rc::clone(&recorded_errors);
    let entries_clone = Rc::clone(&visited_entries);

    let status = request.for_each_entry_with_heartbeat(
        || Ok::<(), Infallible>(()),
        |entry| {
            entries_clone.borrow_mut().push(entry.relative_path.to_string());
            Ok(WalkDecision::Include)
        },
        |err: DirectoryError<'_>| {
            errors_clone.borrow_mut().push(err.path.to_path_buf());
            Ok(WalkDecision::SkipDescend)
        },
    );

    assert!(status.is_ok(), "traversal with error callback should succeed");
    assert_eq!(status.unwrap(), WalkStatus::Complete);

    let errors = recorded_errors.borrow();
    assert!(
        !errors.is_empty(),
        "DirectoryErrorMode::Visit must deliver directory open errors to the callback"
    );
    assert!(
        errors.iter().any(|p| p.ends_with("restricted")),
        "recorded error must be for restricted directory, got: {:?}",
        *errors
    );

    let visited = visited_entries.borrow();
    assert!(visited.contains(&"public/readme.txt".to_string()));
}

#[cfg(unix)]
#[test]
fn test_broken_symlink_directory_error_handling() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("valid.txt"), "valid");
    let broken_target = root.join("non_existent_target");
    let symlink_path = root.join("broken_link");
    std::os::unix::fs::symlink(&broken_target, &symlink_path).expect("failed to create symlink");

    // Under FollowLinks::Never, broken symlink is reported as a Symlink entry
    let request_never = WalkRequest::new(root).follow_links(FollowLinks::Never);
    let outcome_never = request_never.collect().expect("collecting with FollowLinks::Never should succeed");
    let symlink_entry = outcome_never
        .entries
        .iter()
        .find(|e| e.path == "broken_link");
    assert!(symlink_entry.is_some(), "symlink entry should be present");
    assert_eq!(symlink_entry.unwrap().file_type, FileType::Symlink);

    // Under FollowLinks::Always with SkipSkippable, broken symlink should not break the walk
    let request_always = WalkRequest::new(root)
        .follow_links(FollowLinks::Always)
        .directory_errors(DirectoryErrorMode::SkipSkippable);
    let outcome_always = request_always.collect().expect("collecting with FollowLinks::Always and SkipSkippable should succeed");
    assert!(outcome_always.entries.iter().any(|e| e.path == "valid.txt"));
}

// ============================================================================
// 3. Depth Bounds
// ============================================================================

#[test]
fn test_depth_single_level_shallow() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("root_file.txt"), "0");
    write_file(root.join("lvl1/file1.txt"), "1");
    write_file(root.join("lvl1/lvl2/file2.txt"), "2");
    write_file(root.join("lvl1/lvl2/lvl3/file3.txt"), "3");

    // depth(1, 1) means only immediate children of root (depth 1)
    let request = WalkRequest::new(root).depth(1, 1);
    let paths = collect_sorted_paths(&request);

    assert!(paths.contains(&"root_file.txt".to_string()));
    assert!(paths.contains(&"lvl1".to_string()));

    // Deeper paths must NOT be present
    assert!(!paths.contains(&"lvl1/file1.txt".to_string()));
    assert!(!paths.contains(&"lvl1/lvl2".to_string()));
    assert!(!paths.contains(&"lvl1/lvl2/file2.txt".to_string()));
    assert!(!paths.contains(&"lvl1/lvl2/lvl3/file3.txt".to_string()));
}

#[test]
fn test_depth_range_filtering() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("root_file.txt"), "0");
    write_file(root.join("d1/f1.txt"), "1");
    write_file(root.join("d1/d2/f2.txt"), "2");
    write_file(root.join("d1/d2/d3/f3.txt"), "3");

    // depth(2, 2) means only entries at depth 2
    let request = WalkRequest::new(root).depth(2, 2);
    let paths = collect_sorted_paths(&request);

    // Depth 1 entries should not be in the output
    assert!(!paths.contains(&"root_file.txt".to_string()));
    assert!(!paths.contains(&"d1".to_string()));

    // Depth 2 entries should be present
    assert!(paths.contains(&"d1/f1.txt".to_string()));
    assert!(paths.contains(&"d1/d2".to_string()));

    // Depth 3 entries should not be present
    assert!(!paths.contains(&"d1/d2/f2.txt".to_string()));
    assert!(!paths.contains(&"d1/d2/d3/f3.txt".to_string()));
}

#[test]
fn test_depth_root_emission() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("child.txt"), "child");

    // emit_root(true) with depth starting at 0 yields root
    let request_with_root = WalkRequest::new(root)
        .emit_root(true)
        .depth(0, 1);

    let outcome_root = request_with_root.collect().expect("collect should succeed");
    let entries = outcome_root.entries;

    assert!(
        entries.iter().any(|e| e.path.is_empty() && e.file_type == FileType::Dir),
        "root directory (empty relative path) should be emitted when emit_root=true and min_depth=0"
    );
    assert!(entries.iter().any(|e| e.path == "child.txt"));

    // emit_root(false) does not emit root even with min_depth=0
    let request_no_root = WalkRequest::new(root)
        .emit_root(false)
        .depth(0, 1);

    let outcome_no_root = request_no_root.collect().expect("collect should succeed");
    assert!(
        !outcome_no_root.entries.iter().any(|e| e.path.is_empty()),
        "root should not be emitted when emit_root=false"
    );
}

#[test]
fn test_depth_inverted_bounds_returns_empty() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("f1.txt"), "1");
    write_file(root.join("f2.txt"), "2");

    // min_depth > max_depth
    let request = WalkRequest::new(root).depth(5, 2);
    let outcome = request.collect().expect("inverted depth bounds should complete cleanly");

    assert!(
        outcome.entries.is_empty(),
        "inverted depth bounds (min > max) should result in 0 entries"
    );
}

#[test]
fn test_depth_max_zero_bounds() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("file.txt"), "data");

    // emit_root(false) with depth(0, 0) yields nothing
    let request_empty = WalkRequest::new(root)
        .emit_root(false)
        .depth(0, 0);
    let outcome_empty = request_empty.collect().expect("should succeed");
    assert!(outcome_empty.entries.is_empty());

    // emit_root(true) with depth(0, 0) yields ONLY the root directory
    let request_root_only = WalkRequest::new(root)
        .emit_root(true)
        .depth(0, 0);
    let outcome_root_only = request_root_only.collect().expect("should succeed");
    assert_eq!(outcome_root_only.entries.len(), 1);
    assert!(outcome_root_only.entries[0].path.is_empty());
    assert_eq!(outcome_root_only.entries[0].file_type, FileType::Dir);
}

// ============================================================================
// 4. Collectors and High-Level Filters
// ============================================================================

#[test]
fn test_collect_files_and_collect_dirs() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("src/lib.rs"), "pub fn f() {}");
    write_file(root.join("src/utils/mod.rs"), "");
    write_file(root.join("docs/guide.md"), "# Guide");

    let request = WalkRequest::new(root);

    let files = request.collect_files().expect("collect_files should succeed");
    assert!(files.iter().all(|e| e.is_file()));
    let file_paths: Vec<&str> = files.iter().map(|e| e.path.as_str()).collect();
    assert!(file_paths.contains(&"src/lib.rs"));
    assert!(file_paths.contains(&"src/utils/mod.rs"));
    assert!(file_paths.contains(&"docs/guide.md"));
    assert!(!file_paths.contains(&"src"));
    assert!(!file_paths.contains(&"docs"));

    let dirs = request.collect_dirs().expect("collect_dirs should succeed");
    assert!(dirs.iter().all(|e| e.is_dir()));
    let dir_paths: Vec<&str> = dirs.iter().map(|e| e.path.as_str()).collect();
    assert!(dir_paths.contains(&"src"));
    assert!(dir_paths.contains(&"src/utils"));
    assert!(dir_paths.contains(&"docs"));
    assert!(!dir_paths.contains(&"src/lib.rs"));
}

#[test]
fn test_walk_filter_glob() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    write_file(root.join("src/main.rs"), "");
    write_file(root.join("src/style.css"), "");
    write_file(root.join("index.html"), "");
    write_file(root.join("test/spec.rs"), "");

    let glob = CompiledWalkGlob::new(["**/*.rs", "*.rs"]).expect("glob must compile");
    let filter = WalkFilter::files_only().glob(glob);

    let request = WalkRequest::new(root).filter(filter);
    let paths = collect_sorted_paths(&request);

    assert_eq!(paths, vec!["src/main.rs", "test/spec.rs"]);
}

#[test]
fn test_walk_request_limit() {
    let temp = tempdir().expect("failed to create temp dir");
    let root = temp.path();

    for i in 0..20 {
        write_file(root.join(format!("file_{i:02}.txt")), format!("{i}"));
    }

    let request = WalkRequest::new(root)
        .filter(WalkFilter::files_only())
        .limit(5);

    let outcome = request.collect().expect("limit collect should succeed");
    assert_eq!(outcome.entries.len(), 5);
    assert_eq!(outcome.stats.limited_entries, 15);
}
