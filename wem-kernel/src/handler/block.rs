//! Block HTTP 处理层
//!
//! Block 级操作：CRUD、移动、恢复、拆分/合并、批量。
//! 所有路由前缀：`/api/v1/blocks/*`。

use axum::{extract::State, Json};

use crate::api::request::{
    BatchReq, CreateBlockReq, DeleteBlockReq, GetBlockReq, MergeReq, MoveBlockReq,
    MoveHeadingTreeReq, RestoreReq, SplitReq, UpdateBlockReq,
};
use crate::api::response::{BatchResult, DeleteResult, MergeResult, RestoreResult, SplitResult};
use crate::error::{AppError, ApiResponse};
use crate::model::event::BlockEvent;
use crate::model::Block;
use crate::repo::Db;
use crate::service::content;

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
    let operation_id = req.operation_id.clone();
    let blk = tokio::task::spawn_blocking(move || content::create_block(&db, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    crate::service::event::EventBus::global().emit(BlockEvent::BlockCreated {
        document_id: blk.document_id.clone(),
        operation_id,
        block: blk.clone(),
    });

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
        content::get_block(&db, &id, include_deleted)
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
    let operation_id = req.operation_id.clone();
    let id = req.id.clone();
    let blk = tokio::task::spawn_blocking(move || content::update_block(&db, &id, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    crate::service::event::EventBus::global().emit(BlockEvent::BlockUpdated {
        document_id: blk.document_id.clone(),
        operation_id,
        block: blk.clone(),
    });

    Ok(Json(ApiResponse::ok(Some(blk))))
}

/// POST /api/v1/blocks/delete
///
/// 软删除 Block（级联删除子块）
pub async fn delete_block(
    State(db): State<Db>,
    Json(req): Json<DeleteBlockReq>,
) -> Result<Json<ApiResponse<DeleteResult>>, AppError> {
    let operation_id = req.operation_id.clone();
    let id = req.id;
    let id_clone = id.clone();
    let result = tokio::task::spawn_blocking(move || {
        content::delete_block(&db, &id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    crate::service::event::EventBus::global().emit(BlockEvent::BlockDeleted {
        document_id: id_clone,
        operation_id,
        block_id: result.id.clone(),
        cascade_count: result.cascade_count,
    });

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
    let operation_id = req.operation_id.clone();
    let id = req.id.clone();
    let blk = tokio::task::spawn_blocking(move || content::move_block(&db, &id, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    crate::service::event::EventBus::global().emit(BlockEvent::BlockMoved {
        document_id: blk.document_id.clone(),
        operation_id,
        block: blk.clone(),
    });

    Ok(Json(ApiResponse::ok(Some(blk))))
}

/// POST /api/v1/blocks/move-heading-tree
///
/// 移动 Heading 子树（同文档内）
pub async fn move_heading_tree(
    State(db): State<Db>,
    Json(req): Json<MoveHeadingTreeReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let operation_id = req.operation_id.clone();
    let blk = tokio::task::spawn_blocking(move || content::move_heading_tree(&db, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    crate::service::event::EventBus::global().emit(BlockEvent::BlockMoved {
        document_id: blk.document_id.clone(),
        operation_id,
        block: blk.clone(),
    });

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
    let operation_id = req.operation_id.clone();
    let id = req.id;
    let result = tokio::task::spawn_blocking(move || {
        let restore_result = content::restore_block(&db, &id)?;
        // 在同一锁范围内查询最新状态用于广播
        let restored = content::get_block(&db, &id, false)?;
        Ok::<_, AppError>((restore_result, restored))
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    let (_result, restored) = result;

    crate::service::event::EventBus::global().emit(BlockEvent::BlockRestored {
        document_id: restored.document_id.clone(),
        operation_id,
        block: restored,
    });

    Ok(Json(ApiResponse::ok(Some(_result))))
}

// ─── Split / Merge ─────────────────────────────────────────────

/// POST /api/v1/blocks/split
///
/// 原子拆分：更新当前块内容 + 创建新块
pub async fn split_block(
    State(db): State<Db>,
    Json(req): Json<SplitReq>,
) -> Result<Json<ApiResponse<SplitResult>>, AppError> {
    let operation_id = req.operation_id.clone();
    let id = req.id.clone();
    let result = tokio::task::spawn_blocking(move || {
        content::split_block(&db, &id, req)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    let doc_id = result.updated_block.document_id.clone();
    crate::service::event::EventBus::global().emit(BlockEvent::BlockUpdated {
        document_id: doc_id.clone(),
        operation_id: operation_id.clone(),
        block: result.updated_block.clone(),
    });
    crate::service::event::EventBus::global().emit(BlockEvent::BlockCreated {
        document_id: doc_id,
        operation_id,
        block: result.new_block.clone(),
    });

    Ok(Json(ApiResponse::ok(Some(result))))
}

/// POST /api/v1/blocks/merge
///
/// 原子合并：当前块内容追加到前一个兄弟 + 删除当前块
pub async fn merge_block(
    State(db): State<Db>,
    Json(req): Json<MergeReq>,
) -> Result<Json<ApiResponse<MergeResult>>, AppError> {
    let operation_id = req.operation_id.clone();
    let id = req.id.clone();
    let result = tokio::task::spawn_blocking(move || {
        content::merge_block(&db, &id, req)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    let doc_id = result.merged_block.document_id.clone();
    crate::service::event::EventBus::global().emit(BlockEvent::BlockUpdated {
        document_id: doc_id.clone(),
        operation_id: operation_id.clone(),
        block: result.merged_block.clone(),
    });
    crate::service::event::EventBus::global().emit(BlockEvent::BlockDeleted {
        document_id: doc_id,
        operation_id,
        block_id: result.deleted_block_id.clone(),
        cascade_count: 0,
    });

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
    let operation_id = req.operation_id.clone();
    let db_for_query = db.clone();
    let result = tokio::task::spawn_blocking(move || {
        content::batch_operations(&db, req)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    // 简化：对整个批次发一个泛事件，前端收到后 refetch
    let bus = crate::service::event::EventBus::global();
    let first_id = result.results.iter()
        .find(|r| r.error.is_none())
        .map(|r| r.block_id.clone());

    if let Some(first_block_id) = first_id {
        let doc_id = tokio::task::spawn_blocking(move || {
            content::get_block(&db_for_query, &first_block_id, true).ok()
        })
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))?;

        if let Some(blk) = doc_id {
            bus.emit(BlockEvent::BlocksBatchChanged {
                document_id: blk.document_id.clone(),
                operation_id,
            });
        }
    }

    Ok(Json(ApiResponse::ok(Some(result))))
}
