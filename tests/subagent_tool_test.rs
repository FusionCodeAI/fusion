use fusion::agent::AgentRunner;
use fusion::config::Config;
use fusion::provider::LlmClient;
use fusion::tools::{default_registry, ToolContext};
use std::path::PathBuf;

#[test]
fn test_agent_runner_registers_spawn_subagent_tools() {
    let client = LlmClient::new();
    let config = Config::default();
    let tools = default_registry();
    let tool_ctx = ToolContext {
        cwd: PathBuf::from("."),
        env: std::collections::HashMap::new(),
    };

    let runner = AgentRunner::new(client, config, tools, tool_ctx);
    let registered_tools = runner.tools();

    assert!(
        registered_tools.contains("spawn_subagent"),
        "AgentRunner must register spawn_subagent tool"
    );
    assert!(
        registered_tools.contains("spawn_subagents_batch"),
        "AgentRunner must register spawn_subagents_batch tool"
    );
    assert!(
        registered_tools.contains("lsp"),
        "AgentRunner must include native lsp tool"
    );
}
