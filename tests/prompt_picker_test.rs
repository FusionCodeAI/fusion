//! Comprehensive Integration Tests for Prompt Picker Widget
//!
//! Verifies:
//! 1. Template variable extraction (`{{var}}`), whitespace trimming, deduplication, and rendering.
//! 2. Search filtering by name, category, description, and variables.
//! 3. Category tab cycling and category-specific filtering.
//! 4. Ratatui widget buffer rendering using `TestBackend`.
//! 5. Keyboard navigation, search input handling, selection, and cancellation.

#[path = "../src/ui/prompt_picker.rs"]
mod prompt_picker;

use crossterm::event::{KeyCode, KeyModifiers};
use prompt_picker::*;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use std::collections::HashMap;

/// Helper function to convert a Ratatui TestBackend buffer into a plain string.
fn buffer_to_string(buffer: &Buffer) -> String {
    let mut res = String::new();
    for y in buffer.area.top()..buffer.area.bottom() {
        for x in buffer.area.left()..buffer.area.right() {
            res.push_str(buffer.get(x, y).symbol());
        }
        res.push('\n');
    }
    res
}

// ============================================================================
// 1. Template Variable Extraction & Rendering Tests
// ============================================================================

#[test]
fn test_extract_variables_simple() {
    let template = "Hello {{name}}, welcome to {{project}}!";
    let vars = PromptTemplate::extract_variables(template);
    assert_eq!(vars, vec!["name", "project"]);
}

#[test]
fn test_extract_variables_whitespace_handling() {
    let template = "Run tests on {{  module_name   }} with flag {{   verbose  }}";
    let vars = PromptTemplate::extract_variables(template);
    assert_eq!(vars, vec!["module_name", "verbose"]);
}

#[test]
fn test_extract_variables_deduplication() {
    let template = "Read {{file}}, edit {{file}}, and verify {{file}} against {{spec}}.";
    let vars = PromptTemplate::extract_variables(template);
    assert_eq!(vars, vec!["file", "spec"]);
}

#[test]
fn test_extract_variables_edge_cases() {
    // Empty brackets
    assert!(PromptTemplate::extract_variables("No vars {{}} and {{   }} here").is_empty());
    // Unclosed bracket
    assert!(PromptTemplate::extract_variables("Broken {{unclosed bracket").is_empty());
    // Plain text
    assert!(PromptTemplate::extract_variables("Plain text without any variables").is_empty());
    // Adjacent variables
    let adjacent = PromptTemplate::extract_variables("{{a}}{{b}}{{c}}");
    assert_eq!(adjacent, vec!["a", "b", "c"]);
    // Empty template
    assert!(PromptTemplate::extract_variables("").is_empty());
}

#[test]
fn test_curated_prompts_variables_extracted() {
    let prompts = curated_prompts();
    assert_eq!(prompts.len(), 6);

    // 1. Rust: TDD Fix Loop
    let tdd = prompts.iter().find(|p| p.name == "Rust: TDD Fix Loop").unwrap();
    assert_eq!(tdd.category, "Rust");
    assert!(tdd.variables.contains(&"test_name".to_string()));
    assert!(tdd.variables.contains(&"test_file".to_string()));
    assert!(tdd.variables.contains(&"source_file".to_string()));

    // 2. Rust: Clippy & Performance
    let clippy = prompts
        .iter()
        .find(|p| p.name == "Rust: Clippy & Performance")
        .unwrap();
    assert_eq!(clippy.category, "Rust");
    assert!(clippy.variables.contains(&"target_file".to_string()));
    assert!(clippy.variables.contains(&"benchmark_metric".to_string()));

    // 3. TypeScript: React Component
    let react = prompts
        .iter()
        .find(|p| p.name == "TypeScript: React Component")
        .unwrap();
    assert_eq!(react.category, "TypeScript");
    assert!(react.variables.contains(&"component_name".to_string()));
    assert!(react.variables.contains(&"requirements".to_string()));
    assert!(react.variables.contains(&"props_interface".to_string()));

    // 4. Security: Vulnerability Audit
    let sec = prompts
        .iter()
        .find(|p| p.name == "Security: Vulnerability Audit")
        .unwrap();
    assert_eq!(sec.category, "Security");
    assert!(sec.variables.contains(&"codebase_path".to_string()));
    assert!(sec.variables.contains(&"threat_focus".to_string()));

    // 5. Refactor: Simplify & DRY
    let refactor = prompts
        .iter()
        .find(|p| p.name == "Refactor: Simplify & DRY")
        .unwrap();
    assert_eq!(refactor.category, "Refactor");
    assert!(refactor.variables.contains(&"target_module".to_string()));
    assert!(refactor.variables.contains(&"refactor_goal".to_string()));

    // 6. Doc: API Reference
    let doc = prompts
        .iter()
        .find(|p| p.name == "Doc: API Reference")
        .unwrap();
    assert_eq!(doc.category, "Documentation");
    assert!(doc.variables.contains(&"symbol_name".to_string()));
    assert!(doc.variables.contains(&"module_path".to_string()));
    assert!(doc.variables.contains(&"style_guide".to_string()));
}

