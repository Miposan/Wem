//! Agentic Loop — 编排核心
//!
//! 驱动 "LLM 思考 → 工具调用 → 结果反馈 → 再思考" 的循环。

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::Mutex;

use crate::agent::context::ContextManager;
use crate::agent::permission::PermissionGate;
use crate::agent::prompt::PromptAssembly;
use crate::agent::provider::{ContentBlock, Message, Provider, StreamEvent};
use crate::agent::session::{AgentEvent, Session};
use crate::agent::tools::{ToolContext, ToolRegistry};

pub struct AgentLoop {
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    permission: Arc<Mutex<PermissionGate>>,
    prompt_assembly: PromptAssembly,
    context_manager: ContextManager,
    event_tx: tokio::sync::broadcast::Sender<AgentEvent>,
    cancel: tokio_util::sync::CancellationToken,
}

struct PendingToolCall {
    id: String,
    name: String,
    args_json: String,
}

impl AgentLoop {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Arc<ToolRegistry>,
        event_tx: tokio::sync::broadcast::Sender<AgentEvent>,
        cancel: tokio_util::sync::CancellationToken,
        max_context_tokens: u32,
    ) -> Self {
        Self {
            provider,
            tools,
            permission: Arc::new(Mutex::new(PermissionGate::new())),
            prompt_assembly: PromptAssembly::new(),
            context_manager: ContextManager::new(max_context_tokens),
            event_tx,
            cancel,
        }
    }

    pub async fn run(&self, session: &Arc<Mutex<Session>>, user_msg: String) -> Result<(), String> {
        {
            let mut s = session.lock().await;
            s.push_message(Message::user(user_msg));
            s.set_state(crate::agent::session::SessionState::Running);
        }

        let max_steps = {
            let s = session.lock().await;
            s.config.max_steps
        };

        for step in 0..max_steps {
            if self.cancel.is_cancelled() {
                let mut s = session.lock().await;
                s.set_state(crate::agent::session::SessionState::Idle);
                return Ok(());
            }

            // 1. 读取 session 配置和消息（clone 出来，避免跨 await 持锁）
            let (system, tool_defs, allowed_tools, working_dir, mut messages, temperature) = {
                let s = session.lock().await;
                let system = self.prompt_assembly.build(
                    &s.config.allowed_tools,
                    &self.tools,
                    &s.config.working_dir,
                );
                let tool_defs = self.tools.tool_defs(&s.config.allowed_tools);
                let temperature = s.config.temperature;
                let messages = s.messages.clone();
                (
                    system,
                    tool_defs,
                    s.config.allowed_tools.clone(),
                    s.config.working_dir.clone(),
                    messages,
                    temperature,
                )
            };

            // 2. 检查上下文压缩
            let sys_tokens = ContextManager::estimate_tokens(&[Message::assistant(&system)]);
            if self.context_manager.needs_compression(&messages, sys_tokens) {
                let _ = self.context_manager.compress(
                    &mut messages, &system, self.provider.as_ref()
                ).await;
                let mut s = session.lock().await;
                s.messages = messages.clone();
            }

            let _ = self.event_tx.send(AgentEvent::StepProgress { step, max_steps });

            // 3. 调用 Provider
            let mut stream = self.provider
                .stream(&system, &messages, &tool_defs, temperature)
                .await
                .map_err(|e| e.to_string())?;

            // 4. 消费流式事件
            let mut text_response = String::new();
            let mut tool_calls: Vec<PendingToolCall> = Vec::new();
            let mut current_tool_call: Option<PendingToolCall> = None;

            while let Some(event_result) = stream.next().await {
                match event_result {
                    Ok(event) => match event {
                        StreamEvent::TextDelta { text } => {
                            if !text.is_empty() {
                                text_response.push_str(&text);
                                let _ = self.event_tx.send(AgentEvent::TextDelta { text });
                            }
                        }
                        StreamEvent::ToolCallBegin { id, name } => {
                            current_tool_call = Some(PendingToolCall {
                                id,
                                name,
                                args_json: String::new(),
                            });
                        }
                        StreamEvent::ToolCallDelta { args_json, .. } => {
                            if let Some(tc) = &mut current_tool_call {
                                tc.args_json.push_str(&args_json);
                            }
                        }
                        StreamEvent::ToolCallEnd { .. } => {
                            if let Some(tc) = current_tool_call.take() {
                                tool_calls.push(tc);
                            }
                        }
                        StreamEvent::Done { .. } => {
                            break;
                        }
                    },
                    Err(e) => {
                        let message = e.to_string();
                        {
                            let mut s = session.lock().await;
                            s.set_state(crate::agent::session::SessionState::Error);
                        }
                        let _ = self.event_tx.send(AgentEvent::Error {
                            message: message.clone(),
                        });
                        return Err(message);
                    }
                }
            }

            // 5. 将 assistant 回复写入消息历史
            if !text_response.is_empty() || !tool_calls.is_empty() {
                let mut content_blocks = Vec::new();
                if !text_response.is_empty() {
                    content_blocks.push(ContentBlock::Text { text: text_response });
                }
                for tc in &tool_calls {
                    let args_val: serde_json::Value = serde_json::from_str(&tc.args_json)
                        .unwrap_or(serde_json::Value::Null);
                    content_blocks.push(ContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: args_val,
                    });
                }
                let mut s = session.lock().await;
                s.push_message(Message {
                    role: crate::agent::provider::Role::Assistant,
                    content: content_blocks,
                });
            }

            // 6. 无工具调用 → 完成
            if tool_calls.is_empty() {
                let mut s = session.lock().await;
                s.set_state(crate::agent::session::SessionState::Idle);
                let _ = self.event_tx.send(AgentEvent::Done);
                return Ok(());
            }

            // 7. 执行工具
            let results = self.execute_tools(&tool_calls, &allowed_tools, &working_dir).await;

            // 8. 工具结果写入消息历史
            {
                let mut s = session.lock().await;
                for (tc_id, result) in results {
                    s.push_message(Message::tool_result(
                        tc_id.clone(),
                        result.content.clone(),
                        result.is_error,
                    ));
                    let summary = if result.is_error {
                        format!("Error: {}", &result.content[..result.content.len().min(200)])
                    } else {
                        result.content[..result.content.len().min(200)].to_string()
                    };
                    let _ = self.event_tx.send(AgentEvent::ToolCallEnd {
                        id: tc_id,
                        result_summary: summary,
                    });
                }
            }
        }

        let mut s = session.lock().await;
        s.push_message(Message::assistant("Reached max steps limit."));
        s.set_state(crate::agent::session::SessionState::Idle);
        let _ = self.event_tx.send(AgentEvent::Done);
        Ok(())
    }

    async fn execute_tools(
        &self,
        tool_calls: &[PendingToolCall],
        _allowed_tools: &[String],
        working_dir: &std::path::Path,
    ) -> Vec<(String, crate::agent::tools::ToolResult)> {
        let mut results = Vec::new();

        for tc in tool_calls {
            let args_val: serde_json::Value = serde_json::from_str(&tc.args_json)
                .unwrap_or(serde_json::Value::Null);

            let _ = self.event_tx.send(AgentEvent::ToolCallBegin {
                id: tc.id.clone(),
                name: tc.name.clone(),
                args: args_val.clone(),
            });

            // 权限检查
            let permission_result = {
                let mut gate = self.permission.lock().await;
                gate.check_with_cache(&tc.name, &args_val)
            };

            match permission_result {
                crate::agent::permission::Permission::Auto => {
                    let result = match self.tools.get(&tc.name) {
                        Some(tool) => {
                            let ctx = ToolContext {
                                working_dir: working_dir.to_path_buf(),
                                session_id: String::new(),
                            };
                            tool.execute(args_val, &ctx).await
                        }
                        None => crate::agent::tools::ToolResult::error(
                            format!("Unknown tool: {}", tc.name)
                        ),
                    };
                    results.push((tc.id.clone(), result));
                }
                crate::agent::permission::Permission::Deny => {
                    results.push((tc.id.clone(), crate::agent::tools::ToolResult::error(
                        format!("Permission denied for tool: {}", tc.name)
                    )));
                }
                crate::agent::permission::Permission::Ask => {
                    results.push((tc.id.clone(), crate::agent::tools::ToolResult::error(
                        format!("Tool '{}' requires approval (not yet implemented in CLI mode)", tc.name)
                    )));
                }
            }
        }

        results
    }
}
