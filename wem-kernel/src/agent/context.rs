//! Context Manager — 上下文窗口管理
//!
//! 估算 token 数、必要时压缩消息历史。

use crate::agent::provider::{Message, Provider};

pub struct ContextManager {
    max_tokens: u32,
}

impl ContextManager {
    pub fn new(max_tokens: u32) -> Self {
        Self { max_tokens }
    }

    /// 粗估消息列表的 token 数
    pub fn estimate_tokens(messages: &[Message]) -> u32 {
        let mut total = 0u32;
        for msg in messages {
            for block in &msg.content {
                match block {
                    crate::agent::provider::ContentBlock::Text { text } => {
                        total += estimate_text_tokens(text);
                    }
                    crate::agent::provider::ContentBlock::ToolUse { name, input, .. } => {
                        total += estimate_text_tokens(name);
                        total += estimate_text_tokens(&input.to_string());
                    }
                    crate::agent::provider::ContentBlock::ToolResult { content, .. } => {
                        total += estimate_text_tokens(content);
                    }
                }
            }
        }
        total
    }

    pub fn needs_compression(&self, messages: &[Message], system_tokens: u32) -> bool {
        let msg_tokens = Self::estimate_tokens(messages);
        let total = msg_tokens + system_tokens;
        total > (self.max_tokens as f32 * 0.9) as u32
    }

    pub async fn compress(
        &self,
        messages: &mut Vec<Message>,
        system: &str,
        provider: &dyn Provider,
        model: Option<&str>,
    ) -> Result<(), String> {
        if messages.len() <= 4 {
            return Ok(());
        }

        // 保留最近 4 条消息，对中间部分做摘要
        let split_point = messages.len().saturating_sub(4);
        let old_messages: Vec<Message> = messages.drain(..split_point).collect();

        let summary_text = old_messages
            .iter()
            .filter_map(|m| m.text_content())
            .collect::<Vec<_>>()
            .join("\n");

        let summary_prompt = vec![Message::user(format!(
            "Summarize the following conversation in a concise paragraph. \
             Keep key facts, decisions, and results.\n\n{}",
            summary_text
        ))];

        // 摘要也走当前会话模型（如果有覆盖），避免上下文压缩和主对话模型不一致。
        match provider
            .complete(system, &summary_prompt, 0.3, model)
            .await
        {
            Ok(summary) => {
                // 用一条“摘要消息”替代旧历史，给最近对话腾出上下文窗口。
                let summary_msg = Message::user(format!("[Summary of earlier conversation: {}]", summary));
                messages.insert(0, summary_msg);
            }
            Err(_) => {
                // 摘要失败，直接丢弃最早的消息（保留的已经在 messages 里了）
            }
        }

        Ok(())
    }
}

fn estimate_text_tokens(text: &str) -> u32 {
    if text.is_empty() {
        return 0;
    }
    let total_chars = text.chars().count() as f32;
    let ascii_count = text.chars().filter(|c| c.is_ascii()).count() as f32;
    let ascii_ratio = ascii_count / total_chars;
    if ascii_ratio > 0.8 {
        (text.len() as f32 / 4.0) as u32
    } else {
        (text.len() as f32 / 1.5) as u32
    }
}
