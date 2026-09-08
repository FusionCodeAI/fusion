//! Comprehensive integration tests for `IsolatedWorkspace` and `WorktreeManager`.
//!
//! Tests:
//! 1. Workspace creation in `.fusion/worktrees/<task_id>` with base repo files mirrored.
//! 2. Baseline clean diff (empty string).
//! 3. File modification diff capture.
//! 4. Untracked / newly added file diff capture.
//! 5. File deletion diff capture.
//! 6. Multi-file composite mutation diffs in nested directories.
//! 7. Explicit cleanup: directory removal and branch deletion.
//! 8. RAII Drop cleanup: automatic removal on scope exit.
//! 9. Multi-task concurrency: multiple independent workspaces simultaneously.
//! 10. Task ID re-use: clean re-initialization.
//! 11. Error handling on non-git repository.
//! 12. Direct `WorktreeManager` operations (path derivation, listing, branch management).

#[path = "../src/agent/subagent_iso.rs"]
mod subagent_iso;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use fusion_iso::WorktreeManager;
use subagent_iso::IsolatedWorkspace;
use tempfile::{tempdir, TempDir};

// ============================================================================
// Helpers
// ============================================================================

/// Initializes a git repository in `dir` with an initial commit.
fn init_git_repo(dir: &Path) {
    let run = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {:?}: {e}", args));
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run(&["init", "-b", "main"]);
    run(&["config", "user.email", "test@fusion.ai"]);
    run(&["config", "user.name", "Fusion Tester"]);
    run(&["config", "commit.gpgsign", "false"]);

    // Populate initial files
    fs::write(
        dir.join("README.md"),
        "# Fusion Isolated Test\nInitial content.\n",
    )
    .unwrap();
    let src = dir.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    fs::write(
        src.join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .unwrap();

    run(&["add", "."]);
    run(&["commit", "-m", "Initial commit"]);
}

/// Helper to create a temp directory with an initialized git repository.
fn create_test_repo() -> (TempDir, PathBuf) {
    let temp = tempdir().expect("create tempdir");
    let path = fs::canonicalize(temp.path()).unwrap_or_else(|_| temp.path().to_path_buf());
    init_git_repo(&path);
    (temp, path)
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_create_for_task_creates_worktree_structure() {
    let (_temp, repo_path) = create_test_repo();
    let task_id = "task-alpha-001";

    let iso = IsolatedWorkspace::create_for_task(task_id, &repo_path)
        .expect("create_for_task should succeed");

    assert_eq!(iso.task_id(), task_id);
    assert_eq!(iso.task_id, task_id);
    assert_eq!(iso.base_path(), repo_path.as_path());
    assert_eq!(iso.branch(), "fusion-task-task-alpha-001");

    let expected_path = repo_path.join(".fusion").join("worktrees").join(task_id);
    assert_eq!(iso.path(), expected_path.as_path());
    assert!(iso.path().exists(), "Worktree directory should exist");
    assert!(iso.path().is_dir(), "Worktree path should be a directory");

    // Verify git files and tracked files exist in the worktree
    assert!(iso.path().join(".git").exists());
    assert!(iso.path().join("README.md").exists());
    assert!(iso.path().join("src").join("main.rs").exists());
    assert!(iso.path().join("src").join("lib.rs").exists());

    let content = fs::read_to_string(iso.path().join("README.md")).unwrap();
    assert!(content.contains("Initial content."));

    iso.cleanup().expect("cleanup should succeed");
}

#[test]
fn test_collect_diff_clean_workspace() {
    let (_temp, repo_path) = create_test_repo();
    let iso = IsolatedWorkspace::create_for_task("task-clean", &repo_path)
        .expect("create_for_task should succeed");

    let diff = iso.collect_diff().expect("collect_diff should succeed");
    assert!(diff.trim().is_empty(), "Expected clean diff, got: {diff}");

    iso.cleanup().expect("cleanup");
}

#[test]
fn test_collect_diff_modified_file() {
    let (_temp, repo_path) = create_test_repo();
    let iso = IsolatedWorkspace::create_for_task("task-mod", &repo_path)
        .expect("create_for_task should succeed");

    // Modify README.md inside isolated worktree
    let readme = iso.path().join("README.md");
    fs::write(&readme, "# Fusion Isolated Test\nModified by subagent!\n").unwrap();

    let diff = iso.collect_diff().expect("collect_diff should succeed");
    assert!(!diff.is_empty(), "Diff should not be empty");
    assert!(diff.contains("diff --git a/README.md b/README.md"));
    assert!(diff.contains("-Initial content."));
    assert!(diff.contains("+Modified by subagent!"));

    // Verify base repo was NOT modified
    let base_readme = fs::read_to_string(repo_path.join("README.md")).unwrap();
    assert!(base_readme.contains("Initial content."));
    assert!(!base_readme.contains("Modified by subagent!"));

    iso.cleanup().expect("cleanup");
}

#[test]
fn test_collect_diff_added_file() {
    let (_temp, repo_path) = create_test_repo();
    let iso = IsolatedWorkspace::create_for_task("task-add", &repo_path)
        .expect("create_for_task should succeed");

    // Add a new file inside isolated worktree
    let new_file = iso.path().join("NEW_FEATURE.md");
    fs::write(
        &new_file,
        "# New Feature Specification\nBrand new feature.\n",
    )
    .unwrap();

    let diff = iso.collect_diff().expect("collect_diff should succeed");
    assert!(!diff.is_empty(), "Diff should capture added file");
    assert!(diff.contains("diff --git a/NEW_FEATURE.md b/NEW_FEATURE.md"));
    assert!(diff.contains("new file mode"));
    assert!(diff.contains("+Brand new feature."));

    iso.cleanup().expect("cleanup");
}

#[test]
fn test_collect_diff_deleted_file() {
    let (_temp, repo_path) = create_test_repo();
    let iso = IsolatedWorkspace::create_for_task("task-del", &repo_path)
        .expect("create_for_task should succeed");

    // Delete a file inside isolated worktree
    let lib_rs = iso.path().join("src").join("lib.rs");
    fs::remove_file(&lib_rs).unwrap();

    let diff = iso.collect_diff().expect("collect_diff should succeed");
    assert!(!diff.is_empty(), "Diff should capture deleted file");
    assert!(diff.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    assert!(diff.contains("deleted file mode"));
    assert!(diff.contains("-pub fn add"));

    // Base repo should still have the file
    assert!(repo_path.join("src").join("lib.rs").exists());

    iso.cleanup().expect("cleanup");
}

#[test]
fn test_collect_diff_composite_mutations() {
    let (_temp, repo_path) = create_test_repo();
    let iso = IsolatedWorkspace::create_for_task("task-composite", &repo_path)
        .expect("create_for_task should succeed");

    // 1. Modify existing file
    fs::write(
        iso.path().join("src").join("main.rs"),
        "fn main() {\n    println!(\"updated main\");\n}\n",
    )
    .unwrap();

    // 2. Add new file in new nested directory
    let nested_dir = iso.path().join("src").join("components");
    fs::create_dir_all(&nested_dir).unwrap();
    fs::write(
        nested_dir.join("button.rs"),
        "pub struct Button { pub label: String }\n",
    )
    .unwrap();

    // 3. Delete existing file
    fs::remove_file(iso.path().join("src").join("lib.rs")).unwrap();

    let diff = iso.collect_diff().expect("collect_diff should succeed");
    assert!(diff.contains("src/main.rs"));
    assert!(diff.contains("+    println!(\"updated main\");"));
    assert!(diff.contains("src/components/button.rs"));
    assert!(diff.contains("+pub struct Button"));
    assert!(diff.contains("src/lib.rs"));
    assert!(diff.contains("deleted file mode"));

    iso.cleanup().expect("cleanup");
}

#[test]
fn test_explicit_cleanup_removes_worktree_and_branch() {
    let (_temp, repo_path) = create_test_repo();
    let task_id = "task-cleanup-test";

    let iso = IsolatedWorkspace::create_for_task(task_id, &repo_path)
        .expect("create_for_task should succeed");

    let worktree_path = iso.path().to_path_buf();
    let branch = iso.branch().to_string();

    assert!(worktree_path.exists());
    assert!(iso.manager().branch_exists(&branch));
    assert!(!iso.is_cleaned_up());

    // Perform explicit cleanup
    iso.cleanup().expect("cleanup should succeed");

    // Verify directory is deleted
    assert!(
        !worktree_path.exists(),
        "Worktree path should be removed after cleanup: {}",
        worktree_path.display()
    );

    // Verify git branch is deleted
    let manager = WorktreeManager::new(&repo_path);
    assert!(
        !manager.branch_exists(&branch),
        "Branch should be deleted after cleanup: {branch}"
    );
}

#[test]
fn test_raii_drop_cleanup() {
    let (_temp, repo_path) = create_test_repo();
    let task_id = "task-raii-drop";
    let expected_path;
    let expected_branch;

    {
        let iso = IsolatedWorkspace::create_for_task(task_id, &repo_path)
            .expect("create_for_task should succeed");
        expected_path = iso.path().to_path_buf();
        expected_branch = iso.branch().to_string();
        assert!(expected_path.exists());
        assert!(iso.manager().branch_exists(&expected_branch));
        // End of scope: iso is dropped here
    }

    // Verify automatic cleanup occurred upon drop
    assert!(
        !expected_path.exists(),
        "Worktree directory should be removed on drop: {}",
        expected_path.display()
    );

    let manager = WorktreeManager::new(&repo_path);
    assert!(
        !manager.branch_exists(&expected_branch),
        "Branch should be deleted on drop: {expected_branch}"
    );
}

#[test]
fn test_concurrent_isolated_workspaces() {
    let (_temp, repo_path) = create_test_repo();

    let iso1 =
        IsolatedWorkspace::create_for_task("task-worker-1", &repo_path).expect("worker 1 create");
    let iso2 =
        IsolatedWorkspace::create_for_task("task-worker-2", &repo_path).expect("worker 2 create");

    assert_ne!(iso1.path(), iso2.path());
    assert_ne!(iso1.branch(), iso2.branch());
    assert!(iso1.path().exists());
    assert!(iso2.path().exists());

    // Worker 1 modifies README
    fs::write(iso1.path().join("README.md"), "# Worker 1 Changes\n").unwrap();
    // Worker 2 creates worker2.txt
    fs::write(iso2.path().join("worker2.txt"), "worker 2 file\n").unwrap();

    let diff1 = iso1.collect_diff().expect("diff 1");
    let diff2 = iso2.collect_diff().expect("diff 2");

    assert!(diff1.contains("Worker 1 Changes"));
    assert!(!diff1.contains("worker 2 file"));

    assert!(diff2.contains("worker2.txt"));
    assert!(!diff2.contains("Worker 1 Changes"));

    iso1.cleanup().expect("cleanup 1");
    iso2.cleanup().expect("cleanup 2");
}

#[test]
fn test_recreate_for_same_task_id() {
    let (_temp, repo_path) = create_test_repo();
    let task_id = "task-reuse";

    // First run
    let iso1 = IsolatedWorkspace::create_for_task(task_id, &repo_path).expect("first run");
    fs::write(iso1.path().join("temp.txt"), "initial temp file\n").unwrap();
    iso1.cleanup().expect("cleanup 1");

    // Second run with same task_id should succeed cleanly
    let iso2 = IsolatedWorkspace::create_for_task(task_id, &repo_path).expect("second run");
    assert!(iso2.path().exists());
    assert!(
        !iso2.path().join("temp.txt").exists(),
        "Worktree should be clean"
    );

    iso2.cleanup().expect("cleanup 2");
}

#[test]
fn test_error_on_non_git_repository() {
    let temp = tempdir().expect("create tempdir");
    let non_git_path = temp.path();

    let res = IsolatedWorkspace::create_for_task("task-fail", non_git_path);
    assert!(
        res.is_err(),
        "create_for_task should fail on non-git directory"
    );
}

#[test]
fn test_worktree_manager_direct_methods() {
    let (_temp, repo_path) = create_test_repo();
    let manager = WorktreeManager::new(&repo_path);

    assert_eq!(manager.base_path(), repo_path.as_path());

    let task_id = "direct-test";
    let path = manager.worktree_path_for_task(task_id);
    let branch = manager.branch_for_task(task_id);

    assert_eq!(branch, "fusion-task-direct-test");
    assert_eq!(
        path,
        repo_path.join(".fusion").join("worktrees").join(task_id)
    );

    // Create worktree directly
    manager
        .create_worktree(&path, &branch)
        .expect("create_worktree");
    assert!(path.exists());
    assert!(manager.branch_exists(&branch));

    // Listing worktrees
    let list = manager.list_worktrees().expect("list_worktrees");
    assert!(list.iter().any(|p| p.ends_with(task_id)));

    // Cleanup directly
    manager.remove_worktree(&path).expect("remove_worktree");
    assert!(!path.exists());

    manager.delete_branch(&branch).expect("delete_branch");
    assert!(!manager.branch_exists(&branch));
}
