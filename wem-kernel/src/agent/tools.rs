//! Tools — Agent 能力层
//!
//! 定义 Tool trait、ToolRegistry、ToolContext，
//! 以及各工具实现。

pub mod file_ops;
pub mod shell_ops;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::agent::provider::ToolDef;

// ─── 工具执行结果 ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

// ─── 工具执行上下文 ────────────────────────────────────────────

pub struct ToolContext {
    pub working_dir: PathBuf,
    pub session_id: String,
}

// ─── Tool trait ────────────────────────────────────────────────

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;
    fn prompt(&self) -> &str;

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> ToolResult;
}

// ─── ToolRegistry ──────────────────────────────────────────────

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut reg = Self {
            tools: HashMap::new(),
        };
        // 注册内置工具
        reg.register(Box::new(file_ops::FileRead));
        reg.register(Box::new(file_ops::FileWrite));
        reg.register(Box::new(file_ops::FileEdit));
        reg.register(Box::new(shell_ops::ShellExec));
        reg
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn tool_defs(&self, allowed: &[String]) -> Vec<ToolDef> {
        let allowed_set: HashSet<&str> = allowed.iter().map(String::as_str).collect();
        let mut entries: Vec<_> = self
            .tools
            .iter()
            .filter(|(name, _)| allowed_set.is_empty() || allowed_set.contains(name.as_str()))
            .collect();
        entries.sort_unstable_by(|(name_a, _), (name_b, _)| name_a.cmp(name_b));

        entries
            .into_iter()
            .map(|(_, tool)| ToolDef {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema(),
            })
            .collect()
    }

    pub fn tool_prompts(&self, allowed: &[String]) -> Vec<(&str, &str)> {
        let allowed_set: HashSet<&str> = allowed.iter().map(String::as_str).collect();
        let mut entries: Vec<_> = self
            .tools
            .iter()
            .filter(|(name, _)| allowed_set.is_empty() || allowed_set.contains(name.as_str()))
            .collect();
        entries.sort_unstable_by(|(name_a, _), (name_b, _)| name_a.cmp(name_b));

        entries
            .into_iter()
            .map(|(_, tool)| (tool.name(), tool.prompt()))
            .collect()
    }

    pub fn all_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort_unstable();
        names
    }
}
