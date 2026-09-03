use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::agent::advisor::{
    consult_advisors, format_critiques_for_system_prompt, AdvisorRegistry,
};
use crate::agent::prompts::{self, PromptPreset, SystemPromptBuilder};
use crate::agent::session::Session;
use crate::agent::skills::SkillRegistry;
use crate::config::Config;
use crate::provider::types::{Message, StreamChunk, ToolCall};
use crate::provider::LlmClient;
use crate::tools::types::{ToolContext, ToolRegistry};

/// High-level events emitted during an agent execution turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    /// Incremental assistant text chunk.
    TextDelta(String),
    /// Incremental model thinking/reasoning chunk.
    ThinkingDelta(String),
    /// Tool execution started.
    ToolStarted {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    /// Tool execution completed.
    ToolFinished {
        id: String,
        name: String,
        success: bool,
        output: String,
        duration: Duration,
    },
    /// Advisor review started.
    AdvisorStarted { advisor: String, role: String },
    /// Advisor critique received.
    AdvisorCritique {
        advisor: String,
        approved: bool,
        critique: String,
    },
    /// Subagent task started.
    SubagentStarted { name: String, task: String },
    /// Subagent task finished.
    SubagentFinished {
        name: String,
        success: bool,
        output: String,
    },
    /// Informational status update.
    Status(String),
    /// Turn completed.
    Finished { usage: Option<serde_json::Value> },
    /// Error encountered during execution.
    Error(String),
}

/// The core execution engine coordinating LLM streaming, advisor critiques, and tool calls.
#[derive(Clone)]
pub struct AgentRunner {
    client: LlmClient,
    config: Config,
    tools: ToolRegistry,
    tool_ctx: ToolContext,
    advisors: AdvisorRegistry,
    skills: SkillRegistry,
    corrections: crate::agent::correction::CorrectionEngine,
    recovery: crate::agent::recovery::RecoveryManager,
    system_prompt: Option<String>,
    checkpoints: std::sync::Arc<std::sync::Mutex<crate::agent::undo::CheckpointManager>>,
    max_turns: usize,
}
impl AgentRunner {
    /// Creates a new AgentRunner with default advisors.
    pub fn new(
        client: LlmClient,
        config: Config,
        tools: ToolRegistry,
        tool_ctx: ToolContext,
    ) -> Self {
        let max_turns = config.max_turns.unwrap_or(100).clamp(10, 500);
        Self {
            client,
            config,
            tools,
            tool_ctx: tool_ctx.clone(),
            advisors: AdvisorRegistry::default_advisors(),
            skills: SkillRegistry::scan_default(Some(&tool_ctx.cwd)),
            corrections: crate::agent::correction::CorrectionEngine::default(),
            recovery: crate::agent::recovery::RecoveryManager::new(tool_ctx.cwd.clone()),
            checkpoints: std::sync::Arc::new(std::sync::Mutex::new(
                crate::agent::undo::CheckpointManager::new(tool_ctx.cwd.clone()),
            )),
            system_prompt: None,
            max_turns,
        }
    }

    /// Sets a custom AdvisorRegistry.
    pub fn with_advisors(mut self, advisors: AdvisorRegistry) -> Self {
        self.advisors = advisors;
        self
    }

    /// Sets a custom SkillRegistry.
    pub fn with_skills(mut self, skills: SkillRegistry) -> Self {
        self.skills = skills;
        self
    }
    /// Returns a shared reference to the CheckpointManager.
    pub fn checkpoints(
        &self,
    ) -> std::sync::Arc<std::sync::Mutex<crate::agent::undo::CheckpointManager>> {
        self.checkpoints.clone()
    }

    /// Sets a custom CheckpointManager.
    pub fn with_checkpoints(
        mut self,
        checkpoints: std::sync::Arc<std::sync::Mutex<crate::agent::undo::CheckpointManager>>,
    ) -> Self {
        self.checkpoints = checkpoints;
        self
    }

    /// Sets a custom CorrectionConfig for the self-correcting retry loop.
    pub fn with_correction_config(
        mut self,
        config: crate::agent::correction::CorrectionConfig,
    ) -> Self {
        self.corrections = crate::agent::correction::CorrectionEngine::new(config);
        self
    }

