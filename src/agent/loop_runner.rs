use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::agent::advisor::{consult_advisors, format_critiques_for_system_prompt, AdvisorRegistry};
use crate::agent::session::Session;
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
    AdvisorStarted {
        advisor: String,
        role: String,
    },
    /// Advisor critique received.
    AdvisorCritique {
        advisor: String,
        approved: bool,
        critique: String,
    },
    /// Subagent task started.
    SubagentStarted {
        name: String,
        task: String,
    },
    /// Subagent task finished.
    SubagentFinished {
        name: String,
        success: bool,
        output: String,
    },
    /// Informational status update.
    Status(String),
    /// Turn completed.
    Finished {
        usage: Option<serde_json::Value>,
    },
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
    system_prompt: Option<String>,
    max_turns: usize,
}

impl AgentRunner {
    /// Creates a new AgentRunner with default advisors.
    pub fn new(client: LlmClient, config: Config, tools: ToolRegistry, tool_ctx: ToolContext) -> Self {
        Self {
            client,
            config,
            tools,
            tool_ctx,
            advisors: AdvisorRegistry::default_advisors(),
            system_prompt: None,
            max_turns: 30,
        }
    }

    /// Sets a custom AdvisorRegistry.
    pub fn with_advisors(mut self, advisors: AdvisorRegistry) -> Self {
        self.advisors = advisors;
        self
    }

    /// Overrides the default system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Sets the maximum tool loop turns per user turn.
    pub fn with_max_turns(mut self, max: usize) -> Self {
        self.max_turns = max;
        self
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

    /// Default system prompt for Fusion coding assistant.
    pub fn default_system_prompt() -> &'static str {
        r#"You are Fusion, a fast, lightweight, pure-Rust AI coding assistant.
You operate cleanly across macOS, Linux, Windows, and Android (Termux).

Principles:
- Be concise, direct, and technically rigorous. Deliver working solutions without unnecessary filler.
- Use provided tools (read, write, edit, grep, glob, bash) to inspect the workspace and perform actions.
- Prefer targeted reads/greps rather than dumping entire files unnecessarily.
- When modifying existing code, use precise edits. Ensure cross-platform compatibility.
- Zero-cost engineering: avoid redundant allocations, unhandled errors, or unverified assumptions."#
    }

    /// Runs a single turn synchronously/CLI-style, printing tokens directly to stdout.
    pub async fn run_turn(&self, session: &mut Session, user_input: &str) -> anyhow::Result<String> {
        let (tx, mut rx) = unbounded_channel();

        // Spawn a background task to handle event printing
        let print_task = tokio::spawn(async move {
            let mut stdout = std::io::stdout();
            while let Some(event) = rx.recv().await {
                match event {
                    AgentEvent::TextDelta(delta) => {
                        print!("{}", delta);
                        let _ = stdout.flush();
                    }
                    AgentEvent::ThinkingDelta(delta) => {
                        // Print thinking in subdued text if desired
                        eprint!("{}", delta);
                        let _ = std::io::stderr().flush();
                    }
                    AgentEvent::ToolStarted { name, args, .. } => {
                        println!("\n⚙️  Tool [{}] with args: {}", name, args);
                    }
                    AgentEvent::ToolFinished { name, success, output, duration, .. } => {
                        let status = if success { "✓" } else { "✗" };
                        let preview: String = output.lines().take(3).collect::<Vec<_>>().join("\n");
                        println!("  {} Tool [{}] finished in {:.2?}: {}", status, name, duration, preview);
                    }
                    AgentEvent::AdvisorStarted { advisor, role } => {
                        tracing::debug!("Advisor [{}] started: {}", advisor, role);
                    }
                    AgentEvent::AdvisorCritique { advisor, approved, critique } => {
                        let tag = if approved { "APPROVED" } else { "WARNING" };
                        tracing::info!("Advisor [{}] {}: {}", advisor, tag, critique);
                    }
                    AgentEvent::Status(status) => {
                        tracing::debug!("Status: {}", status);
                    }
                    AgentEvent::Error(err) => {
                        eprintln!("\n❌ Error: {}", err);
                    }
                    _ => {}
                }
            }
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
        // Record user input
        session.add_user_message(user_input);

        // Advisor consultation phase (if enabled)
        let mut advisor_notes = String::new();
        if self.config.advisors_enabled && !self.advisors.is_empty() {
            let _ = event_tx.send(AgentEvent::Status("Consulting advisors in parallel...".to_string()));
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
        let base_system_prompt = self
            .system_prompt
            .as_deref()
            .unwrap_or(default_prompt);

        let system_message_content = if advisor_notes.is_empty() {
            base_system_prompt.to_string()
        } else {
            format!("{}\n{}", base_system_prompt, advisor_notes)
        };

        let mut turn = 0;
        let tool_defs = self.tools.definitions();

        while turn < self.max_turns {
            turn += 1;

            // Assemble full message history
            let mut messages = Vec::with_capacity(session.messages().len() + 1);
            messages.push(Message::system(&system_message_content));
            messages.extend_from_slice(session.messages());

            let _ = event_tx.send(AgentEvent::Status("Waiting for model response...".to_string()));

            // Stream response from LLM
            let mut chunk_stream = match self.client.stream_chat(&self.config, &messages, &tool_defs).await {
                Ok(rx) => rx,
                Err(e) => {
                    let err_msg = format!("Failed to connect to provider: {}", e);
                    let _ = event_tx.send(AgentEvent::Error(err_msg.clone()));
                    anyhow::bail!(err_msg);
                }
            };

            let mut full_content = String::new();
            let mut full_thinking = String::new();
            let mut partial_tools: HashMap<usize, (Option<String>, Option<String>, String)> = HashMap::new();

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
                        let entry = partial_tools.entry(index).or_insert((None, None, String::new()));
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

            if tool_calls.is_empty() {
                // No tools requested - final response reached
                session.add_assistant_message(&full_content);
                let _ = event_tx.send(AgentEvent::Finished { usage: None });
                let _ = session.save();
                return Ok(full_content);
            }

            // Record assistant message with tool calls
            session.add_assistant_with_tools(&full_content, tool_calls.clone());

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

                let start = Instant::now();
                let result = self.tools.execute(&tc.name, parsed_args, &self.tool_ctx).await;
                let duration = start.elapsed();

                match result {
                    Ok(output) => {
                        let _ = event_tx.send(AgentEvent::ToolFinished {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            success: true,
                            output: output.clone(),
                            duration,
                        });
                        session.add_tool_result(&tc.id, output);
                    }
                    Err(err) => {
                        let err_msg = format!("Tool '{}' error: {}", tc.name, err);
                        let _ = event_tx.send(AgentEvent::ToolFinished {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            success: false,
                            output: err_msg.clone(),
                            duration,
                        });
                        session.add_tool_result(&tc.id, err_msg);
                    }
                }
            }
        }

        anyhow::bail!(
            "Agent reached maximum execution turns ({}) without completing",
            self.max_turns
        )
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
}
