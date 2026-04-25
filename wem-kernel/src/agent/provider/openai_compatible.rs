//! OpenAI-Compatible Provider — 基于 async-openai crate

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestAssistantMessageContent,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent, ChatCompletionRequestToolMessage,
    ChatCompletionRequestToolMessageContent, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, ChatCompletionTools, ChatCompletionTool,
    CreateChatCompletionRequest, FinishReason, FunctionCall,
};
use async_openai::types::chat::FunctionObject;
use async_trait::async_trait;
use futures::StreamExt;

use super::{Message, Provider, ProviderError, StreamEvent, StreamResult, TokenUsage, ToolDef};

pub struct OpenAICompatibleProvider {
    client: async_openai::Client<OpenAIConfig>,
    model: String,
    max_tokens: u32,
}

impl OpenAICompatibleProvider {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        Self::with_headers(api_key, base_url, model, std::collections::HashMap::new())
    }

    pub fn with_headers(
        api_key: String,
        base_url: String,
        model: String,
        custom_headers: std::collections::HashMap<String, String>,
    ) -> Self {
        let mut config = OpenAIConfig::new();
        if !api_key.is_empty() {
            config = config.with_api_key(&api_key);
        }
        config = config.with_api_base(&base_url);

        let http_client = if custom_headers.is_empty() {
            reqwest::Client::new()
        } else {
            let mut headers = reqwest::header::HeaderMap::new();
            for (k, v) in &custom_headers {
                if let (Ok(name), Ok(value)) = (
                    reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                    reqwest::header::HeaderValue::from_str(v),
                ) {
                    headers.insert(name, value);
                }
            }
            reqwest::Client::builder()
                .default_headers(headers)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        };

        Self {
            client: async_openai::Client::with_config(config).with_http_client(http_client),
            model,
            max_tokens: 16384,
        }
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    fn convert_messages(messages: &[Message]) -> Vec<ChatCompletionRequestMessage> {
        let mut result = Vec::new();
        for msg in messages {
            match msg.role {
                super::Role::System => {
                    let text = msg.text_content().unwrap_or("").to_string();
                    result.push(ChatCompletionRequestMessage::System(
                        ChatCompletionRequestSystemMessage {
                            content: ChatCompletionRequestSystemMessageContent::Text(text),
                            ..Default::default()
                        },
                    ));
                }
                super::Role::User => {
                    for block in &msg.content {
                        match block {
                            super::ContentBlock::Text { text } => {
                                result.push(ChatCompletionRequestMessage::User(
                                    ChatCompletionRequestUserMessage {
                                        content: ChatCompletionRequestUserMessageContent::Text(
                                            text.clone(),
                                        ),
                                        ..Default::default()
                                    },
                                ));
                            }
                            super::ContentBlock::ToolResult {
                                tool_use_id, content, ..
                            } => {
                                result.push(ChatCompletionRequestMessage::Tool(
                                    ChatCompletionRequestToolMessage {
                                        content: ChatCompletionRequestToolMessageContent::Text(
                                            content.clone(),
                                        ),
                                        tool_call_id: tool_use_id.clone(),
                                    },
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                super::Role::Assistant => {
                    let has_tool_use = msg
                        .content
                        .iter()
                        .any(|b| matches!(b, super::ContentBlock::ToolUse { .. }));
                    if has_tool_use {
                        let mut text_parts = Vec::new();
                        let mut tool_calls = Vec::new();
                        for block in &msg.content {
                            match block {
                                super::ContentBlock::Text { text } => text_parts.push(text.clone()),
                                super::ContentBlock::ToolUse { id, name, input } => {
                                    tool_calls.push(ChatCompletionMessageToolCalls::Function(
                                        ChatCompletionMessageToolCall {
                                            id: id.clone(),
                                            function: FunctionCall {
                                                name: name.clone(),
                                                arguments: input.to_string(),
                                            },
                                        },
                                    ));
                                }
                                _ => {}
                            }
                        }
                        result.push(ChatCompletionRequestMessage::Assistant(
                            ChatCompletionRequestAssistantMessage {
                                content: if text_parts.is_empty() {
                                    None
                                } else {
                                    Some(ChatCompletionRequestAssistantMessageContent::Text(
                                        text_parts.join(""),
                                    ))
                                },
                                tool_calls: if tool_calls.is_empty() {
                                    None
                                } else {
                                    Some(tool_calls)
                                },
                                ..Default::default()
                            },
                        ));
                    } else if let Some(text) = msg.text_content() {
                        result.push(ChatCompletionRequestMessage::Assistant(
                            ChatCompletionRequestAssistantMessage {
                                content: Some(ChatCompletionRequestAssistantMessageContent::Text(
                                    text.to_string(),
                                )),
                                ..Default::default()
                            },
                        ));
                    }
                }
            }
        }
        result
    }

    fn convert_tools(tools: &[ToolDef]) -> Vec<ChatCompletionTools> {
        tools
            .iter()
            .map(|t| {
                ChatCompletionTools::Function(ChatCompletionTool {
                    function: FunctionObject {
                        name: t.name.clone(),
                        description: Some(t.description.clone()),
                        parameters: Some(t.input_schema.clone()),
                        ..Default::default()
                    },
                })
            })
            .collect()
    }
}

#[async_trait]
impl Provider for OpenAICompatibleProvider {
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

        let mut chat_messages = Self::convert_messages(messages);
        chat_messages.insert(
            0,
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(system.to_string()),
                ..Default::default()
            }),
        );

        let request = CreateChatCompletionRequest {
            model: selected_model.to_string(),
            messages: chat_messages,
            temperature: if temperature > 0.0 { Some(temperature) } else { None },
            max_completion_tokens: Some(self.max_tokens),
            stream: Some(true),
            tools: if tools.is_empty() {
                None
            } else {
                Some(Self::convert_tools(tools))
            },
            ..Default::default()
        };

        let stream = self
            .client
            .chat()
            .create_stream(request)
            .await
            .map_err(|e| match e {
                async_openai::error::OpenAIError::ApiError(api_err) => ProviderError::Api {
                    status: 0,
                    message: api_err.message,
                },
                other => ProviderError::Network(other.to_string()),
            })?;

        let mapped = stream.flat_map(|result| {
            let events: Vec<Result<StreamEvent, ProviderError>> = match result {
                Ok(response) => {
                    let mut events = Vec::new();
                    for choice in &response.choices {
                        let delta = &choice.delta;
                        if let Some(ref content) = delta.content {
                            if !content.is_empty() {
                                events.push(StreamEvent::TextDelta {
                                    text: content.clone(),
                                });
                            }
                        }
                        if let Some(ref tool_calls) = delta.tool_calls {
                            for tc in tool_calls {
                                if let Some(ref id) = tc.id {
                                    if let Some(ref func) = tc.function {
                                        if let Some(ref name) = func.name {
                                            events.push(StreamEvent::ToolCallBegin {
                                                id: id.clone(),
                                                name: name.clone(),
                                            });
                                        }
                                    }
                                }
                                if let Some(ref func) = tc.function {
                                    if let Some(ref args) = func.arguments {
                                        if !args.is_empty() {
                                            events.push(StreamEvent::ToolCallDelta {
                                                id: String::new(),
                                                args_json: args.clone(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        if choice.finish_reason == Some(FinishReason::ToolCalls) {
                            events.push(StreamEvent::ToolCallEnd { id: String::new() });
                        }
                    }
                    if events.is_empty() {
                        if response
                            .choices
                            .first()
                            .and_then(|c| c.finish_reason)
                            == Some(FinishReason::Stop)
                        {
                            vec![Ok(StreamEvent::Done {
                                usage: TokenUsage {
                                    input_tokens: 0,
                                    output_tokens: 0,
                                },
                            })]
                        } else {
                            vec![]
                        }
                    } else {
                        events.into_iter().map(Ok).collect()
                    }
                }
                Err(e) => vec![Err(ProviderError::StreamParse(e.to_string()))],
            };
            futures::stream::iter(events)
        });

        Ok(Box::pin(mapped))
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

        let mut chat_messages = Self::convert_messages(messages);
        chat_messages.insert(
            0,
            ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                content: ChatCompletionRequestSystemMessageContent::Text(system.to_string()),
                ..Default::default()
            }),
        );

        let request = CreateChatCompletionRequest {
            model: selected_model.to_string(),
            messages: chat_messages,
            temperature: if temperature > 0.0 { Some(temperature) } else { None },
            max_completion_tokens: Some(self.max_tokens),
            stream: Some(false),
            ..Default::default()
        };

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| match e {
                async_openai::error::OpenAIError::ApiError(api_err) => ProviderError::Api {
                    status: 0,
                    message: api_err.message,
                },
                other => ProviderError::Network(other.to_string()),
            })?;

        Ok(response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default())
    }
}
