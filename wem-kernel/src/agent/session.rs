//! Session Manager — Agent 会话生命周期管理

use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;

use crate::agent::provider::Message;

// ─── 会话状态 ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Running,
    WaitingApproval,
    Error,
}

// ─── 会话配置 ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub model: String,
    pub temperature: f32,
    pub max_steps: u32,
    pub allowed_tools: Vec<String>,
    pub system_prompt_override: Option<String>,
    pub working_dir: PathBuf,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-20250514".to_string(),
            temperature: 0.3,
            max_steps: 50,
            allowed_tools: vec![],
            system_prompt_override: None,
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

// ─── SSE 事件（推送给前端）─────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    TextDelta { text: String },
    ToolCallBegin { id: String, name: String, args: serde_json::Value },
    ToolCallEnd { id: String, result_summary: String },
    PermissionRequired { tool_name: String, args: serde_json::Value },
    StepProgress { step: u32, max_steps: u32 },
    Done,
    Error { message: String },
}

// ─── 权限等待 ──────────────────────────────────────────────────

pub struct PendingApproval {
    pub tool_name: String,
    pub args: serde_json::Value,
    pub tx: tokio::sync::oneshot::Sender<bool>,
}

// ─── 会话 ──────────────────────────────────────────────────────

pub struct Session {
    pub id: String,
    pub state: SessionState,
    pub messages: Vec<Message>,
    pub config: SessionConfig,
    pub pending_approval: Option<PendingApproval>,
}

impl Session {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            state: SessionState::Idle,
            messages: Vec::new(),
            config,
            pending_approval: None,
        }
    }

    pub fn push_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    pub fn set_state(&mut self, state: SessionState) {
        self.state = state;
    }
}

// ─── Active Session（运行时）───────────────────────────────────

struct ActiveSession {
    session: Arc<Mutex<Session>>,
    cancel: CancellationToken,
    event_tx: broadcast::Sender<AgentEvent>,
}

// ─── Session Manager ───────────────────────────────────────────

pub struct SessionManager {
    sessions: DashMap<String, ActiveSession>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    pub fn create_session(&self, config: SessionConfig) -> String {
        let session = Session::new(config);
        let id = session.id.clone();
        let (event_tx, _) = broadcast::channel(256);

        self.sessions.insert(id.clone(), ActiveSession {
            session: Arc::new(Mutex::new(session)),
            cancel: CancellationToken::new(),
            event_tx,
        });

        id
    }

    pub fn get_session(&self, id: &str) -> Option<Arc<Mutex<Session>>> {
        self.sessions.get(id).map(|e| e.session.clone())
    }

    pub fn get_event_sender(&self, id: &str) -> Option<broadcast::Sender<AgentEvent>> {
        self.sessions.get(id).map(|e| e.event_tx.clone())
    }

    pub fn get_cancel_token(&self, id: &str) -> Option<CancellationToken> {
        self.sessions.get(id).map(|e| e.cancel.clone())
    }

    pub fn destroy_session(&self, id: &str) -> bool {
        if let Some(entry) = self.sessions.get(id) {
            entry.cancel.cancel();
        }
        self.sessions.remove(id).is_some()
    }

    pub fn list_sessions(&self) -> Vec<String> {
        self.sessions.iter().map(|e| e.key().clone()).collect()
    }
}
