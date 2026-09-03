//! Native implementation of SKILL.state: Scalable Long-Horizon Agent Skills
//! (arXiv:2608.26263, EMNLP 2026 - Badhe, Tiwari, Chung).
//!
//! Replaces unbounded append-only O(T) conversation transcripts with an explicit,
//! mutable structured execution state Σ_t.
//!
//! At each step t, the model receives only the compact triple A_t = (P, Σ_t, O_t):
//! - P: Immutable procedural specification (system prompt / task instructions).
//! - Σ_t: Mutable structured execution state (JSON map bounded by domain schema).
//! - O_t: Latest observation from the environment (tool execution output).
//!
//! The model outputs (R_t, ΔΣ_t, a_t):
//! - R_t: Reasoning / thoughts. Permanently discarded after state update.
//! - ΔΣ_t: `state_patch` (JSON dict of mutations; keys mapped to `null` are deleted).
//! - a_t: Action to execute (tool call or command).
//!
//! Complexity:
//! - Per-step prompt size: O(1) bounded.
//! - Cumulative tokens over T steps: O(T) linear (vs O(T²) quadratic for standard history).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// Structured mutable execution state Σ for an autonomous agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillState {
    /// Mutable structured state dictionary Σ.
    #[serde(default)]
    pub data: Map<String, Value>,
    /// Optional domain schema restricting allowed keys.
    /// If non-empty, out-of-schema keys are dropped during merge to prevent state bloat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schema: Vec<String>,
    /// Execution step counter t.
    #[serde(default)]
    pub step: usize,
}

impl Default for SkillState {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillState {
    /// Creates a new empty execution state with no schema restrictions.
    pub fn new() -> Self {
        Self {
            data: Map::new(),
            schema: Vec::new(),
            step: 0,
        }
    }

    /// Creates a state with an initial structured payload and domain schema.
    pub fn with_schema(schema: Vec<String>) -> Self {
        Self {
            data: Map::new(),
            schema,
            step: 0,
        }
    }

    /// Returns true if the state currently holds no data keys.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns the number of state keys.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Compact JSON representation of Σ for injection into prompts (no unnecessary whitespace).
    pub fn to_compact_json(&self) -> String {
        serde_json::to_string(&self.data).unwrap_or_else(|_| "{}".to_string())
    }

    /// Pretty JSON representation for human inspection and logging.
    pub fn to_pretty_json(&self) -> String {
        serde_json::to_string_pretty(&self.data).unwrap_or_else(|_| "{}".to_string())
    }

    /// Applies a state mutation patch ΔΣ using paper §3.1 dictionary merge with null-deletion semantics:
    /// - If a key's value is `null`, the key is removed from Σ.
    /// - Nested objects are merged recursively.
    /// - If `self.schema` is defined, keys outside the schema are dropped.
    /// - Increments the step counter t.
    pub fn apply_patch(&mut self, patch: &Map<String, Value>) -> StateUpdateReport {
        let mut report = StateUpdateReport::default();
        let mut safe_patch = Map::new();

        for (k, v) in patch {
            safe_patch.insert(k.clone(), v.clone());
        }

        self.data = merge_json_maps(&self.data, &safe_patch, &mut report);

        // Enforce schema boundaries: drop any keys not explicitly declared in schema
        if !self.schema.is_empty() {
            let schema_set: HashSet<&str> = self.schema.iter().map(|s| s.as_str()).collect();
            let keys_before: Vec<String> = self.data.keys().cloned().collect();
            for k in keys_before {
                if !schema_set.contains(k.as_str()) {
                    self.data.remove(&k);
                    report.dropped_out_of_schema_keys.push(k);
                }
            }
        }

        self.step += 1;
        report.new_step = self.step;
        report
    }

    /// Builds the compact step prompt (P, Σ_t, O_t) conforming to paper §3.2 & Appendix A.4.
    pub fn format_step_prompt(
        &self,
        system_spec: &str,
        latest_observation: &str,
    ) -> (String, String) {
        let state_json = self.to_compact_json();
        let system = format!(
            "{}\n\nSkill Execution State (Σ):\n```json\n{}\n```",
            system_spec.trim(),
            state_json
        );

        let user = format!(
            "Latest Observation (O):\n{}\n\nProvide your response with:\n\
            1. Step-by-step reasoning (R_t) - discarded after validated state update.\n\
            2. A single fenced ```json block containing your state patch and next action:\n\
            ```json\n\
            {{\n  \
              \"state_patch\": {{ <key mutations, assign null to delete keys> }},\n  \
              \"action\": \"<command or next tool to execute>\"\n\
            }}\n\
            ```",
            latest_observation.trim()
        );

        (system, user)
    }
}

/// Outcome report of applying a state patch ΔΣ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateUpdateReport {
    pub keys_added: Vec<String>,
    pub keys_updated: Vec<String>,
    pub keys_deleted: Vec<String>,
    pub dropped_out_of_schema_keys: Vec<String>,
    pub new_step: usize,
}

