//! HTTP 处理层
//!
//! Axum route handlers：解析 HTTP 请求 → 调用 service 层 → 返回 JSON 响应。
//! 所有 handler 接收 `State<Db>` 作为数据库连接。
//!
//! **Event-Driven 设计**：
//! 每个 mutation handler 在 service 调用成功后，通过 EventBus 广播变更事件。
//! SSE 端点 (`document_events`) 将事件推送给前端。
//! 无论变更来自前端 REST 调用还是 Agent 后端操作，都走同一条事件通道。

use std::convert::Infallible;

use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::{
    extract::{Path, State},
    Json,
};

use crate::api::request::{
    BatchReq, CreateBlockReq, CreateDocumentReq, DeleteBlockReq, DeleteDocumentReq,
    ExportReq, GetBlockReq, GetChildrenReq, GetDocumentReq, ImportTextReq,
    MergeReq, MoveBlockReq, RestoreReq, SplitReq, UpdateBlockReq,
};
use crate::api::response::{
    BatchResult, DeleteResult, DocumentChildrenResult,
    DocumentContentResult, ExportResult, ImportResult, MergeResult,
    RestoreResult, SplitResult,
};
use crate::model::event::BlockEvent;
use crate::repo::Db;
use crate::error::{AppError, ApiResponse};
use crate::model::Block;
use crate::service::{block, document};

// ─── Health 健康检查 ────────────────────────────────────────────

/// GET /api/v1/health
pub async fn health() -> Json<ApiResponse<()>> {
    Json(ApiResponse::ok(None))
}

// ─── Root API ───────────────────────────────────────────────────
// get_root 已删除：前端不需要单独获取全局根块

// ─── Document API ──────────────────────────────────────────────

/// POST /api/v1/documents
///
/// 创建文档（自动附带一个空段落子块）
pub async fn create_document(
    State(db): State<Db>,
    Json(req): Json<CreateDocumentReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    if req.title.len() > 500 {
        return Err(AppError::BadRequest("title 长度超过限制 (500字符)".to_string()));
    }
    let operation_id = req.operation_id.clone();
    let title = req.title;
    let parent_id = req.parent_id;
    let after_id = req.after_id;

    let doc = tokio::task::spawn_blocking(move || {
        document::create_document(&db, title, parent_id, after_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    crate::service::event::EventBus::global().emit(BlockEvent::BlockCreated {
        document_id: doc.document_id.clone(),
        operation_id,
        block: doc.clone(),
    });

    Ok(Json(ApiResponse::ok(Some(doc))))
}

/// POST /api/v1/documents/list
///
/// 列出所有根文档（不分页）
pub async fn list_documents(
    State(db): State<Db>,
) -> Result<Json<ApiResponse<Vec<Block>>>, AppError> {
    let docs = tokio::task::spawn_blocking(move || document::list_root_documents(&db))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(docs))))
}

