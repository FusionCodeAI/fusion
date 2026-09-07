//! Comprehensive test suite for `/skills` slash command handler (`src/ui/slash_skills.rs`).

use std::fs;
use tempfile::tempdir;

use fusion::agent;
use fusion::ui;

#[path = "../src/ui/slash_skills.rs"]
pub mod slash_skills;

use fusion::agent::skills::{Skill, SkillRegistry, SkillSource};
use slash_skills::{
    format_skills_palette, handle_skills_command, handle_skills_command_with_registry,
    render_skills_table, reset_skill_overrides,
};

// =========================================================================
// 1. List / Default subcommand tests
// =========================================================================

#[test]
fn test_handle_skills_command_empty_args_renders_table() {
    let output = handle_skills_command(&[], None);
    assert!(output.contains("Name"), "Table should contain 'Name' column header");
    assert!(output.contains("Source"), "Table should contain 'Source' column header");
    assert!(output.contains("Triggers"), "Table should contain 'Triggers' column header");
    assert!(
        output.contains("Enabled Status") || output.contains("Enabled"),
        "Table should contain 'Enabled' column header"
    );
}

#[test]
fn test_handle_skills_command_list_args_renders_table() {
    let output1 = handle_skills_command(&["list".to_string()], None);
    let output2 = handle_skills_command(&["".to_string()], None);
    let output3 = handle_skills_command(&["   ".to_string()], None);

    assert!(output1.contains("Name"));
    assert!(output2.contains("Name"));
    assert!(output3.contains("Name"));
}

#[test]
fn test_handle_skills_command_list_with_discovered_project_skills() {
    reset_skill_overrides();
    let temp = tempdir().expect("Failed to create tempdir");
    let skills_dir = temp.path().join(".fusion").join("skills");
    let cf_dir = skills_dir.join("cloudflare-workers");
    fs::create_dir_all(&cf_dir).expect("Failed to create skill dir");

    let skill_content = r#"---
name: cloudflare-workers
description: Cloudflare Workers best practices
triggers: ["wrangler.jsonc", "cloudflare", "worker"]
enabled: true
version: "1.0.0"
---

# Cloudflare Workers Guidelines
1. Always use standard Web standard APIs.
2. Keep cold start overhead minimal.
"#;
    fs::write(cf_dir.join("SKILL.md"), skill_content).expect("Failed to write SKILL.md");

    let output = handle_skills_command(&["list".to_string()], Some(temp.path()));

    assert!(output.contains("cloudflare-workers"), "Should list skill name");
    assert!(output.contains("Project"), "Source should indicate Project");
    assert!(output.contains("wrangler.jsonc"), "Triggers should include wrangler.jsonc");
    assert!(output.contains("Enabled"), "Status should be Enabled");
}

// =========================================================================
// 2. Show subcommand tests
// =========================================================================

#[test]
fn test_handle_skills_command_show_existing_skill() {
    reset_skill_overrides();
    let temp = tempdir().expect("Failed to create tempdir");
    let skills_dir = temp.path().join(".fusion").join("skills");
    let rust_dir = skills_dir.join("rust-async");
    fs::create_dir_all(&rust_dir).expect("Failed to create skill dir");

    let skill_content = r#"---
name: rust-async
description: Tokio async runtime guidelines
triggers: ["tokio", "async"]
enabled: true
---

# Async Guidelines
Use tokio::task::spawn_blocking for CPU-heavy work.
"#;
    fs::write(rust_dir.join("SKILL.md"), skill_content).expect("Failed to write SKILL.md");

    let output = handle_skills_command(
        &["show".to_string(), "rust-async".to_string()],
        Some(temp.path()),
    );

    assert!(output.contains("### Skill: rust-async"));
    assert!(output.contains("Tokio async runtime guidelines"));
    assert!(output.contains("Use tokio::task::spawn_blocking for CPU-heavy work."));
}

#[test]
fn test_handle_skills_command_show_case_insensitive() {
    reset_skill_overrides();
    let temp = tempdir().expect("Failed to create tempdir");
    let skills_dir = temp.path().join(".fusion").join("skills");
    let doc_dir = skills_dir.join("docker-deploy");
    fs::create_dir_all(&doc_dir).expect("Failed to create skill dir");

    let skill_content = r#"---
name: docker-deploy
description: Docker deployment patterns
---
Always use multi-stage builds.
"#;
    fs::write(doc_dir.join("SKILL.md"), skill_content).expect("Failed to write SKILL.md");

    let output = handle_skills_command(
        &["show".to_string(), "DOCKER-DEPLOY".to_string()],
        Some(temp.path()),
    );

    assert!(output.contains("### Skill: docker-deploy"));
    assert!(output.contains("Always use multi-stage builds."));
}