/// Recursive JSON object merge implementing paper §3.1 null-deletion semantics.
fn merge_json_maps(
    base: &Map<String, Value>,
    delta: &Map<String, Value>,
    report: &mut StateUpdateReport,
) -> Map<String, Value> {
    let mut next = base.clone();

    for (k, v) in delta {
        if v.is_null() {
            if next.remove(k).is_some() {
                report.keys_deleted.push(k.clone());
            }
        } else if let (Some(Value::Object(base_obj)), Value::Object(delta_obj)) = (next.get(k), v) {
            let merged_sub = merge_json_maps(base_obj, delta_obj, report);
            next.insert(k.clone(), Value::Object(merged_sub));
            report.keys_updated.push(k.clone());
        } else {
            if next.contains_key(k) {
                report.keys_updated.push(k.clone());
            } else {
                report.keys_added.push(k.clone());
            }
            next.insert(k.clone(), v.clone());
        }
    }

    next
}

/// Parsed output from an agent forward pass (R_t, ΔΣ_t, a_t).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPatch {
    /// Extracted state mutations ΔΣ.
    pub state_patch: Map<String, Value>,
    /// Optional action / command extracted from the output.
    pub action: Option<String>,
    /// Reasoning trace R_t (discarded before subsequent prompt).
    pub reasoning: String,
    /// True if a valid paper-format `state_patch` was successfully parsed.
    pub is_valid: bool,
}

/// Extracts (R_t, ΔΣ_t, a_t) from model response text according to priority format detection:
/// 1. Fenced ```json block with `state_patch` / `delta` / `sigma` key.
/// 2. Whole-output JSON object with `state_patch`.
/// 3. Legacy inline marker `STATE: { ... }` or `ΔΣ: { ... }`.
pub fn extract_state_patch(text: &str) -> ExtractedPatch {
    let trimmed = text.trim();

    // 1. Check for fenced code block: ```json ... ``` or ```state ... ```
    if let Some(start_idx) = trimmed.find("```") {
        let after_fence = &trimmed[start_idx + 3..];
        let content_start = if let Some(newline) = after_fence.find('\n') {
            start_idx + 3 + newline + 1
        } else {
            start_idx + 3
        };

        if let Some(end_rel) = trimmed[content_start..].find("```") {
            let json_body = &trimmed[content_start..content_start + end_rel].trim();
            if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(json_body) {
                if let Some((patch, action)) = parse_object_for_patch(&obj) {
                    let mut reasoning = trimmed[..start_idx].trim().to_string();
                    let post_fence = trimmed[content_start + end_rel + 3..].trim();
                    if !post_fence.is_empty() {
                        if !reasoning.is_empty() {
                            reasoning.push_str("\n\n");
                        }
                        reasoning.push_str(post_fence);
                    }

                    return ExtractedPatch {
                        state_patch: patch,
                        action,
                        reasoning,
                        is_valid: true,
                    };
                }
            }
        }
    }

    // 2. Whole-output JSON: entire output is a JSON payload with state_patch
    if (trimmed.starts_with('{') && trimmed.ends_with('}')) {
        if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(trimmed) {
            if let Some((patch, action)) = parse_object_for_patch(&obj) {
                return ExtractedPatch {
                    state_patch: patch,
                    action,
                    reasoning: String::new(),
                    is_valid: true,
                };
            }
        }
    }

    // 3. Inline marker: STATE: { ... } or ΔΣ: { ... }
    for marker in &["STATE:", "state_patch:", "ΔΣ:", "DELTA:"] {
        if let Some(pos) = trimmed.find(marker) {
            let candidate = trimmed[pos + marker.len()..].trim_start();
            if candidate.starts_with('{') {
                if let Some(close_idx) = candidate.rfind('}') {
                    let json_str = &candidate[..=close_idx];
                    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(json_str) {
                        return ExtractedPatch {
                            state_patch: obj,
                            action: None,
                            reasoning: trimmed[..pos].trim().to_string(),
                            is_valid: true,
                        };
                    }
                }
            }
        }
    }

    // Invalid or missing state patch
    ExtractedPatch {
        state_patch: Map::new(),
        action: None,
        reasoning: trimmed.to_string(),
        is_valid: false,
    }
}

