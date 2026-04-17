//! Oplog HTTP 处理层
//!
//! 历史查询、版本回放、回滚、手动快照 等端点。

use axum::{
    extract::State,
    Json,
};

use crate::api::request::{GetHistoryReq, GetVersionReq, RollbackReq, SnapshotReq};
use crate::api::response::{
    HistoryResponse, RollbackResponse, SnapshotResponse, VersionResponse,
};
use crate::repo::Db;
use crate::error::{AppError, ApiResponse};
use crate::service::oplog;

// ─── 历史查询 ──────────────────────────────────────────────────

/// POST /api/v1/blocks/history
///
/// 获取 Block 的变更历史
pub async fn get_block_history(
    State(db): State<Db>,
    Json(req): Json<GetHistoryReq>,
) -> Result<Json<ApiResponse<HistoryResponse>>, AppError> {
    let id = req.id;
    let limit = req.limit.clamp(1, 500);

    let entries = tokio::task::spawn_blocking(move || {
        oplog::get_block_history(&db, &id, limit)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(HistoryResponse { entries }))))
}

// ─── 版本回放 ──────────────────────────────────────────────────

/// POST /api/v1/blocks/version
///
/// 获取 Block 在指定版本的完整内容
pub async fn get_block_version(
    State(db): State<Db>,
    Json(req): Json<GetVersionReq>,
) -> Result<Json<ApiResponse<VersionResponse>>, AppError> {
    let id = req.id;
    let version = req.version;
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

/// POST /api/v1/blocks/rollback
///
/// 回滚 Block 到指定版本
pub async fn rollback_block(
    State(db): State<Db>,
    Json(req): Json<RollbackReq>,
) -> Result<Json<ApiResponse<RollbackResponse>>, AppError> {
    let id = req.id;
    let target_version = req.target_version;
    let result = tokio::task::spawn_blocking(move || {
        oplog::rollback_block(&db, &id, target_version)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(RollbackResponse { result }))))
}

// ─── 手动快照 ──────────────────────────────────────────────────

/// POST /api/v1/blocks/snapshot
///
/// 手动创建 Block 快照
pub async fn create_snapshot(
    State(db): State<Db>,
    Json(req): Json<SnapshotReq>,
) -> Result<Json<ApiResponse<SnapshotResponse>>, AppError> {
    let id = req.id;
    let result = tokio::task::spawn_blocking(move || {
        oplog::create_snapshot(&db, &id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(SnapshotResponse { result }))))
}