    /// Enables or disables automatic silent tool error recovery.
    pub fn with_auto_correction(mut self, enabled: bool) -> Self {
        self.corrections.config.enable_auto_retry = enabled;
        self
    }

    /// Returns a reference to the CorrectionEngine.
    pub fn corrections(&self) -> &crate::agent::correction::CorrectionEngine {
        &self.corrections
    }

    /// Returns a reference to the RecoveryManager.
    pub fn recovery(&self) -> &crate::agent::recovery::RecoveryManager {
        &self.recovery
    }

    /// Sets a custom RecoveryManager.
    pub fn with_recovery(mut self, recovery: crate::agent::recovery::RecoveryManager) -> Self {
        self.recovery = recovery;
        self
    }

    /// Enables or disables automatic turn recovery snapshots.
    pub fn with_recovery_enabled(mut self, enabled: bool) -> Self {
        self.recovery.set_enabled(enabled);
        self
    }

    /// Overrides the default system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Sets a language-specific system prompt preset.
    pub fn with_preset(mut self, preset: PromptPreset) -> Self {
        let prompt = SystemPromptBuilder::new().with_preset(preset).build();
        self.system_prompt = Some(prompt);
        self
    }

    /// Automatically detects the appropriate preset based on the workspace directory.
    pub fn with_detected_preset(mut self, workspace_path: &std::path::Path) -> Self {
        let prompt = SystemPromptBuilder::new()
            .with_workspace_detection(workspace_path)
            .build();
        self.system_prompt = Some(prompt);
        self
    }

    /// Sets the maximum tool loop turns per user turn.
    pub fn with_max_turns(mut self, max: usize) -> Self {
        self.max_turns = max;
        self
    }

    /// Returns the maximum tool loop turns per user turn.
    pub fn max_turns(&self) -> usize {
        self.max_turns
    }

    /// Returns a reference to the LLM client.
    pub fn client(&self) -> &LlmClient {
        &self.client
    }

    /// Returns a reference to the current Config.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns a mutable reference to the Config.
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Returns a reference to the ToolRegistry.
    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    /// Returns a reference to the ToolContext.
    pub fn tool_ctx(&self) -> &ToolContext {
        &self.tool_ctx
    }

    /// Returns a reference to the AdvisorRegistry.
    pub fn advisors(&self) -> &AdvisorRegistry {
        &self.advisors
    }

    /// Returns a reference to the SkillRegistry.
    pub fn skills(&self) -> &SkillRegistry {
        &self.skills
    }

    /// Returns a mutable reference to the SkillRegistry.
    pub fn skills_mut(&mut self) -> &mut SkillRegistry {
        &mut self.skills
    }

