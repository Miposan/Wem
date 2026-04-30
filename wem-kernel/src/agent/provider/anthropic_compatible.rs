//! Anthropic Provider — 基于 anthropic-ai-sdk 类型 + reqwest 流式传输
//!
//! 使用 anthropic-ai-sdk 的请求/响应类型定义，
//! HTTP 层自行管理以避免 SDK streaming 的生命周期限制。

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde::Serialize;

use anthropic_ai_sdk::types::message::{
    ContentBlockDelta, StreamEvent as SdkStreamEvent,
};

use super::{Message, Provider, ProviderError, StreamEvent, StreamResult, TokenUsage, ToolDef};

// ─── Anthropic API 请求结构 ───────────────────────────────────────

#[derive(Debug, Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

// ─── AnthropicProvider ─────────────────────────────────────────

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: "https://api.anthropic.com/v1".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 16384,
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    pub fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }

    fn convert_messages(messages: &[Message]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .filter(|m| m.role != super::Role::System)
            .map(|m| {
                let role = match m.role {
                    super::Role::User | super::Role::System => "user",
                    super::Role::Assistant => "assistant",
                };
                let content = match m.content.len() {
                    0 => serde_json::Value::String(String::new()),
                    1 => match &m.content[0] {
                        super::ContentBlock::Text { text } => serde_json::Value::String(text.clone()),
                        super::ContentBlock::ToolUse { id, name, input } => serde_json::json!([{
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input,
                        }]),
                        super::ContentBlock::ToolResult { tool_use_id, content, is_error } => serde_json::json!([{
                            "type": "tool_result",
                            "tool_use_id": tool_use_id,
                            "content": content,
                            "is_error": is_error,
                        }]),
                    },
                    _ => {
                        let blocks: Vec<serde_json::Value> = m.content.iter().map(|b| match b {
                            super::ContentBlock::Text { text } => serde_json::json!({
                                "type": "text",
                                "text": text,
                            }),
                            super::ContentBlock::ToolUse { id, name, input } => serde_json::json!({
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": input,
                            }),
                            super::ContentBlock::ToolResult { tool_use_id, content, is_error } => serde_json::json!({
                                "type": "tool_result",
                                "tool_use_id": tool_use_id,
                                "content": content,
                                "is_error": is_error,
                            }),
                        }).collect();
                        serde_json::Value::Array(blocks)
                    }
                };
                serde_json::json!({ "role": role, "content": content })
            })
            .collect()
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDef],
        temperature: f32,
        model: Option<&str>,
    ) -> Result<StreamResult, ProviderError> {
        let selected_model = model
            .filter(|m| !m.trim().is_empty())
            .unwrap_or(&self.model);

        let sdk_tools: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            }))
            .collect();

        let body = MessagesRequest {
            model: selected_model.to_string(),
            max_tokens: self.max_tokens,
            system: system.to_string(),
            messages: Self::convert_messages(messages),
            tools: sdk_tools,
            temperature: if temperature != 0.0 { Some(temperature) } else { None },
            stream: true,
        };

        let resp = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            if status_code == 429 {
                let retry_ms = body_text.contains("rate_limit").then(|| 5000u64);
                return Err(ProviderError::RateLimited { retry_after_ms: retry_ms });
            }
            return Err(ProviderError::Api {
                status: status_code,
                message: body_text,
            });
        }

        // bytes stream → SSE → SDK StreamEvent → our StreamEvent
        let stream = resp
            .bytes_stream()
            .eventsource()
            .map(move |result| {
                match result {
                    Ok(event) => {
                        if event.data == "[DONE]" {
                            return Ok(StreamEvent::Done {
                                usage: TokenUsage {
                                    input_tokens: 0,
                                    output_tokens: 0,
                                },
                            });
                        }
                        match serde_json::from_str::<SdkStreamEvent>(&event.data) {
                            Ok(evt) => match evt {
                                SdkStreamEvent::ContentBlockStart { content_block, .. } => match content_block {
                                    anthropic_ai_sdk::types::message::ContentBlock::Text { .. } => {
                                        Ok(StreamEvent::TextDelta { text: String::new() })
                                    }
                                    anthropic_ai_sdk::types::message::ContentBlock::ToolUse { id, name, .. } => {
                                        Ok(StreamEvent::ToolCallBegin { id, name })
                                    }
                                    _ => Ok(StreamEvent::TextDelta { text: String::new() }),
                                },
                                SdkStreamEvent::ContentBlockDelta { delta, .. } => match delta {
                                    ContentBlockDelta::TextDelta { text } => {
                                        Ok(StreamEvent::TextDelta { text })
                                    }
                                    ContentBlockDelta::InputJsonDelta { partial_json } => {
                                        Ok(StreamEvent::ToolCallDelta {
                                            id: String::new(),
                                            args_json: partial_json,
                                        })
                                    }
                                    _ => Ok(StreamEvent::TextDelta { text: String::new() }),
                                },
                                SdkStreamEvent::ContentBlockStop { .. } => {
                                    Ok(StreamEvent::ToolCallEnd { id: String::new() })
                                }
                                SdkStreamEvent::MessageDelta { usage, .. } => Ok(StreamEvent::Done {
                                    usage: TokenUsage {
                                        input_tokens: 0,
                                        output_tokens: usage.map(|u| u.output_tokens).unwrap_or(0),
                                    },
                                }),
                                SdkStreamEvent::MessageStart { .. } | SdkStreamEvent::Ping => {
                                    Ok(StreamEvent::TextDelta { text: String::new() })
                                }
                                SdkStreamEvent::MessageStop => Ok(StreamEvent::Done {
                                    usage: TokenUsage {
                                        input_tokens: 0,
                                        output_tokens: 0,
                                    },
                                }),
                                SdkStreamEvent::Error { error } => Err(ProviderError::Api {
                                    status: 500,
                                    message: format!("{}: {}", error.type_, error.message),
                                }),
                            },
                            Err(e) => Err(ProviderError::StreamParse(format!(
                                "Failed to parse SSE data: {} — data: {}",
                                e,
                                &event.data[..event.data.len().min(200)]
                            ))),
                        }
                    }
                    Err(e) => Err(ProviderError::StreamParse(e.to_string())),
                }
            });

        Ok(Box::pin(stream))
    }

    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        temperature: f32,
        model: Option<&str>,
    ) -> Result<String, ProviderError> {
        let selected_model = model
            .filter(|m| !m.trim().is_empty())
            .unwrap_or(&self.model);

        let body = MessagesRequest {
            model: selected_model.to_string(),
            max_tokens: self.max_tokens,
            system: system.to_string(),
            messages: Self::convert_messages(messages),
            tools: vec![],
            temperature: if temperature != 0.0 { Some(temperature) } else { None },
            stream: false,
        };

        let resp = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let status_code = status.as_u16();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api {
                status: status_code,
                message: body_text,
            });
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Serialization(e.to_string()))?;

        Ok(json["content"]
            .as_array()
            .and_then(|arr| {
                arr.iter().find_map(|block| {
                    if block["type"] == "text" {
                        block["text"].as_str()
                    } else {
                        None
                    }
                })
            })
            .unwrap_or("")
            .to_string())
    }
}
