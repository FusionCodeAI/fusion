//! Integration tests for `/rewind` and checkpoint restoration.
//!
//! Verifies:
//! - Creating checkpoints before file modifications
//! - Modifying files and reverting working tree changes via `undo_last`
//! - Detailed diff inspection (unified diff, hunks, stats) on reverted checkpoints
//! - Restoring newly created and deleted files
//! - Multi-file transactional checkpoint restoration and inspection
//! - End-to-end `/rewind` and `/undo` slash command integration with `AgentRunner` and `Session`

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Utc;
use fusion::agent::fork::{count_turns, extract_turns, rewind_session_in_place};
use fusion::agent::loop_runner::AgentRunner;
use fusion::agent::session::Session;
use fusion::agent::undo::{
    format_checkpoints_table, format_redo_report, format_undo_report, CheckpointManager,
    FileChangeType, UndoResult,
};
use fusion::config::Config;
use fusion::provider::LlmClient;
use fusion::tools::{default_registry, ToolContext};
use fusion::ui::slash::{execute_slash_command, SlashCommand};

// ===========================================================================
// Extension trait and helper function for `undo_last`
// ===========================================================================

/// Extension trait providing `undo_last` convenience method on `CheckpointManager`.
pub trait CheckpointManagerExt {
    /// Reverts the most recent checkpoint on the undo stack.
    fn undo_last(&mut self, cwd: &Path) -> anyhow::Result<UndoResult>;
}

impl CheckpointManagerExt for CheckpointManager {
    fn undo_last(&mut self, cwd: &Path) -> anyhow::Result<UndoResult> {
        self.undo(cwd)
    }
}

/// Standalone helper function to revert the most recent checkpoint with `undo_last`.
pub fn undo_last(manager: &mut CheckpointManager, cwd: &Path) -> anyhow::Result<UndoResult> {
    manager.undo(cwd)
}

// ===========================================================================
// Test Helpers
// ===========================================================================

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn make_temp_dir() -> PathBuf {
    let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "fusion_rewind_test_{}_{}_{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or(0),
        count
    ));
    fs::create_dir_all(&dir).expect("failed to create temp dir");
    dir
}

fn create_test_runner_and_session(cwd: PathBuf) -> (AgentRunner, Session) {
    let config = Config::default();
    let tools = default_registry();
    let tool_ctx = ToolContext {
        cwd,
        env: HashMap::new(),
    };
    let client = LlmClient::new();
    let model = config.default_model.clone();
    let runner = AgentRunner::new(client, config, tools, tool_ctx);
    let session = Session::new(&model);
    (runner, session)
}

// ===========================================================================
// Tests
// ===========================================================================

#[test]
fn test_slash_command_rewind_and_undo_parsing() {
    // Verify slash command parsing variants for /rewind and /undo
    assert_eq!(
        SlashCommand::parse("/rewind"),
        Some(SlashCommand::Rewind { turns: None })
    );
    assert_eq!(
        SlashCommand::parse("/rewind 1"),
        Some(SlashCommand::Rewind { turns: Some(1) })
    );
    assert_eq!(
        SlashCommand::parse("/rewind 3"),
        Some(SlashCommand::Rewind { turns: Some(3) })
    );
    assert_eq!(
        SlashCommand::parse("/undo"),
        Some(SlashCommand::Rewind { turns: None })
    );
    assert_eq!(
        SlashCommand::parse("/undo 2"),
        Some(SlashCommand::Rewind { turns: Some(2) })
    );
    assert_eq!(
        SlashCommand::parse("/rw"),
        Some(SlashCommand::Rewind { turns: None })
    );
    assert_eq!(
        SlashCommand::parse("/rw 4"),
        Some(SlashCommand::Rewind { turns: Some(4) })
    );
}

