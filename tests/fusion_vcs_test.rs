//! Integration tests for the `fusion-vcs` engine.
//!
//! Verifies:
//! 1. In-process Git repository discovery (root, nested subdirectories, linked worktrees, non-repos).
//! 2. In-process Git diff queries (unstaged, staged, revisions, numstat, changed files, has_diff).
//! 3. Git status and diff queries executing successfully even when the `git` CLI binary cannot be spawned.
//! 4. Status summary, porcelain formatting, and dirty/clean transitions.
//! 5. Polymorphic `fusion_vcs::Repo` dispatch.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use fusion_vcs::git::GitRepo;
use fusion_vcs::types::{
    CommitOptions, DiffOptions, HeadState, StatusOptions, StatusSummary, UntrackedMode, VcsKind,
};
use fusion_vcs::{detect, Error, Repo};
use tempfile::TempDir;

/// Mutex to serialize tests that temporarily alter process environment variables like `PATH`.
static ENV_MUTEX: Mutex<()> = Mutex::new(());

/// RAII guard that safely clears or overrides the `PATH` environment variable and restores it on drop.
struct EnvPathGuard<'a> {
    _lock: std::sync::MutexGuard<'a, ()>,
    original_path: Option<std::ffi::OsString>,
}

impl EnvPathGuard<'_> {
    fn set_empty() -> Self {
        let lock = ENV_MUTEX.lock().unwrap();
        let original_path = std::env::var_os("PATH");
        unsafe {
            std::env::set_var("PATH", "");
        }
        Self {
            _lock: lock,
            original_path,
        }
    }
}

impl Drop for EnvPathGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            if let Some(path) = &self.original_path {
                std::env::set_var("PATH", path);
            } else {
                std::env::remove_var("PATH");
            }
        }
    }
}

/// Helper to run a git CLI command during fixture setup.
fn run_git(cwd: &Path, args: &[&str]) -> String {
    let _lock = ENV_MUTEX.lock().unwrap();
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("failed to execute git command for test fixture setup");
    assert!(
        output.status.success(),
        "git {:?} failed in {}: {}",
        args,
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("valid utf-8 output")
}

/// Initialize a basic git repository fixture with user config.
fn create_test_repo() -> TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    run_git(temp.path(), &["init", "-b", "main"]);
    run_git(temp.path(), &["config", "user.name", "Fusion Test"]);
    run_git(temp.path(), &["config", "user.email", "fusion@example.com"]);
    temp
}

/// Commit a file using git CLI during fixture setup.
fn fixture_commit(root: &Path, filename: &str, content: &str, message: &str) {
    fs::write(root.join(filename), content).expect("write file");
    run_git(root, &["add", filename]);
    run_git(root, &["commit", "-m", message]);
}

// ============================================================================
// 1. In-process Repository Discovery Tests
// ============================================================================

#[test]
fn test_in_process_discovery_repo_root() {
    let temp = create_test_repo();
    let root = temp.path();

    // Discover via GitRepo::discover
    let repo_opt = GitRepo::discover(root).expect("discover should succeed");
    assert!(repo_opt.is_some(), "should discover repository at root");
    let repo = repo_opt.unwrap();

    assert_eq!(repo.root(), root);
    assert_eq!(repo.primary_root(), root);
    assert!(!repo.is_linked_worktree());
    assert!(!repo.is_reftable());

    let info = repo.info();
    assert_eq!(info.repo_root, root);
    assert_eq!(info.git_dir, root.join(".git"));
    assert_eq!(info.common_dir, root.join(".git"));
    assert_eq!(info.head_path, root.join(".git/HEAD"));
    assert_eq!(repo.prefix_of(root), Some(String::new()));

    // Discover via GitRepo::require
    let required = GitRepo::require(root).expect("require should succeed");
    assert_eq!(required.root(), root);

    // Discover via fusion_vcs::detect
    let detected = detect(root).expect("detect should succeed");
    assert!(detected.is_some());
    let detected_repo = detected.unwrap();
    assert_eq!(detected_repo.kind(), VcsKind::Git);
    assert_eq!(detected_repo.root(), root);
    assert!(detected_repo.as_git().is_some());
    assert!(detected_repo.as_jj().is_none());
}

