//! Session Manager — Agent 会话生命周期管理

use std::path::PathBuf;
use std::sync::Arc;
use std::collections::VecDeque;

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
    Completed,
    Error,
}

// ─── 循环阶段（推送给前端的强类型事件）──────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Initializing,
    PreparingTurn,
    StreamingModel,
    ExecutingTools,
    Completed,
    Cancelled,
    Failed,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Phase::Initializing => "initializing",
            Phase::PreparingTurn => "preparing_turn",
            Phase::StreamingModel => "streaming_model",
            Phase::ExecutingTools => "executing_tools",
            Phase::Completed => "completed",
            Phase::Cancelled => "cancelled",
            Phase::Failed => "failed",
        };
        f.write_str(name)
    }
}

impl Phase {
    /// 从 loop_runner 内部的 LoopPhase 转换为对外暴露的 Phase。
    /// LoopPhase 是私有类型，通过此方法桥接。
    pub fn from_loop_phase(lp: crate::agent::loop_runner::LoopPhase) -> Self {
        use crate::agent::loop_runner::LoopPhase;
        match lp {
            LoopPhase::Initializing => Phase::Initializing,
            LoopPhase::PreparingTurn => Phase::PreparingTurn,
            LoopPhase::StreamingModel => Phase::StreamingModel,
            LoopPhase::ExecutingTools => Phase::ExecutingTools,
            LoopPhase::Completed => Phase::Completed,
            LoopPhase::Cancelled => Phase::Cancelled,
            LoopPhase::Failed => Phase::Failed,
        }
    }
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
    PhaseChanged { phase: Phase },
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
    // 当前正在执行的请求 ID。
    // 用途：当客户端因网络重试同一个 request_id 时，避免重复启动 loop。
    active_request_id: Option<String>,
    // 最近已经处理完成的 request_id（固定长度缓存）。
    // 用途：防止同一个 request_id 被重复提交（重放请求）。
    recent_request_ids: VecDeque<String>,
}

// request_id 去重窗口大小：只保留最近 N 个已完成请求。
// N 太小会降低防重放效果，N 太大则会占用更多内存。
const MAX_RECENT_REQUEST_IDS: usize = 128;

impl Session {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            state: SessionState::Idle,
            messages: Vec::new(),
            config,
            pending_approval: None,
            active_request_id: None,
            recent_request_ids: VecDeque::new(),
        }
    }

    pub fn push_message(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    pub fn set_state(&mut self, state: SessionState) {
        self.state = state;
    }

    /// 开始处理一轮请求时记录 request_id（可为空）。
    pub fn begin_request(&mut self, request_id: Option<String>) {
        self.active_request_id = request_id;
    }

    /// 返回当前正在执行的 request_id，用于幂等重试判断。
    pub fn active_request_id(&self) -> Option<&str> {
        self.active_request_id.as_deref()
    }

    /// 判断给定 request_id 是否已经处理完成。
    pub fn is_request_processed(&self, request_id: &str) -> bool {
        self.recent_request_ids.iter().any(|id| id == request_id)
    }

    /// 在请求结束时调用：
    /// 1) 清理 active_request_id
    /// 2) 把已完成 request_id 放入最近缓存（用于防重放）
    pub fn finish_request(&mut self) {
        // 没有 request_id 的请求不参与幂等去重（保持兼容旧行为）。
        let Some(request_id) = self.active_request_id.take() else {
            return;
        };

        // 防止重复插入同一个 request_id。
        if self.recent_request_ids.iter().any(|id| id == &request_id) {
            return;
        }

        self.recent_request_ids.push_back(request_id);
        // 维持固定窗口，超出后丢弃最旧记录。
        while self.recent_request_ids.len() > MAX_RECENT_REQUEST_IDS {
            self.recent_request_ids.pop_front();
        }
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
