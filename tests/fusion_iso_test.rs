//! Integration tests for `fusion-iso` and `WorkspaceIsolation`.
//!
//! Verifies:
//! 1. Workspace isolation lifecycle: creation, directory mirroring, explicit cleanup, and RAII drop cleanup.
//! 2. Multi-isolation concurrency: independent isolated instances from the same lower directory.
//! 3. Baseline capture and mutation diffing:
//!    - Clean baseline (no-op diff)
//!    - File addition (root & nested paths)
//!    - File modification (content changes, mtime checks)
//!    - File removal
//!    - Composite multi-file mutations
//!    - Binary file handling (NUL byte detection, diff: None)
//!    - Workspace change merging back to lower
//!    - Git-backed workspace diffing (git worktree / git diff path)
//! 4. Backend resolution, probes, and `BackendKind` contracts.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use fusion::agent::subagent::WorkspaceIsolation;
use fusion_iso::{
    backend, backend_kind, clone_candidates, default_backend, resolve, BackendKind, ChangeKind,
};
use tempfile::tempdir;

// ===========================================================================
// Helpers
// ===========================================================================

/// Populate a sample directory hierarchy with predictable text files.
fn populate_sample_workspace(root: &Path) {
    fs::write(root.join("README.md"), "# Test Project\nInitial readme text.\n").unwrap();
    fs::write(
        root.join("config.json"),
        "{\n  \"version\": 1,\n  \"name\": \"fusion-test\"\n}\n",
    )
    .unwrap();

    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("main.rs"),
        "fn main() {\n    println!(\"Hello, Fusion!\");\n}\n",
    )
    .unwrap();
    fs::write(
        src_dir.join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .unwrap();

    let sub_dir = src_dir.join("utils");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::write(sub_dir.join("helper.rs"), "// Helper functions\n").unwrap();
}

/// Helper to initialize a minimal git repository in `dir` with an initial commit.
fn init_git_repo(dir: &Path) {
    let run_git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("failed to execute git command");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run_git(&["init", "-b", "main"]);
    run_git(&["config", "user.email", "test@fusioncode.app"]);
    run_git(&["config", "user.name", "Fusion Tester"]);
    run_git(&["add", "."]);
    run_git(&["commit", "-m", "Initial baseline commit"]);
}

// ===========================================================================
// Part 1: Workspace Isolation Lifecycle (Creation & Cleanup)
// ===========================================================================

#[tokio::test]
async fn test_workspace_isolation_creation_and_explicit_cleanup() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    let id = format!("iso-lifecycle-{}", uuid::Uuid::new_v4());
    let mut isolation =
        WorkspaceIsolation::create(&lower, &id).expect("WorkspaceIsolation::create should succeed");

    let merged = isolation.merged.clone();

    // 1. Verify merged directory exists
    assert!(
        merged.exists(),
        "Merged directory should exist after creation: {}",
        merged.display()
    );
    assert!(merged.is_dir(), "Merged path should be a directory");

    // 2. Verify files from lower are mirrored in merged
    assert!(merged.join("README.md").exists());
    assert!(merged.join("config.json").exists());
    assert!(merged.join("src/main.rs").exists());
    assert!(merged.join("src/lib.rs").exists());
    assert!(merged.join("src/utils/helper.rs").exists());

    // Content verification
    let readme_content = fs::read_to_string(merged.join("README.md")).unwrap();
    assert!(readme_content.contains("Initial readme text."));

    let lib_content = fs::read_to_string(merged.join("src/lib.rs")).unwrap();
    assert!(lib_content.contains("pub fn add"));

    // 3. Explicit cleanup
    isolation.cleanup();

    // 4. Verify merged directory is removed
    assert!(
        !merged.exists(),
        "Merged directory should be removed after cleanup: {}",
        merged.display()
    );

    // 5. Verify cleanup is idempotent (calling again shouldn't fail or panic)
    isolation.cleanup();
    assert!(!merged.exists());

    // 6. Lower directory should remain completely intact
    assert!(lower.join("README.md").exists());
    assert!(lower.join("src/main.rs").exists());
}

#[tokio::test]
async fn test_workspace_isolation_raii_drop_cleanup() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    let merged_path: PathBuf;
    let id = format!("iso-raii-{}", uuid::Uuid::new_v4());

    // Enter scope where isolation lives
    {
        let isolation = WorkspaceIsolation::create(&lower, &id)
            .expect("WorkspaceIsolation::create should succeed");
        merged_path = isolation.merged.clone();

        assert!(
            merged_path.exists(),
            "Merged path must exist while WorkspaceIsolation is alive"
        );
        assert!(merged_path.join("README.md").exists());
    } // isolation dropped here

    // Verify RAII drop cleans up the merged directory
    assert!(
        !merged_path.exists(),
        "Merged path should have been cleaned up on drop: {}",
        merged_path.display()
    );
}