#[test]
fn test_template_rendering_substitution() {
    let template = PromptTemplate::new(
        "Custom Test",
        "Test",
        "Test description",
        "Fix {{bug_name}} in {{path}} using {{strategy}}.",
    );

    let mut values = HashMap::new();
    values.insert("bug_name".to_string(), "NullPointerException".to_string());
    values.insert("path".to_string(), "src/main.rs".to_string());
    // "strategy" omitted to test keeping original placeholder

    let rendered = template.render(&values);
    assert_eq!(
        rendered,
        "Fix NullPointerException in src/main.rs using {{strategy}}."
    );
}

// ============================================================================
// 2. Search Filtering & Category Selection Tests
// ============================================================================

#[test]
fn test_search_filtering_by_name() {
    let state = PromptPickerState::default();
    assert_eq!(state.filtered_prompts().len(), 6);

    let mut filtered_state = state.clone();
    filtered_state.set_search_query("tdd");
    let results = filtered_state.filtered_prompts();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Rust: TDD Fix Loop");
}

#[test]
fn test_search_filtering_by_category() {
    let mut state = PromptPickerState::default();
    state.set_search_query("typescript");
    let results = state.filtered_prompts();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "TypeScript: React Component");
}

#[test]
fn test_search_filtering_by_description() {
    let mut state = PromptPickerState::default();
    state.set_search_query("owasp");
    let results = state.filtered_prompts();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Security: Vulnerability Audit");
}

#[test]
fn test_search_filtering_by_variable() {
    let mut state = PromptPickerState::default();
    state.set_search_query("benchmark_metric");
    let results = state.filtered_prompts();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Rust: Clippy & Performance");
}

#[test]
fn test_search_filtering_case_insensitive() {
    let mut state = PromptPickerState::default();
    state.set_search_query("REACT");
    let results = state.filtered_prompts();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "TypeScript: React Component");
}

#[test]
fn test_search_filtering_no_matches() {
    let mut state = PromptPickerState::default();
    state.set_search_query("nonexistent_keyword_xyz");
    let results = state.filtered_prompts();
    assert!(results.is_empty());
}

#[test]
fn test_search_clear_restores_all() {
    let mut state = PromptPickerState::default();
    state.set_search_query("tdd");
    assert_eq!(state.filtered_prompts().len(), 1);

    state.clear_search();
    assert_eq!(state.filtered_prompts().len(), 6);
}

#[test]
fn test_category_tab_filtering() {
    let mut state = PromptPickerState::default();
    assert_eq!(state.current_category(), "All");
    assert_eq!(state.filtered_prompts().len(), 6);

    state.set_category("Rust");
    assert_eq!(state.current_category(), "Rust");
    let rust_prompts = state.filtered_prompts();
    assert_eq!(rust_prompts.len(), 2);
    assert!(rust_prompts.iter().all(|p| p.category == "Rust"));

    state.set_category("Security");
    assert_eq!(state.current_category(), "Security");
    let sec_prompts = state.filtered_prompts();
    assert_eq!(sec_prompts.len(), 1);
    assert_eq!(sec_prompts[0].name, "Security: Vulnerability Audit");

    state.set_category("All");
    assert_eq!(state.filtered_prompts().len(), 6);
}

#[test]
fn test_category_tab_cycling() {
    let mut state = PromptPickerState::default();
    assert_eq!(state.current_category(), "All");

    state.next_category();
    assert_eq!(state.current_category(), "Rust");

    state.next_category();
    assert_eq!(state.current_category(), "TypeScript");

    state.prev_category();
    assert_eq!(state.current_category(), "Rust");

    state.prev_category();
    assert_eq!(state.current_category(), "All");
}

// ============================================================================
// 3. Ratatui Widget Rendering Tests
// ============================================================================

