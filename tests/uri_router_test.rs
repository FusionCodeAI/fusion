//! Integration and unit tests for internal URI routing (`skill://` and `rule://`).

use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

use fusion::tools::file::{FileReadTool, ReadFileTool};
use fusion::tools::types::{Tool, ToolContext};
use fusion::tools::uri_router::resolve_internal_uri;
use serde_json::json;

#[test]
fn test_resolve_skill_directory_skill_md() {
    let dir = tempdir().expect("Failed to create tempdir");
    let skill_dir = dir
        .path()
        .join(".fusion")
        .join("skills")
        .join("brainstorming");
    fs::create_dir_all(&skill_dir).expect("Failed to create skill dir");
    let skill_file = skill_dir.join("SKILL.md");
    fs::write(
        &skill_file,
        "# Brainstorming Skill\nExplore user intent and requirements.",
    )
    .expect("Failed to write SKILL.md");

    // Resolve via skill://brainstorming
    let resolved = resolve_internal_uri("skill://brainstorming", Some(dir.path()));
    assert!(
        resolved.is_some(),
        "Expected skill://brainstorming to resolve"
    );
    assert_eq!(resolved.unwrap(), skill_file);
}

#[test]
fn test_resolve_skill_single_file_md() {
    let dir = tempdir().expect("Failed to create tempdir");
    let skills_dir = dir.path().join(".fusion").join("skills");
    fs::create_dir_all(&skills_dir).expect("Failed to create skills dir");
    let skill_file = skills_dir.join("code-review.md");
    fs::write(&skill_file, "# Code Review Skill\nReview pull requests.").expect("write");

    let resolved = resolve_internal_uri("skill://code-review", Some(dir.path()));
    assert!(
        resolved.is_some(),
        "Expected skill://code-review to resolve"
    );
    assert_eq!(resolved.unwrap(), skill_file);
}

#[test]
fn test_resolve_skill_with_subpath() {
    let dir = tempdir().expect("Failed to create tempdir");
    let skill_dir = dir.path().join(".fusion").join("skills").join("my-skill");
    fs::create_dir_all(&skill_dir).expect("Failed to create skill dir");
    let skill_file = skill_dir.join("SKILL.md");
    fs::write(&skill_file, "# My Skill").expect("write");
    let sub_file = skill_dir.join("checklist.md");
    fs::write(&sub_file, "- [ ] item 1\n- [ ] item 2").expect("write");

    let resolved = resolve_internal_uri("skill://my-skill/checklist.md", Some(dir.path()));
    assert!(
        resolved.is_some(),
        "Expected skill://my-skill/checklist.md to resolve"
    );
    assert_eq!(resolved.unwrap(), sub_file);
}

#[test]
fn test_resolve_rule_fusion_precedence() {
    let dir = tempdir().expect("Failed to create tempdir");
    let fusion_rules = dir.path().join(".fusion").join("rules");
    let cursor_rules = dir.path().join(".cursor").join("rules");
    let claude_rules = dir.path().join(".claude").join("rules");

    fs::create_dir_all(&fusion_rules).unwrap();
    fs::create_dir_all(&cursor_rules).unwrap();
    fs::create_dir_all(&claude_rules).unwrap();

    let fusion_file = fusion_rules.join("formatting.md");
    let cursor_file = cursor_rules.join("formatting.mdc");
    let claude_file = claude_rules.join("formatting.md");

    fs::write(&fusion_file, "# Fusion Formatting Rule").unwrap();
    fs::write(&cursor_file, "# Cursor Formatting Rule").unwrap();
    fs::write(&claude_file, "# Claude Formatting Rule").unwrap();

    // .fusion/rules should take precedence
    let resolved = resolve_internal_uri("rule://formatting", Some(dir.path()));
    assert_eq!(resolved, Some(fusion_file));
}

#[test]
fn test_resolve_rule_cursor_fallback() {
    let dir = tempdir().expect("Failed to create tempdir");
    let cursor_rules = dir.path().join(".cursor").join("rules");
    fs::create_dir_all(&cursor_rules).unwrap();
    let cursor_file = cursor_rules.join("linter.mdc");
    fs::write(&cursor_file, "# Linter Rule").unwrap();

    let resolved = resolve_internal_uri("rule://linter", Some(dir.path()));
    assert_eq!(resolved, Some(cursor_file));
}

#[test]
fn test_resolve_rule_claude_fallback() {
    let dir = tempdir().expect("Failed to create tempdir");
    let claude_rules = dir.path().join(".claude").join("rules");
    fs::create_dir_all(&claude_rules).unwrap();
    let claude_file = claude_rules.join("security.md");
    fs::write(&claude_file, "# Security Rule").unwrap();

    let resolved = resolve_internal_uri("rule://security", Some(dir.path()));
    assert_eq!(resolved, Some(claude_file));
}