#[tokio::test]
async fn test_multiple_concurrent_isolations() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    let id1 = format!("iso-conc-1-{}", uuid::Uuid::new_v4());
    let id2 = format!("iso-conc-2-{}", uuid::Uuid::new_v4());

    let mut iso1 = WorkspaceIsolation::create(&lower, &id1).expect("create iso1");
    let mut iso2 = WorkspaceIsolation::create(&lower, &id2).expect("create iso2");

    let merged1 = iso1.merged.clone();
    let merged2 = iso2.merged.clone();

    assert_ne!(merged1, merged2, "Isolations must have distinct merged paths");
    assert!(merged1.exists());
    assert!(merged2.exists());

    // Mutate merged1 independently
    fs::write(merged1.join("isolated_1.txt"), "data 1").unwrap();
    assert!(merged1.join("isolated_1.txt").exists());
    assert!(!merged2.join("isolated_1.txt").exists());
    assert!(!lower.join("isolated_1.txt").exists());

    // Mutate merged2 independently
    fs::write(merged2.join("isolated_2.txt"), "data 2").unwrap();
    assert!(merged2.join("isolated_2.txt").exists());
    assert!(!merged1.join("isolated_2.txt").exists());
    assert!(!lower.join("isolated_2.txt").exists());

    // Clean up iso1, iso2 should remain intact
    iso1.cleanup();
    assert!(!merged1.exists());
    assert!(merged2.exists());
    assert!(merged2.join("isolated_2.txt").exists());

    // Clean up iso2
    iso2.cleanup();
    assert!(!merged2.exists());
}

// ===========================================================================
// Part 2: Baseline Capture and Mutation Diffing
// ===========================================================================

#[tokio::test]
async fn test_baseline_diff_clean_workspace() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    let id = format!("iso-clean-{}", uuid::Uuid::new_v4());
    let mut isolation = WorkspaceIsolation::create(&lower, &id).unwrap();

    let diff = isolation.diff().await.expect("diff should succeed");
    assert!(
        diff.is_empty(),
        "Clean workspace diff should be empty, but had {} files: {:?}",
        diff.files.len(),
        diff.files
    );
    assert!(diff.files.is_empty());
    assert!(diff.unified_text().is_empty());

    let captured = isolation.capture_changes().await.unwrap();
    assert!(captured.is_empty());

    isolation.cleanup();
}

#[tokio::test]
async fn test_baseline_diff_file_addition() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    let id = format!("iso-add-{}", uuid::Uuid::new_v4());
    let mut isolation = WorkspaceIsolation::create(&lower, &id).unwrap();

    // 1. Add file in root
    let new_root_file = isolation.merged.join("new_file.txt");
    fs::write(&new_root_file, "Line 1\nLine 2\n").unwrap();

    // 2. Add nested file in subdirectory
    let new_nested_file = isolation.merged.join("src/utils/new_helper.rs");
    fs::write(&new_nested_file, "pub fn helper() {}\n").unwrap();

    let diff = isolation.diff().await.expect("diff should succeed");
    assert_eq!(diff.files.len(), 2, "Expected 2 added files in diff");

    // Check root added file
    let root_change = diff
        .files
        .iter()
        .find(|f| f.path == Path::new("new_file.txt"))
        .expect("new_file.txt should be in diff");
    assert_eq!(root_change.op, ChangeKind::Added);
    assert!(root_change.diff.is_some());
    let root_diff_text = root_change.diff.as_ref().unwrap();
    assert!(root_diff_text.contains("+Line 1"));
    assert!(root_diff_text.contains("+Line 2"));

    // Check nested added file
    let nested_change = diff
        .files
        .iter()
        .find(|f| f.path == Path::new("src/utils/new_helper.rs"))
        .expect("src/utils/new_helper.rs should be in diff");
    assert_eq!(nested_change.op, ChangeKind::Added);
    assert!(nested_change.diff.is_some());
    let nested_diff_text = nested_change.diff.as_ref().unwrap();
    assert!(nested_diff_text.contains("+pub fn helper()"));

    // Lower directory remains unmutated
    assert!(!lower.join("new_file.txt").exists());
    assert!(!lower.join("src/utils/new_helper.rs").exists());

    isolation.cleanup();
}

