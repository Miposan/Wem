//! Agent HTTP Handler — Axum 路由处理
//!
//! 提供 Agent 的 REST API 端点。

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::sse::{Event, Sse};
use axum::Json;
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::agent::runtime::{AgentRuntime, ResolvePermissionError, StartChatError};
use crate::agent::session::{AgentEvent, SessionConfig};

// ─── 共享状态 ──────────────────────────────────────────────────

pub struct AgentState {
    pub runtime: Arc<AgentRuntime>,
}

// ─── 请求/响应 DTO ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateSessionReq {
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_steps: Option<u32>,
    pub working_dir: Option<String>,
    pub allowed_tools: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct SessionIdResp {
    pub session_id: String,
}

#[derive(Debug, Serialize)]
pub struct SessionListResp {
    pub sessions: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatReq {
    pub message: String,
    // 可选幂等键：客户端重试同一请求时应传同一个 request_id。
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PermissionDecisionReq {
    pub approved: bool,
}

// ─── Handler 函数 ──────────────────────────────────────────────

pub async fn create_session(
    State(state): State<Arc<AgentState>>,
    Json(req): Json<CreateSessionReq>,
) -> Json<SessionIdResp> {
    let mut config = SessionConfig::default();
    if let Some(model) = req.model {
        config.model = model;
    }
    if let Some(temp) = req.temperature {
        config.temperature = temp;
    }
    if let Some(steps) = req.max_steps {
        config.max_steps = steps;
    }
    if let Some(dir) = req.working_dir {
        config.working_dir = std::path::PathBuf::from(dir);
    }
    if let Some(tools) = req.allowed_tools {
        config.allowed_tools = tools;
    }

    let id = state.runtime.create_session(config);
    Json(SessionIdResp { session_id: id })
}

pub async fn list_sessions(
    State(state): State<Arc<AgentState>>,
) -> Json<SessionListResp> {
    Json(SessionListResp {
        sessions: state.runtime.list_sessions(),
    })
}

pub async fn destroy_session(
    State(state): State<Arc<AgentState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let ok = state.runtime.destroy_session(&id);
    Json(serde_json::json!({ "ok": ok }))
}

pub async fn chat(
    State(state): State<Arc<AgentState>>,
    Path(id): Path<String>,
    Json(req): Json<ChatReq>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, axum::http::StatusCode> {
    // chat 端点同时承担两件事：
    // 1) 启动一轮对话（或幂等重试时复用已有轮次）
    // 2) 返回该轮对话的事件流
    let rx = match state
        .runtime
        .start_chat_stream(&id, req.message, req.request_id)
        .await
    {
        Ok(rx) => rx,
        Err(StartChatError::SessionNotFound) => return Err(axum::http::StatusCode::NOT_FOUND),
        Err(StartChatError::SessionBusy) => return Err(axum::http::StatusCode::CONFLICT),
        Err(StartChatError::DuplicateRequestId) => {
            return Err(axum::http::StatusCode::CONFLICT);
        }
    };
    Ok(Sse::new(event_stream(rx, true)))
}

pub async fn events(
    State(state): State<Arc<AgentState>>,
    Path(id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, axum::http::StatusCode> {
    let rx = state
        .runtime
        .subscribe_events(&id)
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;

    // 会话级事件订阅：连接在 done/error 后保持，便于持续监听下一轮对话。
    // 与 chat 的区别：events 不会触发新对话，只负责“听事件”。
    Ok(Sse::new(event_stream(rx, false)))
}

pub async fn abort_session(
    State(state): State<Arc<AgentState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    state.runtime.abort_session(&id);
    Json(serde_json::json!({ "ok": true }))
}

pub async fn resolve_permission(
    State(state): State<Arc<AgentState>>,
    Path(id): Path<String>,
    Json(req): Json<PermissionDecisionReq>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    match state.runtime.resolve_permission(&id, req.approved).await {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true }))),
        Err(ResolvePermissionError::SessionNotFound) => Err(axum::http::StatusCode::NOT_FOUND),
        Err(ResolvePermissionError::NoPendingApproval) => Err(axum::http::StatusCode::CONFLICT),
        Err(ResolvePermissionError::ApprovalChannelClosed) => Err(axum::http::StatusCode::CONFLICT),
    }
}

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "agent_ok" }))
}

fn event_stream(
    mut rx: tokio::sync::broadcast::Receiver<AgentEvent>,
    break_on_terminal: bool,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok(Event::default().data(data));
                    // chat 模式下在 done/error 后自动断开；
                    // events 模式下保持连接，继续等待后续轮次事件。
                    if break_on_terminal && matches!(event, AgentEvent::Done | AgentEvent::Error { .. }) {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                }
                Err(_) => break,
            }
        }
    }
}
