use fusion::tools::ask::{
    is_interactive_terminal, render_question_prompt, AskTool, Question, QuestionOption,
};
use fusion::tools::{default_registry, Tool, ToolContext};
use serde_json::json;
use std::io::Cursor;

#[tokio::test]
async fn test_ask_tool_name_and_metadata() {
    let tool = AskTool::new();
    assert_eq!(tool.name(), "ask");
    assert!(!tool.description().is_empty());
    assert!(
        tool.description().contains("decision") || tool.description().contains("questions"),
        "Description should describe asking questions or decision making"
    );

    let def = tool.definition();
    assert_eq!(def.name, "ask");
    assert_eq!(def.description, tool.description());
    assert_eq!(def.parameters, tool.parameters());
}

#[tokio::test]
async fn test_ask_tool_registered_in_default_registry() {
    let registry = default_registry();
    assert!(
        registry.contains("ask"),
        "default_registry must contain 'ask' tool"
    );
    let tool = registry.get("ask").expect("ask tool should be present");
    assert_eq!(tool.name(), "ask");
}

#[tokio::test]
async fn test_parameter_schema_structure() {
    let tool = AskTool::new();
    let schema = tool.parameters();

    assert_eq!(schema.get("type").and_then(|v| v.as_str()), Some("object"));

    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("parameters should have required fields");
    assert!(
        required.iter().any(|v| v.as_str() == Some("questions")),
        "questions must be in required list"
    );

    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("parameters should have properties");

    let questions_prop = props
        .get("questions")
        .and_then(|v| v.as_object())
        .expect("questions property must exist");
    assert_eq!(
        questions_prop.get("type").and_then(|v| v.as_str()),
        Some("array")
    );

    let items = questions_prop
        .get("items")
        .and_then(|v| v.as_object())
        .expect("questions must have items definition");
    assert_eq!(items.get("type").and_then(|v| v.as_str()), Some("object"));

    let item_props = items
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("question item must have properties");

    assert!(item_props.contains_key("id"));
    assert_eq!(
        item_props["id"].get("type").and_then(|v| v.as_str()),
        Some("string")
    );

    assert!(item_props.contains_key("question"));
    assert_eq!(
        item_props["question"].get("type").and_then(|v| v.as_str()),
        Some("string")
    );

    assert!(item_props.contains_key("options"));
    assert_eq!(
        item_props["options"].get("type").and_then(|v| v.as_str()),
        Some("array")
    );

    assert!(item_props.contains_key("multi"));
    assert_eq!(
        item_props["multi"].get("type").and_then(|v| v.as_str()),
        Some("boolean")
    );

    assert!(item_props.contains_key("recommended"));
    assert_eq!(
        item_props["recommended"]
            .get("type")
            .and_then(|v| v.as_str()),
        Some("integer")
    );

    let item_required = items
        .get("required")
        .and_then(|v| v.as_array())
        .expect("question items must have required fields");
    let required_names: Vec<&str> = item_required.iter().filter_map(|v| v.as_str()).collect();
    assert!(required_names.contains(&"id"));
    assert!(required_names.contains(&"question"));
    assert!(required_names.contains(&"options"));
}

#[tokio::test]
async fn test_parameter_schema_validation_errors() {
    let tool = AskTool::new().with_interactive(false);
    let ctx = ToolContext::default();

    // Missing 'questions' property entirely
    let bad_args1 = json!({});
    let res1 = tool.execute(bad_args1, &ctx).await;
    assert!(res1.is_err(), "Expected error when 'questions' is missing");

    // Empty 'questions' array
    let bad_args2 = json!({ "questions": [] });
    let res2 = tool.execute(bad_args2, &ctx).await;
    assert!(res2.is_err(), "Expected error when 'questions' is empty");

    // Question with empty 'options'
    let bad_args3 = json!({
        "questions": [
            {
                "id": "q1",
                "question": "What to do?",
                "options": []
            }
        ]
    });
    let res3 = tool.execute(bad_args3, &ctx).await;
    assert!(res3.is_err(), "Expected error when options list is empty");
}

#[tokio::test]
async fn test_non_interactive_execution_returns_default_recommended_option() {
    let tool = AskTool::new().with_interactive(false);
    let ctx = ToolContext::default();

    // 1. With explicit recommended option (index 1)
    let args_recommended = json!({
        "questions": [
            {
                "id": "framework",
                "question": "Which framework should we use?",
                "options": [
                    { "label": "Axum", "description": "Ergonomic and modular" },
                    { "label": "Actix-Web", "description": "High performance" },
                    { "label": "Rocket", "description": "Simple and type-safe" }
                ],
                "recommended": 1
            }
        ]
    });

    let res = tool.execute(args_recommended, &ctx).await.unwrap();
    assert_eq!(res, "Selected option: Actix-Web");

    // 2. Without recommended option specified (defaults to first option, index 0)
    let args_default = json!({
        "questions": [
            {
                "id": "engine",
                "question": "Which rendering engine?",
                "options": [
                    { "label": "Ratatui", "description": "Terminal UI library" },
                    { "label": "Crossterm", "description": "Low-level terminal backend" }
                ]
            }
        ]
    });

    let res_default = tool.execute(args_default, &ctx).await.unwrap();
    assert_eq!(res_default, "Selected option: Ratatui");
}