#[tokio::test]
async fn test_baseline_diff_file_modification() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    let id = format!("iso-mod-{}", uuid::Uuid::new_v4());
    let mut isolation = WorkspaceIsolation::create(&lower, &id).unwrap();

    // Modify src/lib.rs in merged
    let target = isolation.merged.join("src/lib.rs");
    let modified_content =
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b + 1 // modified\n}\n\npub fn sub(a: i32, b: i32) -> i32 {\n    a - b\n}\n";
    fs::write(&target, modified_content).unwrap();

    let diff = isolation.diff().await.expect("diff should succeed");
    assert_eq!(diff.files.len(), 1, "Expected exactly 1 modified file");

    let change = &diff.files[0];
    assert_eq!(change.path, Path::new("src/lib.rs"));
    assert_eq!(change.op, ChangeKind::Modified);
    assert!(change.diff.is_some());

    let unified = change.diff.as_ref().unwrap();
    assert!(unified.contains("-    a + b"));
    assert!(unified.contains("+    a + b + 1 // modified"));
    assert!(unified.contains("+pub fn sub"));

    // Lower directory retains original content
    let lower_lib = fs::read_to_string(lower.join("src/lib.rs")).unwrap();
    assert!(lower_lib.contains("a + b\n}"));
    assert!(!lower_lib.contains("modified"));

    isolation.cleanup();
}

#[tokio::test]
async fn test_baseline_diff_file_removal() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    let id = format!("iso-del-{}", uuid::Uuid::new_v4());
    let mut isolation = WorkspaceIsolation::create(&lower, &id).unwrap();

    // Remove README.md in merged
    fs::remove_file(isolation.merged.join("README.md")).unwrap();

    let diff = isolation.diff().await.expect("diff should succeed");
    assert_eq!(diff.files.len(), 1, "Expected exactly 1 removed file");

    let change = &diff.files[0];
    assert_eq!(change.path, Path::new("README.md"));
    assert_eq!(change.op, ChangeKind::Removed);
    assert!(change.diff.is_some());
    let unified = change.diff.as_ref().unwrap();
    assert!(unified.contains("-# Test Project"));

    // Lower directory still has README.md
    assert!(lower.join("README.md").exists());

    isolation.cleanup();
}

#[tokio::test]
async fn test_baseline_diff_composite_mutations() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    let id = format!("iso-comp-{}", uuid::Uuid::new_v4());
    let mut isolation = WorkspaceIsolation::create(&lower, &id).unwrap();

    // 1. Add a file
    fs::write(
        isolation.merged.join("ADDED.md"),
        "# Newly Added Document\n",
    )
    .unwrap();

    // 2. Modify an existing file
    fs::write(
        isolation.merged.join("config.json"),
        "{\n  \"version\": 2,\n  \"name\": \"fusion-test-modified\"\n}\n",
    )
    .unwrap();

    // 3. Remove an existing file
    fs::remove_file(isolation.merged.join("src/utils/helper.rs")).unwrap();

    // 4. Leave README.md, src/main.rs, src/lib.rs untouched

    let diff = isolation.diff().await.expect("diff should succeed");
    assert_eq!(diff.files.len(), 3, "Expected 3 changes total");

    let added = diff
        .files
        .iter()
        .find(|f| f.path == Path::new("ADDED.md"))
        .expect("ADDED.md should be in diff");
    assert_eq!(added.op, ChangeKind::Added);

    let modified = diff
        .files
        .iter()
        .find(|f| f.path == Path::new("config.json"))
        .expect("config.json should be in diff");
    assert_eq!(modified.op, ChangeKind::Modified);

    let removed = diff
        .files
        .iter()
        .find(|f| f.path == Path::new("src/utils/helper.rs"))
        .expect("src/utils/helper.rs should be in diff");
    assert_eq!(removed.op, ChangeKind::Removed);

    // Verify full unified_text contains all 3 changes
    let full_unified = diff.unified_text();
    assert!(full_unified.contains("ADDED.md"));
    assert!(full_unified.contains("config.json"));
    assert!(full_unified.contains("helper.rs"));

    isolation.cleanup();
}