#[test]
fn test_handle_skills_command_show_combined_single_arg() {
    reset_skill_overrides();
    let temp = tempdir().expect("Failed to create tempdir");
    let skills_dir = temp.path().join(".fusion").join("skills");
    let doc_dir = skills_dir.join("docker-deploy");
    fs::create_dir_all(&doc_dir).expect("Failed to create skill dir");

    let skill_content = r#"---
name: docker-deploy
description: Docker deployment patterns
---
Always use multi-stage builds.
"#;
    fs::write(doc_dir.join("SKILL.md"), skill_content).expect("Failed to write SKILL.md");

    let output = handle_skills_command(
        &["show docker-deploy".to_string()],
        Some(temp.path()),
    );

    assert!(output.contains("### Skill: docker-deploy"));
}

#[test]
fn test_handle_skills_command_show_missing_name() {
    let output = handle_skills_command(&["show".to_string()], None);
    assert!(output.contains("Please specify a skill name"));
}

#[test]
fn test_handle_skills_command_show_not_found() {
    let output = handle_skills_command(
        &["show".to_string(), "nonexistent-skill-xyz".to_string()],
        None,
    );
    assert!(output.contains("Skill 'nonexistent-skill-xyz' not found."));
}

// =========================================================================
// 3. Enable / Disable toggle subcommand tests
// =========================================================================

#[test]
fn test_handle_skills_command_enable_disable_toggle() {
    reset_skill_overrides();
    let temp = tempdir().expect("Failed to create tempdir");
    let skills_dir = temp.path().join(".fusion").join("skills");
    let cf_dir = skills_dir.join("cloudflare-edge");
    fs::create_dir_all(&cf_dir).expect("Failed to create skill dir");

    let skill_content = r#"---
name: cloudflare-edge
description: Cloudflare Edge Workers
enabled: true
---
Edge workers instructions.
"#;
    fs::write(cf_dir.join("SKILL.md"), skill_content).expect("Failed to write SKILL.md");

    // 1. Initial list should show Enabled
    let list_init = handle_skills_command(&["list".to_string()], Some(temp.path()));
    assert!(list_init.contains("cloudflare-edge"));
    assert!(list_init.contains("Enabled"));

    // 2. Disable skill
    let disable_res = handle_skills_command(
        &["disable".to_string(), "cloudflare-edge".to_string()],
        Some(temp.path()),
    );
    assert!(disable_res.contains("Disabled skill 'cloudflare-edge'."));

    // 3. Subsequent list should reflect Disabled
    let list_after_disable = handle_skills_command(&["list".to_string()], Some(temp.path()));
    assert!(list_after_disable.contains("Disabled"));

    // 4. Re-enable skill
    let enable_res = handle_skills_command(
        &["enable".to_string(), "cloudflare-edge".to_string()],
        Some(temp.path()),
    );
    assert!(enable_res.contains("Enabled skill 'cloudflare-edge'."));

    // 5. Subsequent list should reflect Enabled
    let list_after_enable = handle_skills_command(&["list".to_string()], Some(temp.path()));
    assert!(list_after_enable.contains("Enabled"));
}

#[test]
fn test_handle_skills_command_enable_disable_missing_name() {
    let out_en = handle_skills_command(&["enable".to_string()], None);
    assert!(out_en.contains("Please specify a skill name"));

    let out_dis = handle_skills_command(&["disable".to_string()], None);
    assert!(out_dis.contains("Please specify a skill name"));
}

#[test]
fn test_handle_skills_command_enable_disable_not_found() {
    let out_en = handle_skills_command(
        &["enable".to_string(), "phantom-skill".to_string()],
        None,
    );
    assert!(out_en.contains("Skill 'phantom-skill' not found."));

    let out_dis = handle_skills_command(
        &["disable".to_string(), "phantom-skill".to_string()],
        None,
    );
    assert!(out_dis.contains("Skill 'phantom-skill' not found."));
}

// =========================================================================
// 4. Palette formatter & subcommand tests
// =========================================================================

#[test]
fn test_format_skills_palette_empty() {
    let output = format_skills_palette(&[]);
    assert!(output.contains("No skills available in palette"));
}