    /// Default system prompt for Fusion coding assistant.
    pub fn default_system_prompt() -> &'static str {
        prompts::general_system_prompt()
    }

    /// Runs a single turn synchronously/CLI-style, printing tokens directly to stdout.
    pub async fn run_turn(
        &self,
        session: &mut Session,
        user_input: &str,
    ) -> anyhow::Result<String> {
        let (tx, mut rx) = unbounded_channel();

        // Spawn a background task to handle event printing
        let print_task = tokio::spawn(async move {
            let mut md = crate::ui::markdown::MarkdownRenderer::new();
            while let Some(event) = rx.recv().await {
                match event {
                    AgentEvent::TextDelta(delta) => {
                        md.push(&delta);
                    }
                    AgentEvent::ThinkingDelta(_delta) => {
                        // Suppress raw internal chain-of-thought
                    }
                    AgentEvent::ToolStarted { name, args, .. } => {
                        md.finish();
                        println!("\n⚙️  Tool [{}] with args: {}", name, args);
                    }
                    AgentEvent::ToolFinished {
                        name,
                        success,
                        output,
                        duration,
                        ..
                    } => {
                        let status = if success { "✓" } else { "✗" };
                        let preview: String = output.lines().take(3).collect::<Vec<_>>().join("\n");
                        println!(
                            "  {} Tool [{}] finished in {:.2?}: {}",
                            status, name, duration, preview
                        );
                    }
                    AgentEvent::AdvisorStarted { advisor, role } => {
                        tracing::debug!("Advisor [{}] started: {}", advisor, role);
                    }
                    AgentEvent::AdvisorCritique {
                        advisor,
                        approved,
                        critique,
                    } => {
                        let tag = if approved { "APPROVED" } else { "WARNING" };
                        tracing::info!("Advisor [{}] {}: {}", advisor, tag, critique);
                    }
                    AgentEvent::Status(status) => {
                        tracing::debug!("Status: {}", status);
                    }
                    AgentEvent::Error(err) => {
                        eprintln!("\n❌ Error: {}", err);
                    }
                    AgentEvent::Finished { .. } => {
                        md.finish();
                    }
                    _ => {}
                }
            }
            md.finish();
            println!();
        });

        let result = self.run_turn_stream(session, user_input, tx).await;
        let _ = print_task.await;
        result
    }

    /// Executes a full conversation turn, streaming events through the provided channel.
    pub async fn run_turn_stream(
        &self,
        session: &mut Session,
        user_input: &str,
        event_tx: UnboundedSender<AgentEvent>,
    ) -> anyhow::Result<String> {
        // Auto-save recovery state immediately before starting conversation turn
        let _ = self.recovery.on_turn_start(session, user_input, 1);

        // Record user input
        session.add_user_message(user_input);

        // Advisor consultation phase (if enabled)
        let mut advisor_notes = String::new();
        if self.config.advisors_enabled && !self.advisors.is_empty() {
            let _ = event_tx.send(AgentEvent::Status(
                "Consulting advisors in parallel...".to_string(),
            ));
            let _ = self.recovery.on_advisor_phase();
            for adv in self.advisors.all() {
                let _ = event_tx.send(AgentEvent::AdvisorStarted {
                    advisor: adv.name.clone(),
                    role: adv.focus.clone(),
                });
            }

            let critiques = consult_advisors(
                self.advisors.all(),
                user_input,
                "",
                &self.client,
                &self.config,
            )
            .await;

            for critique in &critiques {
                let _ = event_tx.send(AgentEvent::AdvisorCritique {
                    advisor: critique.advisor.clone(),
                    approved: critique.approved,
                    critique: critique.critique.clone(),
                });
            }

            advisor_notes = format_critiques_for_system_prompt(&critiques);
        }

        let default_prompt = Self::default_system_prompt();
        let base_system_prompt = self.system_prompt.as_deref().unwrap_or(default_prompt);

        let mut system_message_content = base_system_prompt.to_string();

        // Domain skills dynamic injection
        let relevant_matches = self
            .skills
            .find_relevant(user_input, Some(&self.tool_ctx.cwd));
        if !relevant_matches.is_empty() {
            let skill_names: Vec<&str> = relevant_matches.iter().map(|m| m.skill.name()).collect();
            let _ = event_tx.send(AgentEvent::Status(format!(
                "Active domain skills injected: {}",
                skill_names.join(", ")
            )));

            if let Some(skills_block) =
                self.skills
                    .inject_relevant_skills(user_input, Some(&self.tool_ctx.cwd), Some(4))
            {
                system_message_content.push_str("\n\n");
                system_message_content.push_str(&skills_block);
            }
        }

        if !advisor_notes.is_empty() {
            system_message_content.push_str("\n\n");
            system_message_content.push_str(&advisor_notes);
        }

        let mut turn = 0;
        let tool_defs = self.tools.definitions();

        while turn < self.max_turns {
            turn += 1;

            // Auto-compact conversation history if it exceeds the context budget (>= 80%)
            let compactor = crate::agent::compaction::Compactor::default();
            if compactor.needs_compaction(session.messages(), session.active_model()) {
                let _ = event_tx.send(AgentEvent::Status(
                    "Context window limit approaching. Compacting history...".to_string(),
                ));
                let compaction_result = compactor.compact_session(session);
                if compaction_result.compacted {
                    let _ = event_tx.send(AgentEvent::Status(compaction_result.format_summary()));
                }
            }

            // Assemble message payload:
            // When SKILL.state Σ is initialized, assemble bounded (P, Σ_t, O_t) prompt
            // keeping per-step prompt size bounded to O(1) tokens (arXiv:2608.26263).
            let mut messages = Vec::new();
            let mut sys_content = system_message_content.clone();
            if !session.execution_state.is_empty() {
                sys_content.push_str("\n\nSkill Execution State (Σ):\n```json\n");
                sys_content.push_str(&session.execution_state.to_compact_json());
                sys_content.push_str("\n```");
            }

            if turn > 1 {
                sys_content.push_str("\n\nCRITICAL DIRECTIVE: Tool results have been returned above. Synthesize your findings, code analysis, and answers directly to fulfill the user's prompt. Do NOT ask rhetorical follow-up questions (such as 'Would you like me to dive deeper or help refactor?') or offer menus of choices instead of answering. Present your full response, technical evaluation, and findings immediately.");
            }
            messages.push(Message::system(&sys_content));
            messages.extend_from_slice(session.messages());
            let _ = event_tx.send(AgentEvent::Status(
                "Waiting for model response...".to_string(),
            ));

            // Stream response from LLM
            let mut chunk_stream = match self
                .client
                .stream_chat(&self.config, &messages, &tool_defs)
                .await
            {
                Ok(rx) => rx,
                Err(e) => {
                    let err_msg = format!("{}", e);
                    let _ = self.recovery.on_turn_error(&err_msg);
                    let _ = event_tx.send(AgentEvent::Error(err_msg.clone()));
                    anyhow::bail!(err_msg);
                }
            };

            let mut full_content = String::new();
            let mut full_thinking = String::new();
            let mut partial_tools: HashMap<usize, (Option<String>, Option<String>, String)> =
                HashMap::new();

            while let Some(chunk) = chunk_stream.recv().await {
                match chunk {
                    StreamChunk::ContentDelta(delta) => {
                        full_content.push_str(&delta);
                        let _ = event_tx.send(AgentEvent::TextDelta(delta));
                    }
                    StreamChunk::ThinkingDelta(delta) => {
                        full_thinking.push_str(&delta);
                        let _ = event_tx.send(AgentEvent::ThinkingDelta(delta));
                    }
                    StreamChunk::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments_delta,
                    } => {
                        let entry =
                            partial_tools
                                .entry(index)
                                .or_insert((None, None, String::new()));
                        if let Some(id_val) = id {
                            entry.0 = Some(id_val);
                        }
                        if let Some(name_val) = name {
                            entry.1 = Some(name_val);
                        }
                        entry.2.push_str(&arguments_delta);
                    }
                    StreamChunk::Done { .. } => {
                        break;
                    }
                    StreamChunk::Error(err) => {
                        let _ = self.recovery.on_turn_error(&err);
                        let _ = event_tx.send(AgentEvent::Error(err.clone()));
                        anyhow::bail!("LLM stream error: {}", err);
                    }
                }
            }

            // Build completed tool calls from stream chunks
            let mut tool_calls = Vec::new();
            let mut sorted_indices: Vec<usize> = partial_tools.keys().copied().collect();
            sorted_indices.sort_unstable();

            for idx in sorted_indices {
                if let Some((id_opt, name_opt, args)) = partial_tools.remove(&idx) {
                    if let (Some(id), Some(name)) = (id_opt, name_opt) {
                        tool_calls.push(ToolCall {
                            id,
                            name,
                            arguments: args,
                        });
                    }
                }
            }

            // SKILL.state: Check if the model emitted a structured state patch ΔΣ (arXiv:2608.26263)
            let raw_for_patch = if !full_content.trim().is_empty() {
                &full_content
            } else {
                &full_thinking
            };
            let extracted_patch = crate::agent::skill_state::extract_state_patch(raw_for_patch);
            if extracted_patch.is_valid && !extracted_patch.state_patch.is_empty() {
                let update_report = session
                    .execution_state
                    .apply_patch(&extracted_patch.state_patch);
                let _ = event_tx.send(AgentEvent::Status(format!(
                    "SKILL.state Σ update: step {} (+{} ~{} -{})",
                    update_report.new_step,
                    update_report.keys_added.len(),
                    update_report.keys_updated.len(),
                    update_report.keys_deleted.len()
                )));
            }

            if tool_calls.is_empty() {
                let final_content = if full_content.trim().is_empty() {
                    if !full_thinking.trim().is_empty() {
                        let clean_answer =
                            if let Some((_, ans)) = full_thinking.split_once("</think>") {
                                ans.trim().to_string()
                            } else if let Some((_, ans)) = full_thinking.rsplit_once("\n\n") {
                                if !ans.trim().is_empty() {
                                    ans.trim().to_string()
                                } else {
                                    full_thinking.trim().to_string()
                                }
                            } else {
                                full_thinking.trim().to_string()
                            };
                        let _ = event_tx.send(AgentEvent::TextDelta(clean_answer.clone()));
                        clean_answer
                    } else {
                        "(empty response)".to_string()
                    }
                } else {
                    full_content
                };
                session.add_assistant_message(&final_content);
                let _ = event_tx.send(AgentEvent::Finished { usage: None });
                let _ = session.save();
                let _ = self.recovery.on_turn_completed(session);
                return Ok(final_content);
            }

            // Record assistant message with tool calls
            let assistant_content = if full_content.trim().is_empty() {
                if !full_thinking.trim().is_empty() {
                    format!("<think>{}</think>", full_thinking.trim())
                } else {
                    "Executing tools...".to_string()
                }
            } else {
                full_content
            };
            session.add_assistant_with_tools(&assistant_content, tool_calls.clone());
            // Execute each tool call in sequence
            for tc in tool_calls {
                let parsed_args = match serde_json::from_str::<serde_json::Value>(&tc.arguments) {
                    Ok(v) => v,
                    Err(e) => {
                        let err_msg = format!("Invalid JSON arguments: {}", e);
                        let _ = event_tx.send(AgentEvent::ToolStarted {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            args: serde_json::Value::Null,
                        });
                        let _ = event_tx.send(AgentEvent::ToolFinished {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            success: false,
                            output: err_msg.clone(),
                            duration: Duration::ZERO,
                        });
                        session.add_tool_result(&tc.id, &err_msg);
                        continue;
                    }
                };

                let _ = event_tx.send(AgentEvent::ToolStarted {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    args: parsed_args.clone(),
                });
                let _ = self.recovery.on_tool_start(&tc.name, &tc.id, &parsed_args);
                // Capture pre-tool file snapshots for reliable checkpoint undo
                let checkpoint_id = match self.checkpoints.lock() {
                    Ok(mut mgr) => mgr
                        .capture_before_tool(&tc.name, &parsed_args, &self.tool_ctx.cwd)
                        .ok()
                        .flatten(),
                    Err(_) => None,
                };

                let start = Instant::now();
                let outcome = self
                    .corrections
                    .execute_with_registry(&tc.name, parsed_args, &self.tool_ctx, &self.tools)
                    .await;
                let duration = start.elapsed();

                // Capture post-execution file state for diff inspection and redo
                if let Some(chk_id) = &checkpoint_id {
                    if let Ok(mut mgr) = self.checkpoints.lock() {
                        let _ = mgr.capture_after_tool(chk_id, &self.tool_ctx.cwd);
                    }
                }

                match outcome {
                    crate::agent::correction::CorrectionOutcome::Success {
                        output,
                        was_corrected,
                        total_corrections,
                        ..
                    } => {
                        if was_corrected {
                            let _ = event_tx.send(AgentEvent::Status(format!(
                                "Tool '{}' self-corrected after {} recovery attempt(s)",
                                tc.name, total_corrections
                            )));
                        }
                        let _ = event_tx.send(AgentEvent::ToolFinished {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            success: true,
                            output: output.clone(),
                            duration,
                        });
                        session.add_tool_result(&tc.id, &output);
                        let _ = self.recovery.on_tool_finish(
                            &tc.name,
                            &tc.id,
                            &tc.arguments,
                            true,
                            &output,
                            duration,
                        );
                    }
                    crate::agent::correction::CorrectionOutcome::Failed {
                        enriched_diagnostic,
                        ..
                    } => {
                        let _ = event_tx.send(AgentEvent::ToolFinished {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            success: false,
                            output: enriched_diagnostic.clone(),
                            duration,
                        });
                        session.add_tool_result(&tc.id, &enriched_diagnostic);
                        let _ = self.recovery.on_tool_finish(
                            &tc.name,
                            &tc.id,
                            &tc.arguments,
                            false,
                            &enriched_diagnostic,
                            duration,
                        );
                    }
                }
            }
        }

        // Agent reached maximum execution turns. Check for active file edits and ongoing
        // tasks, save session state, and provide a helpful resumption message rather than a crash error.
        let mut modified_files = std::collections::BTreeSet::new();
        let mut edit_count = 0;

        if let Ok(mgr) = self.checkpoints.lock() {
            let active = mgr.list_checkpoints();
            edit_count = active.len();
            for chk in &active {
                for path in &chk.files {
                    modified_files.insert(path.to_string_lossy().to_string());
                }
            }
        }

        if modified_files.is_empty() {
            let patch = session.session_patch();
            for path in patch.file_paths() {
                modified_files.insert(path.to_string_lossy().to_string());
            }
            if edit_count == 0 {
                edit_count = patch.file_count();
            }
        }

        let last_tool_calls: Vec<String> = session
            .messages()
            .iter()
            .rev()
            .find_map(|m| {
                m.tool_calls.as_ref().and_then(|calls| {
                    if !calls.is_empty() {
                        Some(calls.iter().map(|tc| tc.name.clone()).collect())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();

        let has_skill_state = !session.execution_state.is_empty();

        let mut resume_msg = format!("Agent completed {} turns.\n\n", self.max_turns);

        if !modified_files.is_empty() {
            resume_msg.push_str(&format!(
                "Active file edits ({} file{} modified):\n",
                modified_files.len(),
                if modified_files.len() == 1 { "" } else { "s" }
            ));
            for file in modified_files.iter().take(10) {
                resume_msg.push_str(&format!("  • {}\n", file));
            }
            if modified_files.len() > 10 {
                resume_msg.push_str(&format!(
                    "  • ... and {} more files\n",
                    modified_files.len() - 10
                ));
            }
            resume_msg.push('\n');
        } else if edit_count > 0 {
            resume_msg.push_str(&format!(
                "Active file edits: {} checkpoint(s) saved.\n\n",
                edit_count
            ));
        }

        if !last_tool_calls.is_empty() {
            resume_msg.push_str(&format!(
                "Ongoing tasks / last tools executed: {}\n\n",
                last_tool_calls.join(", ")
            ));
        }

        if has_skill_state {
            resume_msg.push_str(&format!(
                "Task execution state: Step {}\n\n",
                session.execution_state.step
            ));
        }

        resume_msg.push_str(
            "Session state has been saved. You can continue execution at any time by asking the agent to continue or providing the next prompt."
        );

        let _ = event_tx.send(AgentEvent::TextDelta(resume_msg.clone()));
        let _ = event_tx.send(AgentEvent::Finished { usage: None });
        session.add_assistant_message(&resume_msg);
        let _ = session.save();
        let _ = self.recovery.on_turn_completed(session);

        Ok(resume_msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_runner_builder() {
        let client = LlmClient::new();
        let config = Config::default();
        let tools = ToolRegistry::new();
        let ctx = ToolContext::default();

        let runner = AgentRunner::new(client, config, tools, ctx)
            .with_max_turns(10)
            .with_system_prompt("Custom test prompt");

        assert_eq!(runner.max_turns, 10);
        assert_eq!(runner.system_prompt.as_deref(), Some("Custom test prompt"));
        assert_eq!(runner.advisors.len(), 3);
    }

    #[test]
    fn test_agent_runner_with_preset() {
        let client = LlmClient::new();
        let config = Config::default();
        let tools = ToolRegistry::new();
        let ctx = ToolContext::default();

        let runner = AgentRunner::new(client, config, tools, ctx).with_preset(PromptPreset::Rust);

        let prompt = runner.system_prompt.as_deref().unwrap();
        assert!(prompt.contains("expert Rust systems"));
    }

    #[test]
    fn test_agent_event_serialization() {
        let event = AgentEvent::ToolStarted {
            id: "call_1".into(),
            name: "read".into(),
            args: serde_json::json!({ "path": "src/main.rs" }),
        };

        let json = serde_json::to_string(&event).expect("serialize event");
        let deserialized: AgentEvent = serde_json::from_str(&json).expect("deserialize event");

        match deserialized {
            AgentEvent::ToolStarted { id, name, .. } => {
                assert_eq!(id, "call_1");
                assert_eq!(name, "read");
            }
            _ => panic!("Unexpected event variant"),
        }
    }

    #[test]
    fn test_empty_content_fallback_resolution() {
        let full_content = "";
        let full_thinking = "I should plan carefully.";
        let resolved = if full_content.trim().is_empty() {
            if !full_thinking.trim().is_empty() {
                format!("<think>{}</think>", full_thinking.trim())
            } else {
                "(empty response)".to_string()
            }
        } else {
            full_content.to_string()
        };
        assert_eq!(resolved, "<think>I should plan carefully.</think>");

        let empty_thinking = "   ";
        let resolved_empty = if full_content.trim().is_empty() {
            if !empty_thinking.trim().is_empty() {
                format!("<think>{}</think>", empty_thinking.trim())
            } else {
                "(empty response)".to_string()
            }
        } else {
            full_content.to_string()
        };
        assert_eq!(resolved_empty, "(empty response)");
    }

    #[test]
    fn test_max_turns_resume_message_structure() {
        let max_turns = 100;
        let mut modified_files = std::collections::BTreeSet::new();
        modified_files.insert("src/main.rs".to_string());
        modified_files.insert("package.json".to_string());
        let _edit_count = 2;
        let last_tool_calls = vec!["edit".to_string(), "bash".to_string()];

        let mut resume_msg = format!("Agent completed {} turns.\n\n", max_turns);
        if !modified_files.is_empty() {
            resume_msg.push_str(&format!(
                "Active file edits ({} file{} modified):\n",
                modified_files.len(),
                if modified_files.len() == 1 { "" } else { "s" }
            ));
            for file in modified_files.iter().take(10) {
                resume_msg.push_str(&format!("  • {}\n", file));
            }
            resume_msg.push('\n');
        }
        if !last_tool_calls.is_empty() {
            resume_msg.push_str(&format!(
                "Ongoing tasks / last tools executed: {}\n\n",
                last_tool_calls.join(", ")
            ));
        }
        resume_msg.push_str(
            "Session state has been saved. You can continue execution at any time by asking the agent to continue or providing the next prompt."
        );

        assert!(resume_msg.contains("Agent completed 100 turns."));
        assert!(resume_msg.contains("Active file edits (2 files modified):"));
        assert!(resume_msg.contains("package.json"));
        assert!(resume_msg.contains("src/main.rs"));
        assert!(resume_msg.contains("Ongoing tasks / last tools executed: edit, bash"));
        assert!(resume_msg.contains("Session state has been saved."));
        assert!(resume_msg.contains("continue"));
    }

    #[test]
    fn test_agent_runner_default_max_turns() {
        let client = LlmClient::new();
        let config = Config::default();
        let tools = ToolRegistry::new();
        let ctx = ToolContext::default();

        let runner = AgentRunner::new(client, config, tools, ctx);
        assert_eq!(runner.max_turns(), 100);
    }

    #[test]
    fn test_agent_runner_custom_config_max_turns() {
        let client = LlmClient::new();
        let mut config = Config::default();
        config.max_turns = Some(150);
        let tools = ToolRegistry::new();
        let ctx = ToolContext::default();

        let runner = AgentRunner::new(client, config, tools, ctx);
        assert_eq!(runner.max_turns(), 150);
    }
}
