//! Agent Runtime — 传输层无关的运行时内核
//!
//! 抽离会话管理、循环调度与事件订阅，供 HTTP/CLI 等接入层复用。

use std::sync::Arc;

use tokio::sync::{broadcast, Mutex};

use crate::agent::loop_runner::AgentLoop;
use crate::agent::provider::Provider;
use crate::agent::session::{Session, SessionConfig, SessionManager, SessionState};
use crate::agent::tools::ToolRegistry;

pub struct AgentRuntime {
    session_manager: Arc<SessionManager>,
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    max_context_tokens: u32,
}

impl AgentRuntime {
    pub fn new(
        session_manager: Arc<SessionManager>,
        provider: Arc<dyn Provider>,
        tools: Arc<ToolRegistry>,
        max_context_tokens: u32,
    ) -> Self {
        Self {
            session_manager,
            provider,
            tools,
            max_context_tokens,
        }
    }

    pub fn create_session(&self, config: SessionConfig) -> String {
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
        let session = self
            .session_manager
            .get_session(id)
            .ok_or(StartChatError::SessionNotFound)?;

        // 防止同一 session 并发运行多个 loop。
        {
            let s = session.lock().await;
            if s.state == SessionState::Running {
                return Err(StartChatError::SessionBusy);
            }
        }

        let event_tx = self
            .session_manager
            .get_event_sender(id)
            .ok_or(StartChatError::SessionNotFound)?;
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
        );

        tokio::spawn(async move {
            if let Err(e) = loop_runner.run(&session, user_msg).await {
                tracing::error!("Agent loop error: {}", e);
            }
        });

        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_session(&self, id: &str) -> Option<Arc<Mutex<Session>>> {
        self.session_manager.get_session(id)
    }
}

#[derive(Debug)]
pub enum StartChatError {
    SessionNotFound,
    SessionBusy,
}