#[test]
fn test_in_process_discovery_nested_subdirectories() {
    let temp = create_test_repo();
    let root = temp.path();

    let deep_dir = root.join("src").join("engine").join("vcs");
    fs::create_dir_all(&deep_dir).expect("create deep dirs");

    // Discover from 3 levels deep
    let repo = GitRepo::discover(&deep_dir)
        .expect("discover deep dir")
        .expect("should find repo from nested directory");

    assert_eq!(repo.root(), root);
    assert_eq!(
        repo.prefix_of(&deep_dir),
        Some("src/engine/vcs/".to_string())
    );

    // Also verify detect() from deep dir
    let detected = detect(&deep_dir)
        .expect("detect deep dir")
        .expect("should detect repo from nested dir");
    assert_eq!(detected.root(), root);
    assert_eq!(
        detected.prefix_of(&deep_dir),
        Some("src/engine/vcs/".to_string())
    );
}

#[test]
fn test_in_process_discovery_not_a_repository() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path();

    // Outside any repo should return Ok(None)
    let discovered = GitRepo::discover(path).expect("discover on non-repo");
    assert!(discovered.is_none());

    let detected = detect(path).expect("detect on non-repo");
    assert!(detected.is_none());

    // require should return Error::NotARepository
    let err = GitRepo::require(path).unwrap_err();
    assert!(
        matches!(err, Error::NotARepository { .. }),
        "expected Error::NotARepository, got {err:?}"
    );

    // prefix_of on unrelated path should return None
    let repo_temp = create_test_repo();
    let repo = GitRepo::require(repo_temp.path()).unwrap();
    assert_eq!(repo.prefix_of(path), None);
}

#[test]
fn test_in_process_discovery_linked_worktree_pointer() {
    let temp = create_test_repo();
    let root = temp.path();
    fixture_commit(root, "README.md", "# Test Repo\n", "initial commit");

    // Create a linked worktree via git worktree add
    let wt_dir = tempfile::tempdir().expect("worktree tempdir");
    let wt_path = wt_dir.path().join("wt1");
    run_git(
        root,
        &[
            "worktree",
            "add",
            "-b",
            "feature-wt",
            wt_path.to_str().unwrap(),
        ],
    );

    // Discover the linked worktree
    let repo = GitRepo::discover(&wt_path)
        .expect("discover linked worktree")
        .expect("should discover linked worktree");

    let canonical_root = root.canonicalize().expect("canonical root");
    let canonical_wt = wt_path.canonicalize().expect("canonical wt");
    assert_eq!(repo.root().canonicalize().expect("repo root"), canonical_wt);
    assert_eq!(
        repo.primary_root().canonicalize().expect("primary root"),
        canonical_root
    );
    assert!(
        repo.is_linked_worktree(),
        "should identify as linked worktree"
    );

    let linked = repo.linked_worktree();
    assert!(linked.is_some());
    let linked_info = linked.unwrap();
    assert_eq!(
        linked_info.root.canonicalize().expect("linked root"),
        canonical_wt
    );
    assert_eq!(
        linked_info
            .primary_root
            .canonicalize()
            .expect("linked primary root"),
        canonical_root
    );
    // .git in worktree is a file pointer, not a directory
    assert!(wt_path.join(".git").is_file());

    // Verify detection works from a subdirectory inside the linked worktree
    let sub = wt_path.join("nested");
    fs::create_dir_all(&sub).expect("create nested in worktree");
    let sub_repo = GitRepo::discover(&sub).unwrap().unwrap();
    assert_eq!(
        sub_repo.root().canonicalize().expect("sub repo root"),
        canonical_wt
    );
    assert!(sub_repo.is_linked_worktree());
}

// ============================================================================
// 2. In-process Diff Query Tests (No Git CLI Needed)
// ============================================================================