#[test]
fn test_widget_rendering_standard_layout() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    let state = PromptPickerState::default();
    let widget = PromptPickerWidget::new(&state);

    terminal
        .draw(|f| {
            f.render_widget(widget, f.area());
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered = buffer_to_string(buffer);

    // Verify category tabs are rendered
    assert!(rendered.contains("[All]"), "Expected [All] active tab");
    assert!(rendered.contains("Rust"), "Expected Rust tab");
    assert!(rendered.contains("TypeScript"), "Expected TypeScript tab");

    // Verify search bar presence
    assert!(rendered.contains("Search (/ to type)"));

    // Verify left column template list items
    assert!(rendered.contains("Rust: TDD Fix Loop"));
    assert!(rendered.contains("Rust: Clippy & Performance"));
    assert!(rendered.contains("TypeScript: React Component"));

    // Verify selected cursor
    assert!(rendered.contains("❯ Rust: TDD Fix Loop"));

    // Verify right column preview content
    assert!(rendered.contains("Preview: Rust: TDD Fix Loop"));
    assert!(rendered.contains("Category:"));
    assert!(rendered.contains("Description:"));
    assert!(rendered.contains("Variables:"));
    assert!(rendered.contains("{{test_name}}"));

    // Verify footer keyboard hints
    assert!(rendered.contains("↑↓"));
    assert!(rendered.contains("navigate"));
    assert!(rendered.contains("Tab"));
    assert!(rendered.contains("switch tab"));
    assert!(rendered.contains("Enter"));
    assert!(rendered.contains("select"));
    assert!(rendered.contains("Esc"));
    assert!(rendered.contains("cancel"));
}

#[test]
fn test_widget_rendering_filtered_view() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut state = PromptPickerState::default();
    state.set_search_query("Security");
    let widget = PromptPickerWidget::new(&state);

    terminal
        .draw(|f| {
            f.render_widget(widget, f.area());
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered = buffer_to_string(buffer);

    // Active search query rendered in search block
    assert!(rendered.contains("Security"));

    // Filtered template shown
    assert!(rendered.contains("Security: Vulnerability Audit"));

    // Non-matching template must NOT appear in template list
    assert!(!rendered.contains("Rust: TDD Fix Loop"));

    // Preview shows security prompt
    assert!(rendered.contains("Preview: Security: Vulnerability Audit"));
    assert!(rendered.contains("{{codebase_path}}"));
}

#[test]
fn test_widget_rendering_empty_filter_message() {
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut state = PromptPickerState::default();
    state.set_search_query("nonexistent_pattern_123");
    let widget = PromptPickerWidget::new(&state);

    terminal
        .draw(|f| {
            f.render_widget(widget, f.area());
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered = buffer_to_string(buffer);

    assert!(rendered.contains("No templates match query"));
}

#[test]
fn test_widget_rendering_compact_terminal() {
    let backend = TestBackend::new(60, 15);
    let mut terminal = Terminal::new(backend).unwrap();

    let state = PromptPickerState::default();
    let widget = PromptPickerWidget::new(&state);

    // Should render gracefully without panicking
    terminal
        .draw(|f| {
            f.render_widget(widget, f.area());
        })
        .unwrap();

    let buffer = terminal.backend().buffer();
    let rendered = buffer_to_string(buffer);

    assert!(rendered.contains("Rust: TDD Fix Loop"));
    assert!(rendered.contains("navigate"));
}

// ============================================================================
// 4. Keyboard Navigation & Interaction Tests
// ============================================================================

#[test]
fn test_keyboard_navigation_selection() {
    let mut state = PromptPickerState::default();
    assert_eq!(
        state.selected_prompt().unwrap().name,
        "Rust: TDD Fix Loop"
    );

    // Down arrow moves to second item
    state.handle_key(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        state.selected_prompt().unwrap().name,
        "Rust: Clippy & Performance"
    );

    // Up arrow moves back to first item
    state.handle_key(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        state.selected_prompt().unwrap().name,
        "Rust: TDD Fix Loop"
    );

    // Up arrow wraps around to last item
    state.handle_key(KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(state.selected_prompt().unwrap().name, "Doc: API Reference");

    // Down arrow wraps around to first item
    state.handle_key(KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        state.selected_prompt().unwrap().name,
        "Rust: TDD Fix Loop"
    );
}

#[test]
fn test_keyboard_tab_category_switching() {
    let mut state = PromptPickerState::default();
    assert_eq!(state.current_category(), "All");

    // Tab cycles forward
    state.handle_key(KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(state.current_category(), "Rust");

    // Shift+Tab cycles backward
    state.handle_key(KeyCode::BackTab, KeyModifiers::NONE);
    assert_eq!(state.current_category(), "All");
}

#[test]
fn test_keyboard_search_typing_and_backspace() {
    let mut state = PromptPickerState::default();

    // Type characters
    state.handle_key(KeyCode::Char('t'), KeyModifiers::NONE);
    state.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
    state.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
    assert_eq!(state.search_query, "tdd");
    assert_eq!(state.filtered_prompts().len(), 1);

    // Backspace removes last character
    state.handle_key(KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(state.search_query, "td");

    // Esc clears search query
    let res = state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(res, None);
    assert_eq!(state.search_query, "");
    assert_eq!(state.filtered_prompts().len(), 6);

    // Esc with empty search cancels
    let cancel_res = state.handle_key(KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(cancel_res, Some(PromptPickerResult::Cancelled));
}

#[test]
fn test_keyboard_enter_selects_current() {
    let mut state = PromptPickerState::default();
    let res = state.handle_key(KeyCode::Enter, KeyModifiers::NONE);

    match res {
        Some(PromptPickerResult::Selected(prompt)) => {
            assert_eq!(prompt.name, "Rust: TDD Fix Loop");
            assert_eq!(prompt.category, "Rust");
        }
        other => panic!("Expected Selected result, got {other:?}"),
    }
}
