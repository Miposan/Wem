//! 操作日志 HTTP 处理层
//!
//! 操作日志查询、Undo、Redo。
//! 所有路由前缀：`/api/v1/documents/*`（per-document 操作）。

use axum::{extract::State, Json};

use crate::api::request::{GetHistoryReq, RedoReq, UndoReq};
use crate::api::response::{HistoryResponse, UndoRedoResponse};
use crate::error::{AppError, ApiResponse};
use crate::repo::Db;
use crate::service::oplog;

// ─── 历史查询 ──────────────────────────────────────────────────

/// POST /api/v1/documents/history
///
/// 获取文档变更历史（支持按 Block ID 或 Document ID 查询）
pub async fn get_block_history(
    State(db): State<Db>,
    Json(req): Json<GetHistoryReq>,
) -> Result<Json<ApiResponse<HistoryResponse>>, AppError> {
    let limit = req.limit.clamp(1, 500);

    let entries = if let Some(id) = req.id {
        // 按 Block ID 查询
        let block_id = id;
        let changes = tokio::task::spawn_blocking(move || {
            oplog::get_block_history(&db, &block_id, limit)
        })
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

        // 将 Change 列表包装成简化的 HistoryEntry
        changes
            .into_iter()
            .map(|c| crate::model::oplog::HistoryEntry {
                operation_id: c.operation_id,
                action: c.change_type.as_str().to_string(),
                description: None,
                timestamp: String::new(), // Change 不含 timestamp，由 Batch 提供
                undone: false,
                changes: vec![crate::model::oplog::ChangeSummary {
                    block_id: c.block_id,
                    change_type: c.change_type.as_str().to_string(),
                }],
            })
            .collect()
    } else if let Some(doc_id) = req.document_id {
        // 按文档查询历史
        tokio::task::spawn_blocking(move || oplog::get_history(&db, &doc_id, limit, req.offset))
            .await
            .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??
    } else {
        // 未指定 document_id，返回空
        vec![]
    };

    Ok(Json(ApiResponse::ok(Some(HistoryResponse { entries }))))
}

// ─── Undo ──────────────────────────────────────────────────────

/// POST /api/v1/documents/undo
///
/// 撤销最近一次操作
pub async fn undo(
    State(db): State<Db>,
    Json(req): Json<UndoReq>,
) -> Result<Json<ApiResponse<UndoRedoResponse>>, AppError> {
    let document_id = req.document_id.clone();
    let result = tokio::task::spawn_blocking(move || oplog::undo(&db, &document_id))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(UndoRedoResponse { result }))))
}

// ─── Redo ──────────────────────────────────────────────────────

/// POST /api/v1/documents/redo
///
/// 重做最近被撤销的操作
pub async fn redo(
    State(db): State<Db>,
    Json(req): Json<RedoReq>,
) -> Result<Json<ApiResponse<UndoRedoResponse>>, AppError> {
    let document_id = req.document_id.clone();
    let result = tokio::task::spawn_blocking(move || oplog::redo(&db, &document_id))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(UndoRedoResponse { result }))))
}
