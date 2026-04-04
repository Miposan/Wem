//! Oplog HTTP 处理层
//!
//! 历史查询、版本回放、回滚、手动快照 等端点。

use axum::{
    extract::{Path, Query, State},
    Json,
};

use crate::api::query::{HistoryQuery, RollbackReq};
use crate::api::response::{
    HistoryResponse, RollbackResponse, SnapshotResponse, VersionResponse,
};
use crate::db::Db;
use crate::error::{AppError, ApiResponse};
use crate::service::oplog;

// ─── 历史查询 ──────────────────────────────────────────────────

/// GET /api/v1/blocks/{id}/history
///
/// 获取 Block 的变更历史
pub async fn get_block_history(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<ApiResponse<HistoryResponse>>, AppError> {
    let limit = query.limit.clamp(1, 500);

    let entries = tokio::task::spawn_blocking(move || {
        oplog::get_block_history(&db, &id, limit)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(HistoryResponse { entries }))))
}

// ─── 版本回放 ──────────────────────────────────────────────────

/// GET /api/v1/blocks/{id}/versions/{version}
///
/// 获取 Block 在指定版本的完整内容
pub async fn get_block_version(
    State(db): State<Db>,
    Path((id, version)): Path<(String, u64)>,
) -> Result<Json<ApiResponse<VersionResponse>>, AppError> {
    let version_content = tokio::task::spawn_blocking(move || {
        oplog::get_version_content(&db, &id, version)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(VersionResponse {
        version: version_content,
    }))))
}

// ─── 回滚 ──────────────────────────────────────────────────────

/// POST /api/v1/blocks/{id}/rollback
///
/// 回滚 Block 到指定版本
pub async fn rollback_block(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(req): Json<RollbackReq>,
) -> Result<Json<ApiResponse<RollbackResponse>>, AppError> {
    let result = tokio::task::spawn_blocking(move || {
        oplog::rollback_block(&db, &id, req.target_version, req.current_version)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(RollbackResponse { result }))))
}

// ─── 手动快照 ──────────────────────────────────────────────────

/// POST /api/v1/blocks/{id}/snapshot
///
/// 手动创建 Block 快照
pub async fn create_snapshot(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<SnapshotResponse>>, AppError> {
    let result = tokio::task::spawn_blocking(move || {
        oplog::create_snapshot(&db, &id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(SnapshotResponse { result }))))
}
