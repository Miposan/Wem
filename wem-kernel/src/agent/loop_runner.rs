//! Agentic Loop — 编排核心
//!
//! 驱动 "LLM 思考 → 工具调用 → 结果反馈 → 再思考" 的循环。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::Mutex;

use crate::agent::context::ContextManager;
use crate::agent::permission::PermissionGate;
use crate::agent::prompt::PromptAssembly;
use crate::agent::provider::{ContentBlock, Message, Provider, StreamEvent, ToolDef};
use crate::agent::session::{AgentEvent, PendingApproval, Phase, Session, SessionState};
use crate::agent::tools::{ToolContext, ToolRegistry, ToolResult};

pub struct AgentLoop {
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    permission: Arc<Mutex<PermissionGate>>,
    prompt_assembly: PromptAssembly,
    context_manager: ContextManager,
    event_tx: tokio::sync::broadcast::Sender<AgentEvent>,
    cancel: tokio_util::sync::CancellationToken,
    persist_fn: Arc<dyn Fn(&str, &[Message]) + Send + Sync>,
}

struct PendingToolCall {
    id: String,
    name: String,
    args_json: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopPhase {
    Initializing,
    PreparingTurn,
    StreamingModel,
    ExecutingTools,
    Completed,
    Cancelled,
    Failed,
}

impl LoopPhase {
    fn can_transition_to(self, next: LoopPhase) -> bool {
        use LoopPhase::*;

        if self == next {
            return true;
        }

        matches!(
            (self, next),
            (Initializing, PreparingTurn)
                | (Initializing, Cancelled)
                | (Initializing, Failed)
                | (PreparingTurn, StreamingModel)
                | (PreparingTurn, Completed)
                | (PreparingTurn, Cancelled)
                | (PreparingTurn, Failed)
                | (StreamingModel, ExecutingTools)
                | (StreamingModel, Completed)
                | (StreamingModel, Cancelled)
                | (StreamingModel, Failed)
                | (ExecutingTools, PreparingTurn)
                | (ExecutingTools, Cancelled)
                | (ExecutingTools, Failed)
        )
    }
}

/// Agent 循环的结构化错误类型
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Invalid loop phase transition: {0}")]
    InvalidPhaseTransition(String),

    #[error("Provider request failed: {0}")]
    ProviderError(String),

    #[error("Stream processing failed: {0}")]
    StreamError(String),
}

struct TurnInput {
    model: String,
    system: String,
    tool_defs: Vec<ToolDef>,
    allowed_tools: Vec<String>,
    working_dir: std::path::PathBuf,
    messages: Vec<Message>,
    temperature: f32,
}

struct ModelOutput {
    text_response: String,
    tool_calls: Vec<PendingToolCall>,
}

impl AgentLoop {
    pub fn new(
        provider: Arc<dyn Provider>,
        tools: Arc<ToolRegistry>,
        event_tx: tokio::sync::broadcast::Sender<AgentEvent>,
        cancel: tokio_util::sync::CancellationToken,
        max_context_tokens: u32,
        persist_fn: Arc<dyn Fn(&str, &[Message]) + Send + Sync>,
    ) -> Self {
        Self {
            provider,
            tools,
            permission: Arc::new(Mutex::new(PermissionGate::new())),
            prompt_assembly: PromptAssembly::new(),
            context_manager: ContextManager::new(max_context_tokens),
            event_tx,
            cancel,
            persist_fn,
        }
    }

    /// 推进状态机阶段，校验迁移合法性并通过事件流广播当前阶段。
    fn advance_phase(
        &self,
        current: &mut LoopPhase,
        next: LoopPhase,
    ) -> Result<(), AgentError> {
        if current.can_transition_to(next) {
            let _ = self.event_tx.send(AgentEvent::PhaseChanged {
                phase: Phase::from_loop_phase(next),
            });
            *current = next;
            Ok(())
        } else {
            Err(AgentError::InvalidPhaseTransition(format!(
                "{:?} -> {:?}",
                *current, next
            )))
        }
    }