#[test]
fn test_in_process_diff_query_unstaged_staged_and_revisions() {
    let temp = create_test_repo();
    let root = temp.path();
    fixture_commit(root, "file.txt", "line1\nline2\nline3\n", "base commit");

    let repo = GitRepo::require(root).expect("open repo");

    // Initially clean
    assert!(!repo.has_diff(&DiffOptions::default()).unwrap());
    assert_eq!(
        repo.changed_files(&DiffOptions::default()).unwrap(),
        Vec::<String>::new()
    );

    // 1. Modify file.txt (unstaged change)
    fs::write(
        root.join("file.txt"),
        "line1\nline2 modified\nline3\nline4\n",
    )
    .unwrap();

    assert!(repo.has_diff(&DiffOptions::default()).unwrap());
    let unstaged_diff = repo.diff_text(&DiffOptions::default()).unwrap();
    assert!(unstaged_diff.contains("--- a/file.txt"));
    assert!(unstaged_diff.contains("+++ b/file.txt"));
    assert!(unstaged_diff.contains("-line2"));
    assert!(unstaged_diff.contains("+line2 modified"));
    assert!(unstaged_diff.contains("+line4"));

    let changed = repo.changed_files(&DiffOptions::default()).unwrap();
    assert_eq!(changed, vec!["file.txt"]);

    let numstats = repo.numstat(&DiffOptions::default()).unwrap();
    assert_eq!(numstats.len(), 1);
    assert_eq!(numstats[0].path, "file.txt");
    assert_eq!(numstats[0].added, Some(2));
    assert_eq!(numstats[0].removed, Some(1));

    // 2. Stage the modification in-process using repo.stage_files
    repo.stage_files(&["file.txt".to_string()])
        .expect("in-process stage");

    // After staging: unstaged diff is empty, staged diff has the changes
    assert!(!repo.has_diff(&DiffOptions::default()).unwrap());
    let cached_opts = DiffOptions {
        cached: true,
        ..DiffOptions::default()
    };
    assert!(repo.has_diff(&cached_opts).unwrap());
    let staged_diff = repo.diff_text(&cached_opts).unwrap();
    assert!(staged_diff.contains("+line2 modified"));

    // 3. Add an unstaged new change on top of staged change
    fs::write(
        root.join("file.txt"),
        "line1\nline2 modified\nline3\nline4 extra\n",
    )
    .unwrap();
    let worktree_diff = repo.diff_text(&DiffOptions::default()).unwrap();
    assert!(worktree_diff.contains("-line4"));
    assert!(worktree_diff.contains("+line4 extra"));

    // 4. In-process commit creation
    let commit_opts = CommitOptions::default();
    let new_sha = repo
        .commit_create("updated file.txt", &commit_opts)
        .expect("in-process commit");
    assert!(!new_sha.is_empty());

    // 5. Compare revisions (diff_tree)
    let rev_diff = repo
        .diff_tree("HEAD^", "HEAD", false)
        .expect("diff_tree between revisions");
    assert!(rev_diff.contains("--- a/file.txt"));
    assert!(rev_diff.contains("+++ b/file.txt"));
    assert!(rev_diff.contains("+line2 modified"));
}

// ============================================================================
// 3. Git Status and Diff Query WITHOUT Spawning Git CLI
// ============================================================================

#[test]
fn test_git_status_and_diff_without_git_cli() {
    let temp = create_test_repo();
    let root = temp.path();
    fixture_commit(root, "existing.txt", "hello\n", "init");

    // Make various changes:
    // - Modify existing.txt (unstaged)
    fs::write(root.join("existing.txt"), "hello world\n").unwrap();
    // - Create a new untracked file
    fs::write(root.join("untracked.rs"), "fn main() {}\n").unwrap();
    // - Create a new file and stage it
    fs::write(root.join("staged_new.txt"), "staged content\n").unwrap();
    let repo = GitRepo::require(root).unwrap();
    repo.stage_files(&["staged_new.txt".to_string()]).unwrap();

    // Now isolate PATH: wipe it so `git` executable cannot be spawned!
    let _guard = EnvPathGuard::set_empty();

    // Verify PATH is indeed empty in this scope
    assert!(
        std::env::var_os("PATH").map_or(true, |p| p.is_empty()),
        "PATH should be empty to prevent git CLI spawning"
    );

    // 1. Status query via porcelain: must seamlessly fallback to in-process gitoxide!
    let status = repo
        .status_porcelain(&StatusOptions::default())
        .expect("status_porcelain must succeed without git CLI");

    assert!(
        status.contains(" M existing.txt"),
        "should detect unstaged modification in: {status}"
    );
    assert!(
        status.contains("A  staged_new.txt"),
        "should detect staged addition in: {status}"
    );
    assert!(
        status.contains("?? untracked.rs"),
        "should detect untracked file in: {status}"
    );

    // 2. Status summary query without git CLI
    let summary = repo
        .status_summary()
        .expect("status_summary must succeed without git CLI");
    assert_eq!(
        summary,
        StatusSummary {
            staged: 1,
            unstaged: 1,
            untracked: 1,
        }
    );

    // 3. Diff queries without git CLI
    // Unstaged diff
    let unstaged_diff = repo
        .diff_text(&DiffOptions::default())
        .expect("diff_text unstaged must succeed without git CLI");
    assert!(unstaged_diff.contains("--- a/existing.txt"));
    assert!(unstaged_diff.contains("+hello world"));

    // Staged diff
    let staged_diff = repo
        .diff_text(&DiffOptions {
            cached: true,
            ..DiffOptions::default()
        })
        .expect("diff_text staged must succeed without git CLI");
    assert!(staged_diff.contains("--- /dev/null"));
    assert!(staged_diff.contains("+++ b/staged_new.txt"));
    assert!(staged_diff.contains("+staged content"));

    // Numstat without git CLI
    let stats = repo
        .numstat(&DiffOptions::default())
        .expect("numstat must succeed without git CLI");
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].path, "existing.txt");
    assert_eq!(stats[0].added, Some(1));
    assert_eq!(stats[0].removed, Some(1));

    // Changed files without git CLI
    let files = repo
        .changed_files(&DiffOptions::default())
        .expect("changed_files must succeed without git CLI");
    assert_eq!(files, vec!["existing.txt"]);

    // Head state without git CLI
    let head = repo.head().expect("head must resolve without git CLI");
    match &head {
        HeadState::Ref { branch, .. } => assert_eq!(branch.as_deref(), Some("main")),
        other => panic!("expected HeadState::Ref, got {other:?}"),
    }
}