fn parse_object_for_patch(
    obj: &Map<String, Value>,
) -> Option<(Map<String, Value>, Option<String>)> {
    let action = obj
        .get("action")
        .or_else(|| obj.get("command"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    for key in &["state_patch", "statePatch", "delta", "state", "sigma", "Σ"] {
        if let Some(Value::Object(patch)) = obj.get(*key) {
            return Some((patch.clone(), action));
        }
    }

    // Direct object without wrapper
    Some((obj.clone(), action))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_state_null_deletion() {
        let mut state = SkillState::new();

        let mut initial = Map::new();
        initial.insert("target_file".to_string(), json!("src/main.rs"));
        initial.insert("bug_fixed".to_string(), json!(false));
        initial.insert("notes".to_string(), json!("initial hypothesis"));

        let r1 = state.apply_patch(&initial);
        assert_eq!(r1.keys_added.len(), 3);
        assert_eq!(state.data.get("bug_fixed"), Some(&json!(false)));
        assert_eq!(state.step, 1);

        // Mutate bug_fixed to true, delete notes using null-deletion
        let mut patch = Map::new();
        patch.insert("bug_fixed".to_string(), json!(true));
        patch.insert("notes".to_string(), Value::Null);
        patch.insert("new_fact".to_string(), json!("tests passing"));

        let r2 = state.apply_patch(&patch);
        assert_eq!(r2.keys_updated, vec!["bug_fixed".to_string()]);
        assert_eq!(r2.keys_deleted, vec!["notes".to_string()]);
        assert_eq!(r2.keys_added, vec!["new_fact".to_string()]);

        assert_eq!(state.data.get("bug_fixed"), Some(&json!(true)));
        assert!(state.data.get("notes").is_none());
        assert_eq!(state.data.get("new_fact"), Some(&json!("tests passing")));
        assert_eq!(state.step, 2);
    }

    #[test]
    fn test_schema_enforcement_drops_out_of_schema_keys() {
        let mut state =
            SkillState::with_schema(vec!["active_phase".to_string(), "target_files".to_string()]);

        let mut patch = Map::new();
        patch.insert("active_phase".to_string(), json!("testing"));
        patch.insert("unauthorized_bloat".to_string(), json!("large text dump"));

        let report = state.apply_patch(&patch);
        assert!(state.data.contains_key("active_phase"));
        assert!(!state.data.contains_key("unauthorized_bloat"));
        assert_eq!(
            report.dropped_out_of_schema_keys,
            vec!["unauthorized_bloat".to_string()]
        );
    }

    #[test]
    fn test_extract_state_patch_fenced_json() {
        let response = r#"
We need to read Cargo.toml and update our dependency tracking.

```json
{
  "state_patch": {
    "explored_files": ["Cargo.toml"],
    "current_hypothesis": "missing tokio feature"
  },
  "action": "read Cargo.toml"
}
```

This will confirm the version.
"#;

        let extracted = extract_state_patch(response);
        assert!(extracted.is_valid);
        assert_eq!(extracted.action.as_deref(), Some("read Cargo.toml"));
        assert_eq!(
            extracted.state_patch.get("current_hypothesis"),
            Some(&json!("missing tokio feature"))
        );
        assert!(extracted.reasoning.contains("We need to read Cargo.toml"));
        assert!(extracted
            .reasoning
            .contains("This will confirm the version."));
    }

    #[test]
    fn test_format_step_prompt_bounded_structure() {
        let mut state = SkillState::new();
        let mut patch = Map::new();
        patch.insert("status".to_string(), json!("in_progress"));
        state.apply_patch(&patch);

        let (sys, usr) =
            state.format_step_prompt("TASK: Fix race condition", "Ran cargo test: 1 failed");

        assert!(sys.contains("Skill Execution State (Σ)"));
        assert!(sys.contains(r#"{"status":"in_progress"}"#));
        assert!(usr.contains("Latest Observation (O)"));
        assert!(usr.contains("Ran cargo test: 1 failed"));
        assert!(usr.contains("state_patch"));
    }
}