    pub async fn run(&self, session: &Arc<Mutex<Session>>, user_msg: String) -> Result<(), AgentError> {
        let mut phase = LoopPhase::Initializing;

        let (session_id, max_steps) = {
            let mut s = session.lock().await;
            s.push_message(Message::user(user_msg));
            s.set_state(SessionState::Running);
            (s.id.clone(), s.config.max_steps)
        };
        self.persist(&session_id, session).await;

        self.advance_phase(&mut phase, LoopPhase::PreparingTurn)?;

        // 每轮循环 = 一次“思考 + （可选）工具执行”。
        for step in 0..max_steps {
            if self.cancel.is_cancelled() {
                self.advance_phase(&mut phase, LoopPhase::Cancelled)?;
                self.finish_completed(session).await;
                return Ok(());
            }

            let turn = self.prepare_turn(session).await;

            let _ = self.event_tx.send(AgentEvent::StepProgress { step, max_steps });

            // 先让模型回答（可能包含 tool call）。
            self.advance_phase(&mut phase, LoopPhase::StreamingModel)?;
            let output = match self.stream_model_response(&turn).await {
                Ok(output) => output,
                Err(e) => {
                    self.advance_phase(&mut phase, LoopPhase::Failed)?;
                    return Err(self.fail_session(session, e).await);
                }
            };

            self.persist_assistant_message(session, &output).await;

            if output.tool_calls.is_empty() {
                self.persist(&session_id, session).await;
                self.advance_phase(&mut phase, LoopPhase::Completed)?;
                self.finish_completed(session).await;
                return Ok(());
            }

            // 模型请求了工具：执行后把结果喂回消息历史，再进入下一轮思考。
            self.advance_phase(&mut phase, LoopPhase::ExecutingTools)?;
            let results = self
                .execute_tools(
                    session,
                    &output.tool_calls,
                    &turn.allowed_tools,
                    &turn.working_dir,
                    &session_id,
                )
                .await;

            self.persist_tool_results(session, results).await;
            self.persist(&session_id, session).await;
            self.advance_phase(&mut phase, LoopPhase::PreparingTurn)?;
        }

        self.advance_phase(&mut phase, LoopPhase::Completed)?;
        self.finish_max_steps(session).await;
        Ok(())
    }

    async fn prepare_turn(&self, session: &Arc<Mutex<Session>>) -> TurnInput {
        // 读取 session 配置和消息（clone 出来，避免跨 await 持锁）
        let (model, system, tool_defs, allowed_tools, working_dir, mut messages, temperature) = {
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
                s.config.model.clone(),
                system,
                tool_defs,
                s.config.allowed_tools.clone(),
                s.config.working_dir.clone(),
                messages,
                temperature,
            )
        };

        let sys_tokens = ContextManager::estimate_tokens(&[Message::assistant(&system)]);
        if self.context_manager.needs_compression(&messages, sys_tokens) {
            let _ = self
                .context_manager
                .compress(
                    &mut messages,
                    &system,
                    self.provider.as_ref(),
                    Some(model.as_str()),
                )
                .await;
            // 压缩后同步回 session，避免下轮 prepare_turn 重复压缩
            {
                let mut s = session.lock().await;
                s.messages = messages;
            }
            // 从 session 重新读出以保持一致
            messages = session.lock().await.messages.clone();
        }