#[tokio::test]
async fn test_binary_file_handling() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();

    // Baseline with binary file
    let binary_data = vec![0u8, 159, 146, 150, 0, 255, 128, 0];
    fs::write(lower.join("binary.bin"), &binary_data).unwrap();
    fs::write(lower.join("text.txt"), "plain text\n").unwrap();

    let id = format!("iso-bin-{}", uuid::Uuid::new_v4());
    let mut isolation = WorkspaceIsolation::create(&lower, &id).unwrap();

    // Modify existing binary file
    let mut modified_binary = binary_data.clone();
    modified_binary.push(42);
    fs::write(isolation.merged.join("binary.bin"), &modified_binary).unwrap();

    // Add a new binary file
    let new_binary_data = vec![0u8, 1, 2, 3, 0];
    fs::write(isolation.merged.join("new_binary.dat"), &new_binary_data).unwrap();

    let diff = isolation.diff().await.expect("diff should succeed");
    assert_eq!(diff.files.len(), 2, "Expected 2 binary file changes");

    for change in &diff.files {
        // According to fusion-iso PAL contract, binary files emit `diff: None`
        assert!(
            change.diff.is_none(),
            "Binary file {:?} must have diff: None",
            change.path
        );
    }

    let modified_entry = diff
        .files
        .iter()
        .find(|f| f.path == Path::new("binary.bin"))
        .unwrap();
    assert_eq!(modified_entry.op, ChangeKind::Modified);

    let added_entry = diff
        .files
        .iter()
        .find(|f| f.path == Path::new("new_binary.dat"))
        .unwrap();
    assert_eq!(added_entry.op, ChangeKind::Added);

    // unified_text should skip binary entries without panic
    assert!(diff.unified_text().is_empty());

    isolation.cleanup();
}

#[tokio::test]
async fn test_workspace_isolation_merge_to_lower() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    let id = format!("iso-merge-{}", uuid::Uuid::new_v4());
    let mut isolation = WorkspaceIsolation::create(&lower, &id).unwrap();

    // Apply mutations in merged:
    // 1. Add file
    fs::write(
        isolation.merged.join("merged_feature.rs"),
        "pub fn feature() -> bool { true }\n",
    )
    .unwrap();
    // 2. Modify file
    fs::write(
        isolation.merged.join("README.md"),
        "# Updated Title\nUpdated description.\n",
    )
    .unwrap();
    // 3. Remove file
    fs::remove_file(isolation.merged.join("src/utils/helper.rs")).unwrap();

    let changes = isolation
        .capture_changes()
        .await
        .expect("capture_changes should succeed");
    assert_eq!(changes.files.len(), 3);

    // Merge changes back to lower
    isolation
        .merge(&changes)
        .await
        .expect("merge should succeed");

    // Verify lower now reflects the merged mutations!
    assert!(lower.join("merged_feature.rs").exists());
    let added_in_lower = fs::read_to_string(lower.join("merged_feature.rs")).unwrap();
    assert!(added_in_lower.contains("pub fn feature"));

    let updated_readme = fs::read_to_string(lower.join("README.md")).unwrap();
    assert!(updated_readme.contains("# Updated Title"));

    assert!(!lower.join("src/utils/helper.rs").exists());

    // And untouched files remain intact
    assert!(lower.join("src/main.rs").exists());
    assert!(lower.join("src/lib.rs").exists());

    isolation.cleanup();
}

#[tokio::test]
async fn test_git_workspace_diffing() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    // Initialize git repository in lower
    init_git_repo(&lower);

    let id = format!("iso-git-{}", uuid::Uuid::new_v4());
    let mut isolation = WorkspaceIsolation::create(&lower, &id).unwrap();

    // 1. Modify a tracked file
    let mod_file = isolation.merged.join("src/main.rs");
    fs::write(
        &mod_file,
        "fn main() {\n    println!(\"Hello from isolated git branch!\");\n}\n",
    )
    .unwrap();

    // 2. Add an untracked file
    let new_file = isolation.merged.join("untracked_note.txt");
    fs::write(&new_file, "Note created in isolation\n").unwrap();

    // 3. Delete a tracked file
    fs::remove_file(isolation.merged.join("config.json")).unwrap();

    let diff = isolation.diff().await.expect("git diff should succeed");

    let modified = diff
        .files
        .iter()
        .find(|f| f.path == Path::new("src/main.rs"))
        .expect("src/main.rs should be in git diff");
    assert_eq!(modified.op, ChangeKind::Modified);
    assert!(modified
        .diff
        .as_ref()
        .unwrap()
        .contains("Hello from isolated git branch!"));

    let added = diff
        .files
        .iter()
        .find(|f| f.path == Path::new("untracked_note.txt"))
        .expect("untracked_note.txt should be in git diff");
    assert_eq!(added.op, ChangeKind::Added);

    let removed = diff
        .files
        .iter()
        .find(|f| f.path == Path::new("config.json"))
        .expect("config.json should be in git diff");
    assert_eq!(removed.op, ChangeKind::Removed);

    // Filtered capture should exclude any .git internal entries
    let captured = isolation.capture_changes().await.unwrap();
    for file in &captured.files {
        assert!(
            !file.path.components().any(|c| c.as_os_str() == ".git"),
            "Captured changes must never include .git paths: {:?}",
            file.path
        );
    }

    isolation.cleanup();
}