#[tokio::test]
async fn test_multi_question_formatting() {
    let tool = AskTool::new().with_interactive(false);
    let ctx = ToolContext::default();

    let multi_args = json!({
        "questions": [
            {
                "id": "database",
                "question": "Which primary database?",
                "options": [
                    { "label": "PostgreSQL", "description": "Relational" },
                    { "label": "SQLite", "description": "Embedded" }
                ],
                "recommended": 0
            },
            {
                "id": "cache",
                "question": "Which cache layer?",
                "options": [
                    { "label": "Redis", "description": "In-memory store" },
                    { "label": "Memcached", "description": "Simple key-value" }
                ],
                "recommended": 1
            },
            {
                "id": "deployment",
                "question": "Deployment target?",
                "options": [
                    { "label": "Docker Container", "description": "OCI image" },
                    { "label": "Bare Metal", "description": "Direct binary" }
                ]
            }
        ]
    });

    let res = tool.execute(multi_args, &ctx).await.unwrap();
    let lines: Vec<&str> = res.lines().collect();
    assert_eq!(
        lines.len(),
        3,
        "Output should contain 3 lines, one per question"
    );
    assert_eq!(lines[0], "database: Selected option: PostgreSQL");
    assert_eq!(lines[1], "cache: Selected option: Memcached");
    assert_eq!(lines[2], "deployment: Selected option: Docker Container");
}

#[tokio::test]
async fn test_interactive_choice_resolution_and_rendering() {
    let tool = AskTool::new();
    let q = Question {
        id: "strategy".into(),
        question: "Choose migration strategy".into(),
        options: vec![
            QuestionOption::new("Blue-Green").with_description("Zero-downtime deployment"),
            QuestionOption::new("Canary").with_description("Gradual traffic rollout"),
            QuestionOption::new("Recreate").with_description("Stop and recreate"),
        ],
        multi: false,
        recommended: Some(1),
    };

    // Test rendering contains (Recommended) chip for index 1
    let mut rendered_output = Vec::new();
    render_question_prompt(&mut rendered_output, &q).unwrap();
    let rendered_text = String::from_utf8(rendered_output).unwrap();
    assert!(rendered_text.contains("❓ Choose migration strategy"));
    assert!(rendered_text.contains("[1] Blue-Green"));
    assert!(!rendered_text.contains("[1] Blue-Green (Recommended)"));
    assert!(rendered_text.contains("[2] Canary (Recommended)"));
    assert!(rendered_text.contains("Gradual traffic rollout"));
    assert!(rendered_text.contains("[3] Recreate"));

    // Test interactive response by numeric index
    let mut reader1 = Cursor::new(b"1\n");
    let mut writer1 = Vec::new();
    let choice1 = tool
        .prompt_question(&q, &mut reader1, &mut writer1)
        .unwrap();
    assert_eq!(choice1, "Blue-Green");

    // Test interactive response by label string (case-insensitive)
    let mut reader2 = Cursor::new(b"recreate\n");
    let mut writer2 = Vec::new();
    let choice2 = tool
        .prompt_question(&q, &mut reader2, &mut writer2)
        .unwrap();
    assert_eq!(choice2, "Recreate");

    // Test interactive response with empty input (hits Enter) -> returns recommended
    let mut reader3 = Cursor::new(b"\n");
    let mut writer3 = Vec::new();
    let choice3 = tool
        .prompt_question(&q, &mut reader3, &mut writer3)
        .unwrap();
    assert_eq!(choice3, "Canary");
}

#[tokio::test]
async fn test_interactive_multi_choice_resolution() {
    let tool = AskTool::new();
    let q = Question {
        id: "addons".into(),
        question: "Select optional add-ons".into(),
        options: vec![
            QuestionOption::new("Telemetry"),
            QuestionOption::new("Tracing"),
            QuestionOption::new("HealthChecks"),
        ],
        multi: true,
        recommended: Some(0),
    };

    let mut reader = Cursor::new(b"1, 3\n");
    let mut writer = Vec::new();
    let choice = tool.prompt_question(&q, &mut reader, &mut writer).unwrap();
    assert_eq!(choice, "Telemetry, HealthChecks");
}

#[tokio::test]
async fn test_single_question_fallback_schema() {
    let tool = AskTool::new().with_interactive(false);
    let ctx = ToolContext::default();

    // Test flat single question argument format
    let args = json!({
        "id": "lang",
        "question": "Target programming language?",
        "options": [
            { "label": "Rust" },
            { "label": "TypeScript" }
        ],
        "recommended": 0
    });

    let res = tool.execute(args, &ctx).await.unwrap();
    assert_eq!(res, "Selected option: Rust");
}

#[test]
fn test_interactive_detection() {
    // In unit test runner, is_interactive_terminal should report false
    assert!(!is_interactive_terminal());

    let tool_auto = AskTool::new();
    assert!(!tool_auto.is_interactive());

    let tool_forced = AskTool::new().with_interactive(true);
    assert!(tool_forced.is_interactive());
}
