use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::provider::types::ToolDefinition;

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            env: std::env::vars().collect(),
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> anyhow::Result<String>;
}

pub type DynTool = Arc<dyn Tool>;

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, DynTool>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: DynTool) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<DynTool> {
        self.tools.get(name).cloned().or_else(|| match name {
            "read_file" => self.tools.get("read").cloned(),
            "write_file" => self.tools.get("write").cloned(),
            "edit_file" => self.tools.get("edit").cloned(),
            "read" => self.tools.get("read_file").cloned(),
            "write" => self.tools.get("write_file").cloned(),
            "edit" => self.tools.get("edit_file").cloned(),
            _ => None,
        })
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    pub async fn execute(&self, name: &str, args: Value, ctx: &ToolContext) -> anyhow::Result<String> {
        let tool = self.get(name).ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;
        tool.execute(args, ctx).await
    }
}