// ============================================================================
// 4. In-process Status Transitions (Clean, Dirty, Untracked Options)
// ============================================================================

#[test]
fn test_in_process_status_transitions() {
    let temp = create_test_repo();
    let root = temp.path();
    fixture_commit(root, "seed.txt", "data\n", "seed");

    let repo = GitRepo::require(root).unwrap();

    // Isolate PATH so every status check runs purely in-process
    let _guard = EnvPathGuard::set_empty();

    // Clean state
    assert!(!repo.is_dirty().unwrap());
    assert_eq!(
        repo.status_summary().unwrap(),
        StatusSummary {
            staged: 0,
            unstaged: 0,
            untracked: 0,
        }
    );

    // Create untracked file
    fs::write(root.join("new.txt"), "new file\n").unwrap();
    assert!(repo.is_dirty().unwrap());
    assert_eq!(repo.status_summary().unwrap().untracked, 1);

    // Untracked modes
    let status_normal = repo
        .status_porcelain(&StatusOptions {
            untracked: UntrackedMode::Normal,
            ..StatusOptions::default()
        })
        .unwrap();
    assert!(status_normal.contains("?? new.txt"));

    let status_no_untracked = repo
        .status_porcelain(&StatusOptions {
            untracked: UntrackedMode::No,
            ..StatusOptions::default()
        })
        .unwrap();
    assert!(!status_no_untracked.contains("?? new.txt"));

    // Stage file
    repo.stage_files(&["new.txt".to_string()]).unwrap();
    assert!(repo.is_dirty().unwrap());
    assert_eq!(
        repo.status_summary().unwrap(),
        StatusSummary {
            staged: 1,
            unstaged: 0,
            untracked: 0,
        }
    );

    // Commit staged file in-process
    repo.commit_create("add new.txt", &CommitOptions::default())
        .unwrap();
    assert!(!repo.is_dirty().unwrap());
    assert_eq!(
        repo.status_summary().unwrap(),
        StatusSummary {
            staged: 0,
            unstaged: 0,
            untracked: 0,
        }
    );
}

// ============================================================================
// 5. Polymorphic Repo Dispatch Tests
// ============================================================================

#[test]
fn test_repo_wrapper_enum_dispatch() {
    let temp = create_test_repo();
    let root = temp.path();
    fixture_commit(root, "tracked.txt", "line 1\n", "initial");

    fs::write(root.join("tracked.txt"), "line 1\nline 2\n").unwrap();

    let repo: Repo = detect(root).unwrap().expect("detected repo");
    assert_eq!(repo.kind(), VcsKind::Git);
    assert_eq!(repo.root(), root);

    // Label and HEAD id
    assert_eq!(repo.label().unwrap(), Some("main".to_string()));
    assert!(repo.head_id().unwrap().is_some());

    // Status summary through polymorphic Repo
    let summary = repo.status_summary().unwrap();
    assert_eq!(summary.unstaged, 1);

    // Diff text through polymorphic Repo
    let diff = repo.diff_text(&DiffOptions::default()).unwrap();
    assert!(diff.contains("+line 2"));

    // Uncommitted diff through polymorphic Repo
    let uncommitted = repo.uncommitted_diff(&[]).unwrap();
    assert!(uncommitted.contains("+line 2"));

    // Numstat through polymorphic Repo
    let stats = repo.numstat(&DiffOptions::default()).unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].path, "tracked.txt");
    assert_eq!(stats[0].added, Some(1));
    assert_eq!(stats[0].removed, Some(0));

    // Changed files through polymorphic Repo
    let changed = repo.changed_files(&DiffOptions::default()).unwrap();
    assert_eq!(changed, vec!["tracked.txt"]);
}
