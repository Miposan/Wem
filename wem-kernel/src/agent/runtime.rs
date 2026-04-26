//! Agent Runtime — 传输层无关的运行时内核
//!
//! 抽离会话管理、循环调度与事件订阅，供 HTTP/CLI 等接入层复用。

use std::sync::Arc;
use std::collections::HashSet;

use tokio::sync::{broadcast, Mutex};

use crate::agent::loop_runner::AgentLoop;
use crate::agent::provider::Provider;
use crate::agent::session::{AgentEvent, Session, SessionConfig, SessionManager, SessionState};
use crate::agent::tools::ToolRegistry;

pub struct AgentRuntime {
    session_manager: Arc<SessionManager>,
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    max_context_tokens: u32,
    persist_fn: Arc<dyn Fn(&str, &[crate::agent::provider::Message]) + Send + Sync>,
}

impl AgentRuntime {
    pub fn new(
        session_manager: Arc<SessionManager>,
        provider: Arc<dyn Provider>,
        tools: Arc<ToolRegistry>,
        max_context_tokens: u32,
    ) -> Self {
        let sm = session_manager.clone();
        let persist_fn: Arc<dyn Fn(&str, &[crate::agent::provider::Message]) + Send + Sync> =
            Arc::new(move |session_id: &str, messages: &[crate::agent::provider::Message]| {
                sm.save_messages(session_id, messages);
            });
        Self {
            session_manager,
            provider,
            tools,
            max_context_tokens,
            persist_fn,
        }
    }

    pub fn create_session(&self, mut config: SessionConfig) -> String {
        // 未指定 allowed_tools 时，默认启用全部已注册工具
        if config.allowed_tools.is_empty() {
            config.allowed_tools = self.tools.all_names();
        } else {
            let mut seen = HashSet::new();
            config
                .allowed_tools
                .retain(|name| seen.insert(name.clone()));
        }
        self.session_manager.create_session(config)
    }

    pub fn list_sessions(&self) -> Vec<String> {
        self.session_manager.list_sessions()
    }

    pub fn destroy_session(&self, id: &str) -> bool {
        self.session_manager.destroy_session(id)
    }

    pub fn abort_session(&self, id: &str) {
        if let Some(cancel) = self.session_manager.get_cancel_token(id) {
            cancel.cancel();
        }
    }

    pub fn subscribe_events(&self, id: &str) -> Option<broadcast::Receiver<crate::agent::session::AgentEvent>> {
        self.session_manager.get_event_sender(id).map(|tx| tx.subscribe())
    }

    pub async fn start_chat(&self, id: &str, user_msg: String) -> Result<(), StartChatError> {
        self.start_chat_stream(id, user_msg, None).await.map(|_| ())
    }

    pub async fn start_chat_stream(
        &self,
        id: &str,
        user_msg: String,
        request_id: Option<String>,
    ) -> Result<broadcast::Receiver<AgentEvent>, StartChatError> {
        let session = self
            .session_manager
            .get_session(id)
            .ok_or(StartChatError::SessionNotFound)?;

        // request_id 规范化：空白字符串按 None 处理，避免出现 "   " 这种伪 ID。
        let normalized_request_id = normalize_request_id(request_id);
        // true 表示“同一个 request_id 的重试请求”，只复用事件订阅，不重复启动 loop。
        let mut idempotent_retry = false;

        // 原子地检查并设置 Running 状态，防止 TOCTOU 竞态。
        {
            let mut s = session.lock().await;
            if let Some(req_id) = normalized_request_id.as_deref() {
                // 这个 request_id 已经完成过：直接拒绝，防止重放。
                if s.is_request_processed(req_id) {
                    return Err(StartChatError::DuplicateRequestId);
                }

                if matches!(s.state, SessionState::Running | SessionState::WaitingApproval) {
                    // 会话正在运行时：
                    // - 同 request_id 视为幂等重试（允许接入同一事件流）
                    // - 不同 request_id 视为并发请求（拒绝）
                    if s.active_request_id() == Some(req_id) {
                        idempotent_retry = true;
                    } else {
                        return Err(StartChatError::SessionBusy);
                    }
                } else {
                    // 会话空闲：开启新一轮，并记录 active request_id。
                    s.set_state(SessionState::Running);
                    s.begin_request(Some(req_id.to_string()));
                }
            } else {
                // 未提供 request_id：保持旧语义，只做并发保护。
                if matches!(s.state, SessionState::Running | SessionState::WaitingApproval) {
                    return Err(StartChatError::SessionBusy);
                }
                s.set_state(SessionState::Running);
                s.begin_request(None);
            }
        }

        let event_tx = self
            .session_manager
            .get_event_sender(id)
            .ok_or(StartChatError::SessionNotFound)?;
        let rx = event_tx.subscribe();

        // 同 request_id 重试请求：到这里就够了，不要再次 spawn loop。
        if idempotent_retry {
            return Ok(rx);
        }

        let cancel = self
            .session_manager
            .get_cancel_token(id)
            .ok_or(StartChatError::SessionNotFound)?;

        let loop_runner = AgentLoop::new(
            self.provider.clone(),
            self.tools.clone(),
            event_tx,
            cancel,
            self.max_context_tokens,
            self.persist_fn.clone(),
        );

        tokio::spawn(async move {
            if let Err(e) = loop_runner.run(&session, user_msg).await {
                tracing::error!("Agent loop error: {}", e);
            }
        });

        Ok(rx)
    }

    pub async fn resolve_permission(
        &self,
        id: &str,
        approved: bool,
    ) -> Result<(), ResolvePermissionError> {
        let session = self
            .session_manager
            .get_session(id)
            .ok_or(ResolvePermissionError::SessionNotFound)?;

        let pending = {
            let mut s = session.lock().await;
            let pending = s
                .pending_approval
                .take()
                .ok_or(ResolvePermissionError::NoPendingApproval)?;
            // 审批动作发生后，把会话从 WaitingApproval 恢复到 Running。
            if s.state == SessionState::WaitingApproval {
                s.set_state(SessionState::Running);
            }
            pending
        };

        pending
            .tx
            .send(approved)
            .map_err(|_| ResolvePermissionError::ApprovalChannelClosed)
    }

    #[allow(dead_code)]
    pub fn get_session(&self, id: &str) -> Option<Arc<Mutex<Session>>> {
        self.session_manager.get_session(id)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StartChatError {
    #[error("Session not found")]
    SessionNotFound,
    #[error("Session is already running or waiting for approval")]
    SessionBusy,
    #[error("Request ID has already been processed")]
    DuplicateRequestId,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolvePermissionError {
    #[error("Session not found")]
    SessionNotFound,
    #[error("No pending approval in this session")]
    NoPendingApproval,
    #[error("Approval channel closed unexpectedly")]
    ApprovalChannelClosed,
}

fn normalize_request_id(request_id: Option<String>) -> Option<String> {
    // 统一清洗请求 ID：trim 后空串视为 None。
    request_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
}
