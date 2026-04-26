//! 操作日志 HTTP 处理层
//!
//! 操作日志查询、Undo、Redo。
//! 所有路由前缀：`/api/v1/documents/*`（per-document 操作）。

use axum::{extract::State, Json};

use crate::api::request::{GetHistoryReq, RedoReq, UndoReq};
use crate::api::response::{HistoryResponse, UndoRedoResponse};
use crate::error::{AppError, ApiResponse, blocking};
use crate::repo::Db;
use crate::block_system::service::oplog;

// ─── 历史查询 ──────────────────────────────────────────────────

/// POST /api/v1/documents/history
///
/// 获取文档变更历史（支持按 Block ID 或 Document ID 查询）
pub async fn get_block_history(
    State(db): State<Db>,
    Json(req): Json<GetHistoryReq>,
) -> Result<Json<ApiResponse<HistoryResponse>>, AppError> {
    let limit = req.limit.clamp(1, 500);
    blocking(move || {
        let entries = if let Some(block_id) = req.id {
            oplog::get_block_history(&db, &block_id, limit)?
        } else if let Some(doc_id) = req.document_id {
            oplog::get_history(&db, &doc_id, limit, req.offset)?
        } else {
            vec![]
        };
        Ok(HistoryResponse { entries })
    }).await
}

// ─── Undo ──────────────────────────────────────────────────────

/// POST /api/v1/documents/undo
///
/// 撤销最近一次操作
pub async fn undo(
    State(db): State<Db>,
    Json(req): Json<UndoReq>,
) -> Result<Json<ApiResponse<UndoRedoResponse>>, AppError> {
    blocking(move || {
        let result = oplog::undo(&db, &req.document_id)?;
        Ok(UndoRedoResponse { result })
    }).await
}

// ─── Redo ──────────────────────────────────────────────────────

/// POST /api/v1/documents/redo
///
/// 重做最近被撤销的操作
pub async fn redo(
    State(db): State<Db>,
    Json(req): Json<RedoReq>,
) -> Result<Json<ApiResponse<UndoRedoResponse>>, AppError> {
    blocking(move || {
        let result = oplog::redo(&db, &req.document_id)?;
        Ok(UndoRedoResponse { result })
    }).await
}