// ===========================================================================
// Part 3: Backend Contracts, Resolution, and Probes
// ===========================================================================

#[test]
fn test_backend_kind_properties() {
    let all_kinds = [
        BackendKind::Apfs,
        BackendKind::Btrfs,
        BackendKind::Zfs,
        BackendKind::LinuxReflink,
        BackendKind::Overlayfs,
        BackendKind::WindowsBlockClone,
        BackendKind::Projfs,
        BackendKind::Rcopy,
    ];

    for kind in all_kinds {
        let name = kind.as_str();
        assert!(!name.is_empty());
        assert_eq!(
            BackendKind::from_str(name),
            Some(kind),
            "Round-trip parsing failed for {:?}",
            kind
        );
        let display = format!("{}", kind);
        assert!(!display.is_empty());
    }

    // Aliases
    assert_eq!(
        BackendKind::from_str("reflink"),
        Some(BackendKind::LinuxReflink)
    );
    assert_eq!(
        BackendKind::from_str("block-clone"),
        Some(BackendKind::WindowsBlockClone)
    );
    assert_eq!(BackendKind::from_str("invalid_backend_xyz"), None);

    // clones_tree property check
    assert!(BackendKind::Apfs.clones_tree());
    assert!(BackendKind::LinuxReflink.clones_tree());
    assert!(BackendKind::WindowsBlockClone.clones_tree());
    assert!(!BackendKind::Rcopy.clones_tree());
    assert!(!BackendKind::Overlayfs.clones_tree());
}

#[test]
fn test_fusion_iso_resolution() {
    let resolution = resolve(None);
    assert!(
        !resolution.candidates.is_empty(),
        "Candidates list should not be empty"
    );
    assert!(
        resolution.candidates.contains(&BackendKind::Rcopy),
        "Rcopy should always be present as universal fallback candidate"
    );

    // The chosen kind must be the first candidate
    assert_eq!(resolution.kind, resolution.candidates[0]);
    // Backend corresponding to chosen kind must report available
    let chosen_backend = backend(resolution.kind);
    let probe = chosen_backend.probe();
    assert!(
        probe.available,
        "Chosen backend {:?} probe must report available",
        resolution.kind
    );
}
#[test]
fn test_default_backend_contract() {
    let def = default_backend();
    let kind = backend_kind();
    assert_eq!(def.kind(), kind);

    let native = BackendKind::native();
    assert_eq!(kind, native);
}

#[test]
fn test_clone_candidates() {
    let candidates = clone_candidates(None);
    for kind in &candidates {
        assert!(
            kind.clones_tree(),
            "Clone candidate {:?} must have clones_tree() == true",
            kind
        );
        let b = backend(*kind);
        assert!(b.probe().available);
    }
}
#[tokio::test]
async fn test_rcopy_backend_direct() {
    let temp = tempdir().unwrap();
    let lower = temp.path().join("lower");
    let merged = temp.path().join("merged");
    fs::create_dir_all(&lower).unwrap();
    populate_sample_workspace(&lower);

    let rcopy = backend(BackendKind::Rcopy);
    assert_eq!(rcopy.kind(), BackendKind::Rcopy);
    assert!(rcopy.probe().available);

    // Start
    rcopy.start(&lower, &merged).expect("rcopy.start failed");
    assert!(merged.exists());
    assert!(merged.join("README.md").exists());
    assert!(merged.join("src/main.rs").exists());
    // Mutate
    fs::write(merged.join("direct_rcopy.txt"), "direct rcopy test").unwrap();

    // Diff
    let diff = rcopy.diff(&lower, &merged).await.expect("rcopy.diff failed");
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].path, Path::new("direct_rcopy.txt"));
    assert_eq!(diff.files[0].op, ChangeKind::Added);

    // Stop
    rcopy.stop(&merged).expect("rcopy.stop failed");
    assert!(!merged.exists());
}