#[test]
fn test_format_skills_palette_with_skills() {
    let skill1 = Skill::new(
        "cloudflare-workers",
        "Cloudflare Workers guidelines",
        vec!["worker".into(), "wrangler.toml".into()],
        "Follow Cloudflare guidelines.",
        SkillSource::Builtin,
    );

    let skill2 = Skill::new(
        "rust-best-practices",
        "Idiomatic Rust coding style",
        vec!["rust".into(), "cargo".into()],
        "Write safe and clean Rust.",
        SkillSource::Project("/workspace/path".into()),
    )
    .with_enabled(false);

    let skills: Vec<&Skill> = vec![&skill1, &skill2];
    let output = format_skills_palette(&skills);

    assert!(output.contains("✦ Fusion Skills Palette"));
    assert!(output.contains("cloudflare-workers"));
    assert!(output.contains("● Active"));
    assert!(output.contains("[builtin]"));
    assert!(output.contains("Cloudflare Workers guidelines"));
    assert!(output.contains("Triggers:"));
    assert!(output.contains("worker, wrangler.toml"));

    assert!(output.contains("rust-best-practices"));
    assert!(output.contains("○ Disabled"));
    assert!(output.contains("[project]"));
}

#[test]
fn test_handle_skills_command_palette_subcommand() {
    reset_skill_overrides();
    let temp = tempdir().expect("Failed to create tempdir");
    let skills_dir = temp.path().join(".fusion").join("skills");
    let sk_dir = skills_dir.join("graphql-api");
    fs::create_dir_all(&sk_dir).expect("Failed to create skill dir");

    let skill_content = r#"---
name: graphql-api
description: GraphQL schema and resolver design
triggers: ["graphql", "schema.gql"]
---
Design robust GraphQL schemas.
"#;
    fs::write(sk_dir.join("SKILL.md"), skill_content).expect("Failed to write SKILL.md");

    let output = handle_skills_command(&["palette".to_string()], Some(temp.path()));
    assert!(output.contains("✦ Fusion Skills Palette"));
    assert!(output.contains("graphql-api"));
    assert!(output.contains("GraphQL schema and resolver design"));
}

// =========================================================================
// 5. Help, unknown commands, and direct registry execution tests
// =========================================================================

#[test]
fn test_handle_skills_command_help() {
    let output = handle_skills_command(&["help".to_string()], None);
    assert!(output.contains("Extensible Domain Skills Management:"));
    assert!(output.contains("/skills [list]"));
    assert!(output.contains("/skills show <name>"));
    assert!(output.contains("/skills enable <name>"));
    assert!(output.contains("/skills disable <name>"));
}

#[test]
fn test_handle_skills_command_unknown_subcommand() {
    let output = handle_skills_command(&["foobar_unknown".to_string()], None);
    assert!(output.contains("Unknown subcommand 'foobar_unknown'."));
    assert!(output.contains("Usage: /skills"));
}

#[test]
fn test_handle_skills_command_with_registry_direct() {
    let mut registry = SkillRegistry::new();
    let skill = Skill::new(
        "custom-eval",
        "Code evaluation engine",
        vec!["eval".into()],
        "Run evaluation safely.",
        SkillSource::Custom("in-memory".into()),
    );
    registry.register(skill);

    // List
    let list_res = handle_skills_command_with_registry(&["list".to_string()], &mut registry);
    assert!(list_res.contains("custom-eval"));
    assert!(list_res.contains("Custom"));
    assert!(list_res.contains("Enabled"));

    // Show
    let show_res = handle_skills_command_with_registry(
        &["show".to_string(), "custom-eval".to_string()],
        &mut registry,
    );
    assert!(show_res.contains("### Skill: custom-eval"));
    assert!(show_res.contains("Run evaluation safely."));

    // Disable
    let dis_res = handle_skills_command_with_registry(
        &["disable".to_string(), "custom-eval".to_string()],
        &mut registry,
    );
    assert!(dis_res.contains("Disabled skill 'custom-eval'."));
    assert!(!registry.get("custom-eval").unwrap().is_enabled());

    // Enable
    let en_res = handle_skills_command_with_registry(
        &["enable".to_string(), "custom-eval".to_string()],
        &mut registry,
    );
    assert!(en_res.contains("Enabled skill 'custom-eval'."));
    assert!(registry.get("custom-eval").unwrap().is_enabled());
}

#[test]
fn test_render_skills_table_direct() {
    let s1 = Skill::new(
        "alpha",
        "Alpha description",
        vec!["alpha".into()],
        "Alpha instructions",
        SkillSource::Builtin,
    );
    let s2 = Skill::new(
        "beta",
        "Beta description",
        vec![],
        "Beta instructions",
        SkillSource::Global("/home/user/.fusion/skills/beta".into()),
    )
    .with_enabled(false);

    let table = render_skills_table(&[&s1, &s2]);
    assert!(table.contains("alpha"));
    assert!(table.contains("beta"));
    assert!(table.contains("Builtin"));
    assert!(table.contains("Global"));
    assert!(table.contains("Enabled"));
    assert!(table.contains("Disabled"));
}
