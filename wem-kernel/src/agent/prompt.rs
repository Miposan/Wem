//! Prompt Assembly — System Prompt 模块化拼接

use crate::agent::tools::ToolRegistry;

pub struct PromptAssembly;

impl PromptAssembly {
    pub fn new() -> Self {
        Self
    }

    pub fn build(
        &self,
        allowed_tools: &[String],
        registry: &ToolRegistry,
        working_dir: &std::path::Path,
    ) -> String {
        let mut parts = Vec::new();

        // 1. 基础角色指令
        parts.push(format!(
            "You are Wem Agent, an AI assistant for knowledge management. \
             You help users manage documents, blocks, and files.\n\
             Current working directory: {}\n\
             Current time: {}",
            working_dir.display(),
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        ));

        // 2. 工具使用指南
        let tool_prompts = registry.tool_prompts(allowed_tools);
        if !tool_prompts.is_empty() {
            let mut tool_section = "## Available Tools\n".to_string();
            for (name, prompt) in &tool_prompts {
                tool_section.push_str(&format!("\n### {}\n{}\n", name, prompt));
            }
            parts.push(tool_section);
        }

        // 3. 行为规则
        parts.push(
            "## Rules\n\
             - Always use tools when you need to perform actions\n\
             - Provide concise, helpful responses\n\
             - If a tool fails, explain the error and suggest alternatives"
                .to_string(),
        );

        parts.join("\n\n")
    }
}
