//! Provider — LLM 接入层
//!
//! 定义统一的 Provider trait 和跨 Provider 共享的类型。

pub mod anthropic;
pub mod openai_compatible;

use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};

// ─── 流式事件契约 ──────────────────────────────────────────────

/// 所有 Provider 统一输出的流式事件枚举
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    TextDelta { text: String },
    ToolCallBegin { id: String, name: String },
    ToolCallDelta { id: String, args_json: String },
    ToolCallEnd { id: String },
    Done { usage: TokenUsage },
}

/// Token 用量统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

// ─── 消息模型 ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// 消息中的内容块（文本、工具调用、工具结果）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

/// 一条消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    pub fn assistant_tool_calls(calls: Vec<ContentBlock>) -> Self {
        Self {
            role: Role::Assistant,
            content: calls,
        }
    }

    pub fn tool_result(tool_use_id: String, content: String, is_error: bool) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            }],
        }
    }

    pub fn text_content(&self) -> Option<&str> {
        self.content.iter().find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
    }
}

// ─── 工具定义（发给 LLM 的格式）──────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

// ─── Provider trait ────────────────────────────────────────────

pub type StreamResult = Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("API error (status {status}): {message}")]
    Api { status: u16, message: String },
    #[error("Rate limited, retry after {retry_after_ms:?}ms")]
    RateLimited { retry_after_ms: Option<u64> },
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Stream parse error: {0}")]
    StreamParse(String),
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// 流式调用 LLM
    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDef],
        temperature: f32,
    ) -> Result<StreamResult, ProviderError>;

    /// 非流式调用（Context Manager 摘要用）
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        temperature: f32,
    ) -> Result<String, ProviderError>;
}
