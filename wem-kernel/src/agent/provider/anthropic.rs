//! Anthropic Provider — Claude API 的流式实现
//!
//! 通过 Anthropic Messages API 实现 Provider trait。
//! 使用 eventsource-stream 解析 SSE 文本流，转换为 StreamEvent。

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::{
    Message, Provider, ProviderError, StreamEvent, StreamResult, TokenUsage, ToolDef,
};

// ─── Anthropic API 请求/响应结构 ───────────────────────────────

#[derive(Debug, Serialize)]
struct MessagesRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum AnthropicEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: MessageStartData },
    #[serde(rename = "content_block_start")]
    ContentBlockStart { index: usize, content_block: ContentBlockData },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { index: usize, delta: DeltaData },
    #[serde(rename = "content_block_stop")]
    ContentBlockStop { index: usize },
    #[serde(rename = "message_delta")]
    MessageDelta { usage: Option<DeltaUsage>, stop_reason: Option<String> },
    #[serde(rename = "message_stop")]
    MessageStop,
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "error")]
    Error { error: ErrorData },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MessageStartData {
    id: String,
    model: String,
    usage: Option<ApiUsage>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ApiUsage {
    input_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct DeltaUsage {
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum ContentBlockData {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String, input: serde_json::Value },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum DeltaData {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
}

#[derive(Debug, Deserialize)]
struct ErrorData {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
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
            base_url: "https://api.anthropic.com".to_string(),
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

    fn convert_messages(messages: &[Message]) -> Vec<AnthropicMessage> {
        messages
            .iter()
            .filter(|m| m.role != super::Role::System)
            .map(|m| {
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
                let role = match m.role {
                    super::Role::User | super::Role::System => "user",
                    super::Role::Assistant => "assistant",
                };
                AnthropicMessage {
                    role: role.to_string(),
                    content,
                }
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

        let anthropic_tools: Vec<AnthropicTool> = tools
            .iter()
            .map(|t| AnthropicTool {
                name: t.name.clone(),
                description: t.description.clone(),
                input_schema: t.input_schema.clone(),
            })
            .collect();

        let body = MessagesRequest {
            model: selected_model.to_string(),
            max_tokens: self.max_tokens,
            system: system.to_string(),
            messages: Self::convert_messages(messages),
            tools: anthropic_tools,
            temperature: if temperature != 0.0 { Some(temperature) } else { None },
            stream: true,
        };

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
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
                let retry_ms = body_text
                    .contains("rate_limit")
                    .then(|| 5000u64);
                return Err(ProviderError::RateLimited { retry_after_ms: retry_ms });
            }
            return Err(ProviderError::Api {
                status: status_code,
                message: body_text,
            });
        }

        // 将 byte stream → SSE event stream → StreamEvent stream
        let stream = resp
            .bytes_stream()
            .eventsource()
            .map(move |result| {
                match result {
                    Ok(event) => {
                        // Anthropic SSE data 是 JSON
                        if event.data == "[DONE]" {
                            return Ok(StreamEvent::Done {
                                usage: TokenUsage {
                                    input_tokens: 0,
                                    output_tokens: 0,
                                },
                            });
                        }
                        match serde_json::from_str::<AnthropicEvent>(&event.data) {
                            Ok(evt) => match evt {
                                AnthropicEvent::ContentBlockStart {
                                    content_block: ContentBlockData::Text { .. },
                                    ..
                                } => Ok(StreamEvent::TextDelta {
                                    text: String::new(),
                                }),
                                AnthropicEvent::ContentBlockStart {
                                    content_block: ContentBlockData::ToolUse { id, name, .. },
                                    ..
                                } => Ok(StreamEvent::ToolCallBegin { id, name }),
                                AnthropicEvent::ContentBlockDelta {
                                    delta: DeltaData::TextDelta { text },
                                    ..
                                } => Ok(StreamEvent::TextDelta { text }),
                                AnthropicEvent::ContentBlockDelta {
                                    delta: DeltaData::InputJsonDelta { partial_json },
                                    ..
                                } => Ok(StreamEvent::ToolCallDelta {
                                    id: String::new(),
                                    args_json: partial_json,
                                }),
                                AnthropicEvent::ContentBlockStop { .. } => {
                                    // 对于 tool_use block，发送 ToolCallEnd
                                    Ok(StreamEvent::ToolCallEnd { id: String::new() })
                                }
                                AnthropicEvent::MessageDelta { usage, .. } => {
                                    Ok(StreamEvent::Done {
                                        usage: TokenUsage {
                                            input_tokens: 0,
                                            output_tokens: usage
                                                .map(|u| u.output_tokens)
                                                .unwrap_or(0),
                                        },
                                    })
                                }
                                AnthropicEvent::MessageStart { .. } => {
                                    // 跳过，不产生事件
                                    Ok(StreamEvent::TextDelta { text: String::new() })
                                }
                                AnthropicEvent::MessageStop => {
                                    Ok(StreamEvent::Done {
                                        usage: TokenUsage {
                                            input_tokens: 0,
                                            output_tokens: 0,
                                        },
                                    })
                                }
                                AnthropicEvent::Ping { .. } => {
                                    Ok(StreamEvent::TextDelta { text: String::new() })
                                }
                                AnthropicEvent::Error { error } => {
                                    Err(ProviderError::Api {
                                        status: 500,
                                        message: format!("{}: {}", error.error_type, error.message),
                                    })
                                }
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
            .post(format!("{}/v1/messages", self.base_url))
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

        // 提取文本内容
        let text = json["content"]
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
            .to_string();

        Ok(text)
    }
}