#[test]
fn test_resolve_rule_with_extension_in_uri() {
    let dir = tempdir().expect("Failed to create tempdir");
    let fusion_rules = dir.path().join(".fusion").join("rules");
    fs::create_dir_all(&fusion_rules).unwrap();
    let fusion_file = fusion_rules.join("architecture.md");
    fs::write(&fusion_file, "# Architecture Guidelines").unwrap();

    let resolved = resolve_internal_uri("rule://architecture.md", Some(dir.path()));
    assert_eq!(resolved, Some(fusion_file));
}

#[test]
fn test_resolve_unknown_uris_return_none() {
    let dir = tempdir().expect("Failed to create tempdir");
    assert_eq!(
        resolve_internal_uri("skill://nonexistent", Some(dir.path())),
        None
    );
    assert_eq!(
        resolve_internal_uri("rule://nonexistent", Some(dir.path())),
        None
    );
    assert_eq!(
        resolve_internal_uri("http://example.com", Some(dir.path())),
        None
    );
    assert_eq!(
        resolve_internal_uri("file:///tmp/foo", Some(dir.path())),
        None
    );
    assert_eq!(resolve_internal_uri("src/main.rs", Some(dir.path())), None);
    assert_eq!(resolve_internal_uri("skill://", Some(dir.path())), None);
    assert_eq!(resolve_internal_uri("rule://", Some(dir.path())), None);
}

#[tokio::test]
async fn test_file_read_tool_reads_skill_uri() {
    let dir = tempdir().expect("Failed to create tempdir");
    let skill_dir = dir.path().join(".fusion").join("skills").join("planning");
    fs::create_dir_all(&skill_dir).unwrap();
    let skill_file = skill_dir.join("SKILL.md");
    fs::write(
        &skill_file,
        "Line 1: Planning\nLine 2: Design\nLine 3: Execute\n",
    )
    .unwrap();

    let tool = FileReadTool::new();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    };

    let output = tool
        .execute(
            json!({
                "path": "skill://planning"
            }),
            &ctx,
        )
        .await
        .expect("ReadFileTool should succeed with skill:// URI");

    assert!(output.contains("Planning"));
    assert!(output.contains("Design"));
    assert!(output.contains("Execute"));
}

#[tokio::test]
async fn test_file_read_tool_reads_skill_subpath_uri() {
    let dir = tempdir().expect("Failed to create tempdir");
    let skill_dir = dir.path().join(".fusion").join("skills").join("planning");
    fs::create_dir_all(&skill_dir).unwrap();
    let doc_file = skill_dir.join("notes.txt");
    fs::write(&doc_file, "Note 1\nNote 2\n").unwrap();

    let tool = ReadFileTool::new();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    };

    let output = tool
        .execute(
            json!({
                "path": "skill://planning/notes.txt"
            }),
            &ctx,
        )
        .await
        .expect("ReadFileTool should succeed with skill://planning/notes.txt");

    assert!(output.contains("Note 1"));
    assert!(output.contains("Note 2"));
}

#[tokio::test]
async fn test_file_read_tool_reads_rule_uri_with_selector() {
    let dir = tempdir().expect("Failed to create tempdir");
    let rules_dir = dir.path().join(".fusion").join("rules");
    fs::create_dir_all(&rules_dir).unwrap();
    let rule_file = rules_dir.join("safety.md");
    fs::write(
        &rule_file,
        "Line 1: No eval\nLine 2: No sudo\nLine 3: No leaks\n",
    )
    .unwrap();

    let tool = FileReadTool::new();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    };

    // Use inline selector :1-2 on rule:// URI
    let output = tool
        .execute(
            json!({
                "path": "rule://safety:1-2"
            }),
            &ctx,
        )
        .await
        .expect("ReadFileTool should succeed with rule://safety:1-2");

    assert!(output.contains("No eval"));
    assert!(output.contains("No sudo"));
    assert!(!output.contains("No leaks"));
}

#[tokio::test]
async fn test_file_read_tool_error_on_nonexistent_uri() {
    let dir = tempdir().expect("Failed to create tempdir");
    let tool = ReadFileTool::new();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        env: HashMap::new(),
    };

    let err = tool
        .execute(
            json!({
                "path": "rule://ghost"
            }),
            &ctx,
        )
        .await
        .unwrap_err();

    assert!(err.to_string().contains("File not found") || err.to_string().contains("ghost"));
}

#[test]
fn test_resolve_agent_and_artifact_uri() {
    let dir = tempdir().expect("Failed to create tempdir");
    let artifacts_dir = dir.path().join(".fusion").join("artifacts");
    fs::create_dir_all(&artifacts_dir).expect("create artifacts dir");
    let artifact_file = artifacts_dir.join("scout_arch.txt");
    fs::write(&artifact_file, "Architecture findings: clean modules.").expect("write artifact");

    // Resolve via agent://scout_arch
    let resolved = resolve_internal_uri("agent://scout_arch", Some(dir.path()));
    assert_eq!(resolved, Some(artifact_file.clone()));

    // Resolve via artifact://scout_arch
    let resolved_artifact = resolve_internal_uri("artifact://scout_arch", Some(dir.path()));
    assert_eq!(resolved_artifact, Some(artifact_file));
}
