//! 系统与 SSE 事件 HTTP 处理层
//!
//! 健康检查 + Server-Sent Events 实时推送。

use std::convert::Infallible;

use axum::extract::Path;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;

use crate::dto::ApiResponse;

// ─── Health 健康检查 ────────────────────────────────────────────

/// GET /api/v1/health
pub async fn health() -> Json<ApiResponse<()>> {
    Json(ApiResponse::ok(None))
}

// ─── SSE 实时事件 ──────────────────────────────────────────────

/// GET /api/v1/documents/{id}/events
///
/// SSE 实时事件端点。订阅指定文档的所有变更事件。
/// 前端通过 EventSource 连接此端点，实现"后端即数据真相源"。
///
/// 事件格式：
/// ```text
/// event: block_created
/// data: {"type":"block_created","document_id":"...","block":{...}}
/// ```
pub async fn document_events(
    Path(document_id): Path<String>,
) -> impl IntoResponse {
    let mut receiver = crate::block_system::service::event::EventBus::global().subscribe();

    // 将 broadcast::Receiver 转为 Stream，过滤当前文档事件
    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(event) if event.document_id() == document_id => {
                    let data = match serde_json::to_string(&event) {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::error!("SSE 序列化失败: {}", e);
                            continue;
                        }
                    };
                    yield Ok::<_, Infallible>(Event::default().event(event.event_type()).data(data));
                }
                Ok(_) => continue, // 其他文档事件，跳过
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("SSE client lagged, skipped {} events", n);
                    continue;
                }
                Err(_) => break, // 通道关闭
            }
        }
    };

    Sse::new(Box::pin(stream)).keep_alive(KeepAlive::default())
}