#[test]
fn test_checkpoint_create_modify_and_revert_with_undo_last() {
    let dir = make_temp_dir();
    let file_path = dir.join("calculator.rs");
    let original_content = "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
    fs::write(&file_path, original_content).expect("write original file");

    let mut mgr = CheckpointManager::new(dir.clone());

    // 1. Create a checkpoint before tool execution
    let args = serde_json::json!({
        "path": "calculator.rs",
        "content": "pub fn add(a: i32, b: i32) -> i32 {\n    // Mutated implementation\n    a.saturating_add(b)\n}\n"
    });
    let chk_id = mgr
        .capture_before_tool("write", &args, &dir)
        .expect("capture before tool")
        .expect("checkpoint id generated");

    // 2. Modify the file on disk
    let modified_content = "pub fn add(a: i32, b: i32) -> i32 {\n    // Mutated implementation\n    a.saturating_add(b)\n}\n";
    fs::write(&file_path, modified_content).expect("write modified file");
    mgr.capture_after_tool(&chk_id, &dir)
        .expect("capture after tool");

    // Verify modified state
    assert_eq!(
        fs::read_to_string(&file_path).expect("read file"),
        modified_content
    );
    assert_eq!(mgr.undo_count(), 1);
    assert!(mgr.can_undo());

    // 3. Revert with undo_last
    let undo_res = undo_last(&mut mgr, &dir).expect("undo_last should succeed");
    assert!(undo_res.success);
    assert_eq!(undo_res.restored_files.len(), 1);
    assert_eq!(undo_res.restored_files[0], file_path);

    // 4. Verify original content is completely restored
    let restored = fs::read_to_string(&file_path).expect("read restored file");
    assert_eq!(restored, original_content);
    assert_eq!(mgr.undo_count(), 0);
    assert_eq!(mgr.redo_count(), 1);
    assert!(mgr.can_redo());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_checkpoint_manager_ext_trait_undo_last() {
    let dir = make_temp_dir();
    let file_path = dir.join("greeter.rs");
    let original_content =
        "pub fn greet(name: &str) -> String {\n    format!(\"Hello, {}!\", name)\n}\n";
    fs::write(&file_path, original_content).expect("write original file");

    let mut mgr = CheckpointManager::new(dir.clone());

    // Create manual checkpoint
    let chk_id = mgr
        .create_manual_checkpoint("Pre-greeting refactor", &[file_path.clone()], &dir)
        .expect("create manual checkpoint");

    // Modify file
    let modified_content =
        "pub fn greet(name: &str) -> String {\n    format!(\"Greetings, {}!\", name)\n}\n";
    fs::write(&file_path, modified_content).expect("write modified file");

    // Revert using trait method syntax: mgr.undo_last(&dir)
    let undo_res = mgr.undo_last(&dir).expect("mgr.undo_last should succeed");
    assert!(undo_res.success);
    assert_eq!(
        fs::read_to_string(&file_path).expect("read restored file"),
        original_content
    );
    assert_eq!(mgr.redo_count(), 1);
    assert_eq!(
        mgr.peek_redo().map(|c| c.id.as_str()),
        Some(chk_id.as_str())
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_diff_inspection_on_reverted_checkpoint() {
    let dir = make_temp_dir();
    let file_path = dir.join("service.rs");
    let original = "struct Service {\n    port: u16,\n}\n\nimpl Service {\n    fn new() -> Self {\n        Self { port: 8080 }\n    }\n}\n";
    fs::write(&file_path, original).expect("write original");

    let mut mgr = CheckpointManager::new(dir.clone());

    // Capture checkpoint
    let args = serde_json::json!({
        "path": "service.rs",
        "content": "struct Service {\n    port: u16,\n    host: String,\n}\n\nimpl Service {\n    fn new() -> Self {\n        Self { port: 3000, host: \"127.0.0.1\".into() }\n    }\n}\n"
    });
    let chk_id = mgr
        .capture_before_tool("write", &args, &dir)
        .expect("capture before")
        .expect("chk id");

    let modified = "struct Service {\n    port: u16,\n    host: String,\n}\n\nimpl Service {\n    fn new() -> Self {\n        Self { port: 3000, host: \"127.0.0.1\".into() }\n    }\n}\n";
    fs::write(&file_path, modified).expect("write modified");
    mgr.capture_after_tool(&chk_id, &dir)
        .expect("capture after");

    // Revert with undo_last
    let undo_res = undo_last(&mut mgr, &dir).expect("undo_last");
    assert!(undo_res.success);
    assert_eq!(fs::read_to_string(&file_path).unwrap(), original);

    // Inspect the reverted checkpoint (checkpoint is preserved on redo stack!)
    let inspection = mgr
        .inspect_checkpoint(&chk_id, &dir)
        .expect("inspect reverted checkpoint should succeed");

    assert_eq!(inspection.checkpoint_id, chk_id);
    assert_eq!(inspection.tool_name, "write");
    assert_eq!(inspection.files.len(), 1);

    let diff = &inspection.files[0];
    assert_eq!(diff.change_type, FileChangeType::Modified);
    assert!(diff.unified_diff.is_some());

    let unified = diff.unified_diff.as_ref().unwrap();
    // Check diff content contains removed and added lines
    assert!(
        unified.contains("-        Self { port: 8080 }") || unified.contains("port: 8080"),
        "diff should contain old line: {}",
        unified
    );
    assert!(
        unified.contains("+    host: String,") || unified.contains("port: 3000"),
        "diff should contain new line: {}",
        unified
    );

    // Check stats
    assert!(diff.stats.insertions > 0);
    assert!(diff.stats.deletions > 0);
    assert_eq!(inspection.total_stats.files_changed, 1);
    assert!(inspection.total_stats.insertions > 0);
    assert!(inspection.total_stats.deletions > 0);

    // Inspect single file diff
    let single_diff = mgr
        .inspect_file_diff(&chk_id, &file_path, &dir)
        .expect("inspect single file diff");
    assert_eq!(single_diff.change_type, FileChangeType::Modified);
    assert!(!single_diff.hunks.is_empty());
    assert!(single_diff.colorized_diff.is_some());

    // Check formatted undo report output
    let report = format_undo_report(&undo_res);
    assert!(report.contains("service.rs") || report.contains("Restored"));

    // Check formatted checkpoints table
    let table = format_checkpoints_table(&mgr.list_checkpoints());
    assert!(table.contains("No checkpoints recorded") || table.contains("ID"));

    // Redo re-applies the modifications
    let redo_res = mgr.redo(&dir).expect("redo should succeed");
    assert!(redo_res.success);
    assert_eq!(fs::read_to_string(&file_path).unwrap(), modified);

    let redo_report = format_redo_report(&redo_res);
    assert!(redo_report.contains("service.rs") || redo_report.contains("Reapplied"));

    // Undo once more with undo_last
    let undo_again = undo_last(&mut mgr, &dir).expect("undo again");
    assert!(undo_again.success);
    assert_eq!(fs::read_to_string(&file_path).unwrap(), original);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_revert_created_and_deleted_files_with_diff_inspection() {
    let dir = make_temp_dir();

    // 1. Revert newly created file
    let created_path = dir.join("new_file.txt");
    assert!(!created_path.exists());

    let mut mgr = CheckpointManager::new(dir.clone());
    let args_create = serde_json::json!({
        "path": "new_file.txt",
        "content": "Newly generated content\nLine 2\n"
    });
    let chk_create = mgr
        .capture_before_tool("write", &args_create, &dir)
        .unwrap()
        .unwrap();

    fs::write(&created_path, "Newly generated content\nLine 2\n").unwrap();
    mgr.capture_after_tool(&chk_create, &dir).unwrap();
    assert!(created_path.exists());

    // Revert with undo_last -> newly created file is removed
    let undo_create = undo_last(&mut mgr, &dir).expect("undo creation");
    assert!(undo_create.success);
    assert!(!created_path.exists());
    assert_eq!(undo_create.deleted_files, vec![created_path.clone()]);

    // Inspect diff on reverted creation checkpoint
    let diff_create = mgr.inspect_checkpoint(&chk_create, &dir).unwrap();
    assert_eq!(diff_create.files.len(), 1);
    assert_eq!(diff_create.files[0].change_type, FileChangeType::Created);
    assert!(diff_create.files[0].stats.insertions >= 2);

    // 2. Revert deleted file
    let to_delete_path = dir.join("to_delete.txt");
    let del_content = "Important config to preserve\nKey: Value\n";
    fs::write(&to_delete_path, del_content).unwrap();

    let args_del = serde_json::json!({ "path": "to_delete.txt" });
    let chk_del = mgr
        .capture_before_tool("rm", &args_del, &dir)
        .unwrap()
        .unwrap();

    fs::remove_file(&to_delete_path).unwrap();
    mgr.capture_after_tool(&chk_del, &dir).unwrap();
    assert!(!to_delete_path.exists());

    // Revert with undo_last -> deleted file is resurrected
    let undo_del = undo_last(&mut mgr, &dir).expect("undo deletion");
    assert!(undo_del.success);
    assert!(to_delete_path.exists());
    assert_eq!(fs::read_to_string(&to_delete_path).unwrap(), del_content);

    // Inspect diff on reverted deletion checkpoint
    let diff_del = mgr.inspect_checkpoint(&chk_del, &dir).unwrap();
    assert_eq!(diff_del.files.len(), 1);
    assert_eq!(diff_del.files[0].change_type, FileChangeType::Deleted);
    assert!(diff_del.files[0].stats.deletions >= 2);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_multi_file_transaction_checkpoint_and_diff_inspection() {
    let dir = make_temp_dir();
    let file1 = dir.join("module_a.rs");
    let file2 = dir.join("module_b.rs");

    let orig_a = "pub fn a() -> usize { 1 }\n";
    let orig_b = "pub fn b() -> usize { 2 }\n";
    fs::write(&file1, orig_a).unwrap();
    fs::write(&file2, orig_b).unwrap();

    let mut mgr = CheckpointManager::new(dir.clone());

    // Begin multi-file transaction
    let _tx_id = mgr
        .begin_transaction("refactor", "Refactor module_a and module_b", None)
        .unwrap();

    mgr.record_pre_edit_for_transaction(&file1, &dir).unwrap();
    mgr.record_pre_edit_for_transaction(&file2, &dir).unwrap();

    let mod_a = "pub fn a() -> usize { 100 }\n";
    let mod_b = "pub fn b() -> usize { 200 }\n";
    fs::write(&file1, mod_a).unwrap();
    fs::write(&file2, mod_b).unwrap();

    mgr.record_post_edit_for_transaction(&file1, &dir).unwrap();
    mgr.record_post_edit_for_transaction(&file2, &dir).unwrap();

    let chk_id = mgr.commit_transaction(&dir).unwrap();
    assert_eq!(mgr.undo_count(), 1);

    // Revert transaction via undo_last
    let undo_res = undo_last(&mut mgr, &dir).expect("undo transaction");
    assert!(undo_res.success);
    assert_eq!(fs::read_to_string(&file1).unwrap(), orig_a);
    assert_eq!(fs::read_to_string(&file2).unwrap(), orig_b);

    // Inspect diff on reverted transaction
    let inspection = mgr.inspect_checkpoint(&chk_id, &dir).unwrap();
    assert_eq!(inspection.files.len(), 2);
    assert_eq!(inspection.total_stats.files_changed, 2);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_rewind_slash_command_execution_with_runner_and_session() {
    let dir = make_temp_dir();
    let (mut runner, mut session) = create_test_runner_and_session(dir.clone());

    // 1. Setup workspace file
    let file_path = dir.join("main.rs");
    let initial_code = "fn main() {\n    println!(\"Hello\");\n}\n";
    fs::write(&file_path, initial_code).unwrap();

    // 2. Simulate tool capturing checkpoint through runner's CheckpointManager
    let args = serde_json::json!({
        "path": "main.rs",
        "content": "fn main() {\n    println!(\"Agent broke this!\");\n}\n"
    });
    let checkpoints = runner.checkpoints();
    let chk_id = {
        let mut mgr = checkpoints.lock().unwrap();
        mgr.capture_before_tool("write", &args, &dir)
            .unwrap()
            .unwrap()
    };

    // Simulate tool modifying the file
    let broken_code = "fn main() {\n    println!(\"Agent broke this!\");\n}\n";
    fs::write(&file_path, broken_code).unwrap();
    {
        let mut mgr = checkpoints.lock().unwrap();
        mgr.capture_after_tool(&chk_id, &dir).unwrap();
    }
    // 3. Add conversation turns to the session
    session.add_user_message("Please update main.rs");
    session.add_assistant_message("I updated main.rs to break it.");
    assert_eq!(count_turns(&session), 1);
    assert_eq!(fs::read_to_string(&file_path).unwrap(), broken_code);

    // 4. Execute `/undo` slash command
    let cmd = SlashCommand::parse("/undo").unwrap();
    let result = execute_slash_command(&cmd, &mut runner, &mut session);
    assert!(matches!(result, fusion::ui::slash::CommandResult::Continue));

    // 5. Verify both file checkpoint restoration AND conversation turn rewind
    let restored_code = fs::read_to_string(&file_path).unwrap();
    assert_eq!(
        restored_code, initial_code,
        "File must be restored to initial code by /undo"
    );
    assert_eq!(
        count_turns(&session),
        0,
        "Session turns must be rewound by /undo"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_rewind_in_place_multi_turn_session() {
    let mut session = Session::new("test-model");

    // Turn 1
    session.add_user_message("Turn 1 request");
    session.add_assistant_message("Turn 1 answer");

    // Turn 2
    session.add_user_message("Turn 2 request");
    session.add_assistant_message("Turn 2 answer");

    // Turn 3
    session.add_user_message("Turn 3 request");
    session.add_assistant_message("Turn 3 answer");

    assert_eq!(count_turns(&session), 3);

    // Rewind 2 turns
    let reverted = rewind_session_in_place(&mut session, 2);
    assert_eq!(reverted, 2);
    assert_eq!(count_turns(&session), 1);

    let turns = extract_turns(session.messages());
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].user_message.as_deref(), Some("Turn 1 request"));
    assert_eq!(turns[0].assistant_message.as_deref(), Some("Turn 1 answer"));
}
