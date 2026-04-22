//! Block HTTP 处理层
//!
//! Block 级操作：CRUD、移动、恢复、拆分/合并、批量、导出。
//! 所有路由前缀：`/api/v1/blocks/*`。

use axum::{extract::State, Json};

use crate::api::request::{
    BatchReq, CreateBlockReq, DeleteBlockReq, ExportBlockReq, GetBlockReq, MergeReq, MoveBlockReq,
    MoveHeadingTreeReq, RestoreReq, SplitReq, UpdateBlockReq,
};
use crate::api::response::{BatchResult, DeleteResult, ExportResult, MergeResult, RestoreResult, SplitResult};
use crate::error::{AppError, ApiResponse};
use crate::block_system::model::Block;
use crate::repo::Db;
use crate::block_system::service::block;

// ─── Block API ─────────────────────────────────────────────────

/// POST /api/v1/blocks
///
/// 创建 Block
pub async fn create_block(
    State(db): State<Db>,
    Json(req): Json<CreateBlockReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    if req.content.len() > 1_000_000 {
        return Err(AppError::BadRequest("content 长度超过限制 (1MB)".to_string()));
    }
    let blk = tokio::task::spawn_blocking(move || block::create_block(&db, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(blk))))
}

/// POST /api/v1/blocks/get
///
/// 获取单个 Block
pub async fn get_block(
    State(db): State<Db>,
    Json(req): Json<GetBlockReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let id = req.id;
    let include_deleted = req.include_deleted;
    let blk = tokio::task::spawn_blocking(move || {
        block::get_block(&db, &id, include_deleted)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(blk))))
}

/// POST /api/v1/blocks/update
///
/// 更新 Block 内容和/或属性
pub async fn update_block(
    State(db): State<Db>,
    Json(req): Json<UpdateBlockReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    if let Some(ref content) = req.content {
        if content.len() > 1_000_000 {
            return Err(AppError::BadRequest("content 长度超过限制 (1MB)".to_string()));
        }
    }
    let id = req.id.clone();
    let blk = tokio::task::spawn_blocking(move || block::update_block(&db, &id, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(blk))))
}

/// POST /api/v1/blocks/delete
///
/// 删除单个 Block（子块提升到父级）
pub async fn delete_block(
    State(db): State<Db>,
    Json(req): Json<DeleteBlockReq>,
) -> Result<Json<ApiResponse<DeleteResult>>, AppError> {
    let id = req.id;
    let result = tokio::task::spawn_blocking(move || {
        block::delete_block(&db, &id, req.editor_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

/// POST /api/v1/blocks/delete-tree
///
/// 级联删除 Block 及其所有后代
pub async fn delete_tree(
    State(db): State<Db>,
    Json(req): Json<DeleteBlockReq>,
) -> Result<Json<ApiResponse<DeleteResult>>, AppError> {
    let id = req.id;
    let result = tokio::task::spawn_blocking(move || {
        block::delete_tree(&db, &id, req.editor_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

// ─── Move API ──────────────────────────────────────────────────

/// POST /api/v1/blocks/move
///
/// 移动 Block（改变父块和/或位置）
pub async fn move_block(
    State(db): State<Db>,
    Json(req): Json<MoveBlockReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let id = req.id.clone();
    let blk = tokio::task::spawn_blocking(move || block::move_block(&db, &id, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(blk))))
}

/// POST /api/v1/blocks/move-heading-tree
///
/// 移动 Heading 子树（同文档内）
pub async fn move_heading_tree(
    State(db): State<Db>,
    Json(req): Json<MoveHeadingTreeReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let blk = tokio::task::spawn_blocking(move || block::move_heading_tree(&db, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(blk))))
}

// ─── Restore ───────────────────────────────────────────────────

/// POST /api/v1/blocks/restore
///
/// 恢复已软删除的 Block
pub async fn restore_block(
    State(db): State<Db>,
    Json(req): Json<RestoreReq>,
) -> Result<Json<ApiResponse<RestoreResult>>, AppError> {
    let id = req.id;
    let result = tokio::task::spawn_blocking(move || {
        block::restore_block(&db, &id, req.editor_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

// ─── Split / Merge ─────────────────────────────────────────────

/// POST /api/v1/blocks/split
///
/// 原子拆分：更新当前块内容 + 创建新块
pub async fn split_block(
    State(db): State<Db>,
    Json(req): Json<SplitReq>,
) -> Result<Json<ApiResponse<SplitResult>>, AppError> {
    let id = req.id.clone();
    let result = tokio::task::spawn_blocking(move || {
        block::split_block(&db, &id, req)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

/// POST /api/v1/blocks/merge
///
/// 原子合并：当前块内容追加到前一个兄弟 + 删除当前块
pub async fn merge_block(
    State(db): State<Db>,
    Json(req): Json<MergeReq>,
) -> Result<Json<ApiResponse<MergeResult>>, AppError> {
    let id = req.id.clone();
    let result = tokio::task::spawn_blocking(move || {
        block::merge_block(&db, &id, req)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

// ─── Batch ─────────────────────────────────────────────────────

/// POST /api/v1/blocks/batch
///
/// 批量执行多个 Block 操作（创建/更新/删除/移动）
pub async fn batch_blocks(
    State(db): State<Db>,
    Json(req): Json<BatchReq>,
) -> Result<Json<ApiResponse<BatchResult>>, AppError> {
    let result = tokio::task::spawn_blocking(move || {
        block::batch_operations(&db, req)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

// ─── Export ─────────────────────────────────────────────────────

/// POST /api/v1/blocks/export
///
/// 导出任意 Block 及其子树为 Markdown 等格式
pub async fn export_block(
    State(db): State<Db>,
    Json(req): Json<ExportBlockReq>,
) -> Result<Json<ApiResponse<ExportResult>>, AppError> {
    let depth = match req.depth.as_str() {
        "children" => block::ExportDepth::Children,
        _ => block::ExportDepth::Descendants,
    };
    let id = req.id;
    let format = req.format;
    let result = tokio::task::spawn_blocking(move || {
        block::export_block(&db, &id, &format, depth)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}
