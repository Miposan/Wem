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

use crate::agent::runtime::{AgentRuntime, StartChatError};
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
    config.allowed_tools = vec![];

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
    let event_tx = state
        .runtime
        .subscribe_events(&id)
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;

    match state.runtime.start_chat(&id, req.message).await {
        Ok(()) => {}
        Err(StartChatError::SessionNotFound) => return Err(axum::http::StatusCode::NOT_FOUND),
        Err(StartChatError::SessionBusy) => return Err(axum::http::StatusCode::CONFLICT),
    }

    // SSE 流：订阅 Agent 事件
    let mut rx = event_tx;
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok(Event::default().data(data));
                    if matches!(event, AgentEvent::Done | AgentEvent::Error { .. }) {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                }
                Err(_) => break,
            }
        }
    };

    Ok(Sse::new(stream))
}

pub async fn abort_session(
    State(state): State<Arc<AgentState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    state.runtime.abort_session(&id);
    Json(serde_json::json!({ "ok": true }))
}

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "agent_ok" }))
}