/// POST /api/v1/documents/get
///
/// 获取文档内容（编辑器渲染用）：文档块 + 所有非 document 类型的内容块
pub async fn get_document(
    State(db): State<Db>,
    Json(req): Json<GetDocumentReq>,
) -> Result<Json<ApiResponse<DocumentContentResult>>, AppError> {
    let doc_id = req.id;
    let result = tokio::task::spawn_blocking(move || document::get_document_content(&db, &doc_id))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

/// POST /api/v1/documents/children
///
/// 获取文档直系子文档列表（侧边栏导航用）
pub async fn get_document_children(
    State(db): State<Db>,
    Json(req): Json<GetChildrenReq>,
) -> Result<Json<ApiResponse<DocumentChildrenResult>>, AppError> {
    let id = req.id;
    let result = tokio::task::spawn_blocking(move || document::get_document_children(&db, &id))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

/// POST /api/v1/documents/delete
///
/// 删除文档（级联软删除所有子块）
pub async fn delete_document(
    State(db): State<Db>,
    Json(req): Json<DeleteDocumentReq>,
) -> Result<Json<ApiResponse<DeleteResult>>, AppError> {
    let operation_id = req.operation_id.clone();
    let id = req.id;
    let id_clone = id.clone();
    let result = tokio::task::spawn_blocking(move || {
        block::delete_block(&db, &id)
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
    let blk = tokio::task::spawn_blocking(move || block::create_block(&db, req))
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
    let operation_id = req.operation_id.clone();
    let id = req.id.clone();
    let blk = tokio::task::spawn_blocking(move || block::update_block(&db, &id, req))
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
        block::delete_block(&db, &id)
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

/// POST /api/v1/blocks/move
///
/// 移动 Block（改变父块和/或位置）
pub async fn move_block(
    State(db): State<Db>,
    Json(req): Json<MoveBlockReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let operation_id = req.operation_id.clone();
    let id = req.id.clone();
    let blk = tokio::task::spawn_blocking(move || block::move_block(&db, &id, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    crate::service::event::EventBus::global().emit(BlockEvent::BlockMoved {
        document_id: blk.document_id.clone(),
        operation_id,
        block: blk.clone(),
    });

    Ok(Json(ApiResponse::ok(Some(blk))))
}

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
        let restore_result = block::restore_block(&db, &id)?;
        // 在同一锁范围内查询最新状态用于广播
        let restored = block::get_block(&db, &id, false)?;
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

// ─── Split / Merge 意图 API ──────────────────────────────────

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
        block::split_block(&db, &id, req)
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
        block::merge_block(&db, &id, req)
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
    let mut receiver = crate::service::event::EventBus::global().subscribe();

    // 将 broadcast::Receiver 转为 Stream，过滤当前文档事件
    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(event) if event.document_id() == document_id => {
                    let data = serde_json::to_string(&event).unwrap_or_default();
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
// get_children (Block 子块列表) 已删除
// MVP 阶段通过 GET /documents/{id} 获取完整内容树

// ─── 文本导入/导出 API ──────────────────────────────────────────

/// POST /api/v1/blocks/import
///
/// 导入 Markdown 等格式文本，解析为 Block 树并插入数据库
pub async fn import_text(
    State(db): State<Db>,
    Json(req): Json<ImportTextReq>,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    let operation_id = req.operation_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::service::import::import_text(&db, req)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    crate::service::event::EventBus::global().emit(BlockEvent::BlockCreated {
        document_id: result.root.id.clone(),
        operation_id,
        block: result.root.clone(),
    });

    Ok(Json(ApiResponse::ok(Some(result))))
}

/// POST /api/v1/documents/export
///
/// 导出文档为 Markdown 等格式文本
pub async fn export_text(
    State(db): State<Db>,
    Json(req): Json<ExportReq>,
) -> Result<Json<ApiResponse<ExportResult>>, AppError> {
    let id = req.id;
    let format = req.format;
    let result = tokio::task::spawn_blocking(move || {
        crate::service::export::export_text(&db, &id, &format)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

// ─── 批量操作 API ──────────────────────────────────────────────

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
        block::batch_operations(&db, req)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    // 对每个成功的操作，查询其 document_id 并广播事件
    let bus = crate::service::event::EventBus::global();
    for op_result in &result.results {
        if op_result.error.is_some() { continue; }

        // 批量操作后前端统一 refetch，用 document_id="" 作为占位
        // 前端 SSE handler 会按 document_id 过滤，空字符串不会匹配
        // 因此改为逐块查询 document_id
    }

    // 简化：对整个批次发一个泛事件，前端收到后 refetch
    // 查第一个成功操作的 block_id 来确定 document_id
    let first_id = result.results.iter()
        .find(|r| r.error.is_none())
        .map(|r| r.block_id.clone());

    if let Some(first_block_id) = first_id {
        let doc_id = tokio::task::spawn_blocking(move || {
            block::get_block(&db_for_query, &first_block_id, true).ok()
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