        TurnInput {
            model,
            system,
            tool_defs,
            allowed_tools,
            working_dir,
            messages,
            temperature,
        }
    }

    async fn stream_model_response(&self, turn: &TurnInput) -> Result<ModelOutput, AgentError> {
        let mut stream = self
            .provider
            .stream(
                &turn.system,
                &turn.messages,
                &turn.tool_defs,
                turn.temperature,
                Some(turn.model.as_str()),
            )
            .await
            .map_err(|e| AgentError::ProviderError(e.to_string()))?;

        let mut text_response = String::new();
        let mut tool_calls: Vec<PendingToolCall> = Vec::new();
        // Provider 的工具调用参数通常按增量分片返回，需要先暂存再拼接。
        let mut current_tool_call: Option<PendingToolCall> = None;

        while let Some(event_result) = stream.next().await {
            let event = event_result.map_err(|e| AgentError::StreamError(e.to_string()))?;
            match event {
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
                StreamEvent::Done { .. } => break,
            }
        }

        Ok(ModelOutput {
            text_response,
            tool_calls,
        })
    }

    async fn persist_assistant_message(&self, session: &Arc<Mutex<Session>>, output: &ModelOutput) {
        if output.text_response.is_empty() && output.tool_calls.is_empty() {
            return;
        }

        let mut content_blocks = Vec::new();
        if !output.text_response.is_empty() {
            content_blocks.push(ContentBlock::Text {
                text: output.text_response.clone(),
            });
        }

        for tc in &output.tool_calls {
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

    async fn persist_tool_results(
        &self,
        session: &Arc<Mutex<Session>>,
        results: Vec<(String, ToolResult)>,
    ) {
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

    async fn finish_completed(&self, session: &Arc<Mutex<Session>>) {
        let mut s = session.lock().await;
        s.finish_request();
        s.set_state(SessionState::Completed);
        let _ = self.event_tx.send(AgentEvent::Done);
    }

    async fn finish_max_steps(&self, session: &Arc<Mutex<Session>>) {
        let mut s = session.lock().await;
        s.push_message(Message::assistant("Reached max steps limit."));
        s.finish_request();
        s.set_state(SessionState::Completed);
        let _ = self.event_tx.send(AgentEvent::Done);
    }

    /// 将会话消息持久化到 SQLite
    async fn persist(&self, session_id: &str, session: &Arc<Mutex<Session>>) {
        let msgs = session.lock().await.messages.clone();
        (self.persist_fn)(session_id, &msgs);
    }

    async fn fail_session(&self, session: &Arc<Mutex<Session>>, err: AgentError) -> AgentError {
        {
            let mut s = session.lock().await;
            // 失败路径也要回收 request_id，避免把请求永久卡在“进行中”。
            s.finish_request();
            s.set_state(SessionState::Error);
        }
        let _ = self.event_tx.send(AgentEvent::Error {
            message: err.to_string(),
        });
        err
    }

    async fn execute_tools(
        &self,
        session: &Arc<Mutex<Session>>,
        tool_calls: &[PendingToolCall],
        allowed_tools: &[String],
        working_dir: &std::path::Path,
        session_id: &str,
    ) -> Vec<(String, ToolResult)> {
        let mut results = Vec::new();
        let allowed_set: HashSet<&str> = allowed_tools.iter().map(String::as_str).collect();
        let restricted = !allowed_set.is_empty();

        for tc in tool_calls {
            let args_val: serde_json::Value = serde_json::from_str(&tc.args_json)
                .unwrap_or(serde_json::Value::Null);

            let _ = self.event_tx.send(AgentEvent::ToolCallBegin {
                id: tc.id.clone(),
                name: tc.name.clone(),
                args: args_val.clone(),
            });

            // 会话声明了 allowed_tools 时，先做白名单拦截。
            if restricted && !allowed_set.contains(tc.name.as_str()) {
                results.push((
                    tc.id.clone(),
                    ToolResult::error(format!(
                        "Tool '{}' is not allowed in this session",
                        tc.name
                    )),
                ));
                continue;
            }

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
                                session_id: session_id.to_string(),
                            };
                            tool.execute(args_val, &ctx).await
                        }
                        None => ToolResult::error(
                            format!("Unknown tool: {}", tc.name)
                        ),
                    };
                    results.push((tc.id.clone(), result));
                }
                crate::agent::permission::Permission::Deny => {
                    results.push((tc.id.clone(), ToolResult::error(
                        format!("Permission denied for tool: {}", tc.name)
                    )));
                }
                crate::agent::permission::Permission::Ask => {
                    // Ask 模式：
                    // 1) 在 session 里登记 pending_approval
                    // 2) 发 PermissionRequired 事件给前端
                    // 3) 异步等待 approve/deny（或超时/取消）
                    let (approval_tx, approval_rx) = tokio::sync::oneshot::channel::<bool>();
                    {
                        let mut s = session.lock().await;
                        if s.pending_approval.is_some() {
                            results.push((
                                tc.id.clone(),
                                ToolResult::error(
                                    "Another tool approval is already pending"
                                ),
                            ));
                            continue;
                        }
                        s.pending_approval = Some(PendingApproval {
                            tool_name: tc.name.clone(),
                            args: args_val.clone(),
                            tx: approval_tx,
                        });
                        s.set_state(SessionState::WaitingApproval);
                    }

                    let _ = self.event_tx.send(AgentEvent::PermissionRequired {
                        tool_name: tc.name.clone(),
                        args: args_val.clone(),
                    });

                    // 审批等待期间不占用线程：tokio 会挂起当前任务。
                    let approval_result = tokio::select! {
                        _ = self.cancel.cancelled() => {
                            Err("Session cancelled while waiting for tool approval".to_string())
                        }
                        recv = tokio::time::timeout(Duration::from_secs(120), approval_rx) => {
                            match recv {
                                Ok(Ok(approved)) => Ok(approved),
                                Ok(Err(_)) => Err("Approval response channel closed".to_string()),
                                Err(_) => Err("Tool approval timed out".to_string()),
                            }
                        }
                    };

                    {
                        let mut s = session.lock().await;
                        // 无论审批结果如何，都先清理 pending 状态。
                        s.pending_approval = None;
                        if s.state == SessionState::WaitingApproval {
                            s.set_state(SessionState::Running);
                        }
                    }

                    match approval_result {
                        Ok(true) => {
                            // 用户批准后，把 (tool,args) 写入权限缓存，后续同参可自动放行。
                            {
                                let mut gate = self.permission.lock().await;
                                gate.approve(&tc.name, &args_val);
                            }
                            let result = match self.tools.get(&tc.name) {
                                Some(tool) => {
                                    let ctx = ToolContext {
                                        working_dir: working_dir.to_path_buf(),
                                        session_id: session_id.to_string(),
                                    };
                                    tool.execute(args_val, &ctx).await
                                }
                                None => ToolResult::error(format!("Unknown tool: {}", tc.name)),
                            };
                            results.push((tc.id.clone(), result));
                        }
                        Ok(false) => {
                            results.push((
                                tc.id.clone(),
                                ToolResult::error(format!(
                                    "Permission denied for tool: {}",
                                    tc.name
                                )),
                            ));
                        }
                        Err(message) => {
                            results.push((tc.id.clone(), ToolResult::error(message)));
                        }
                    }
                }
            }
        }

        results
    }
}
