//! HTTP 处理层
//!
//! Axum route handlers：解析 HTTP 请求 → 调用 service 层 → 返回 JSON 响应。
//! 所有 handler 接收 `State<Db>` 作为数据库连接。
//!
//! 参考 03-api-rest.md §1~§3

use axum::{
    extract::{Path, Query, State},
    Json,
};

use crate::api::request::{CreateBlockReq, CreateDocumentReq, ImportTextReq, MoveBlockReq, UpdateBlockReq};
use crate::api::response::{
    ChildrenResult, DeleteResult, DocumentListResponse, DocumentTreeResult,
    ExportResult, ImportResult, RestoreResult,
};
use crate::api::query::{ChildrenQuery, ExportQuery, GetBlockQuery, VersionQuery};
use crate::db::Db;
use crate::error::{AppError, ApiResponse};
use crate::model::Block;
use crate::service::block;

// ─── Health 健康检查 ────────────────────────────────────────────

/// GET /api/v1/health
pub async fn health() -> Json<ApiResponse<()>> {
    Json(ApiResponse::ok(None))
}

// ─── Root API ───────────────────────────────────────────────────

/// GET /api/v1/root
///
/// 获取全局根块 "/"（所有文档的挂载点）
pub async fn get_root(
    State(db): State<Db>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let root = tokio::task::spawn_blocking(move || block::get_root(&db))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(root))))
}

// ─── Document API ──────────────────────────────────────────────

/// POST /api/v1/documents
///
/// 创建文档（自动附带一个空段落子块）
pub async fn create_document(
    State(db): State<Db>,
    Json(req): Json<CreateDocumentReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    // spawn_blocking 将同步的 SQLite 操作移到阻塞线程池
    let title = req.title;
    let parent_id = req.parent_id;
    let after_id = req.after_id;

    let doc = tokio::task::spawn_blocking(move || {
        block::create_document(&db, title, parent_id, after_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(doc))))
}

/// GET /api/v1/documents
///
/// 列出所有根文档
pub async fn list_documents(
    State(db): State<Db>,
) -> Result<Json<ApiResponse<DocumentListResponse>>, AppError> {
    let docs = tokio::task::spawn_blocking(move || block::list_root_documents(&db))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(DocumentListResponse {
        blocks: docs,
    }))))
}

/// GET /api/v1/documents/{id}
///
/// 获取文档元数据（不含子块）
pub async fn get_document(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let doc = tokio::task::spawn_blocking(move || block::get_block(&db, &id))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(doc))))
}

/// GET /api/v1/documents/{id}/tree
///
/// 获取文档 Block 树（扁平列表）
pub async fn get_document_tree(
    State(db): State<Db>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<DocumentTreeResult>>, AppError> {
    let tree = tokio::task::spawn_blocking(move || block::get_document_tree(&db, &id))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(tree))))
}

/// DELETE /api/v1/documents/{id}?version=N
///
/// 删除文档（级联软删除所有子块）
pub async fn delete_document(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(params): Query<VersionQuery>,
) -> Result<Json<ApiResponse<DeleteResult>>, AppError> {
    let result = tokio::task::spawn_blocking(move || {
        block::delete_block(&db, &id, params.version)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

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
    let block = tokio::task::spawn_blocking(move || block::create_block(&db, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(block))))
}

/// GET /api/v1/blocks/{id}
///
/// 获取单个 Block
pub async fn get_block(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(params): Query<GetBlockQuery>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let blk = tokio::task::spawn_blocking(move || {
        block::get_block_include_deleted(&db, &id, params.include_deleted)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(blk))))
}

/// PUT /api/v1/blocks/{id}
///
/// 更新 Block 内容和/或属性
pub async fn update_block(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(req): Json<UpdateBlockReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let blk = tokio::task::spawn_blocking(move || block::update_block(&db, &id, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(blk))))
}

/// DELETE /api/v1/blocks/{id}?version=N
///
/// 软删除 Block（级联删除子块）
pub async fn delete_block(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(params): Query<VersionQuery>,
) -> Result<Json<ApiResponse<DeleteResult>>, AppError> {
    let result = tokio::task::spawn_blocking(move || {
        block::delete_block(&db, &id, params.version)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

/// POST /api/v1/blocks/{id}/move
///
/// 移动 Block（改变父块和/或位置）
pub async fn move_block(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(req): Json<MoveBlockReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let blk = tokio::task::spawn_blocking(move || block::move_block(&db, &id, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(blk))))
}

/// POST /api/v1/blocks/{id}/restore
///
/// 恢复已软删除的 Block
pub async fn restore_block(
    State(db): State<Db>,
    Path(id): Path<String>,
    Json(body): Json<VersionQuery>,
) -> Result<Json<ApiResponse<RestoreResult>>, AppError> {
    let result = tokio::task::spawn_blocking(move || {
        block::restore_block(&db, &id, body.version)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

/// GET /api/v1/blocks/{id}/children
///
/// 获取子块列表（分页）
pub async fn get_children(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(params): Query<ChildrenQuery>,
) -> Result<Json<ApiResponse<ChildrenResult>>, AppError> {
    let result = tokio::task::spawn_blocking(move || {
        block::get_children(&db, &id, params.limit, params.cursor.as_deref())
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

// ─── 文本导入/导出 API ──────────────────────────────────────────

/// POST /api/v1/blocks/import
///
/// 导入 Markdown 等格式文本，解析为 Block 树并插入数据库
pub async fn import_text(
    State(db): State<Db>,
    Json(req): Json<ImportTextReq>,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    let result = tokio::task::spawn_blocking(move || {
        crate::service::import::import_text(&db, req)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

/// GET /api/v1/documents/{id}/export?format=markdown
///
/// 导出文档为 Markdown 等格式文本
pub async fn export_text(
    State(db): State<Db>,
    Path(id): Path<String>,
    Query(params): Query<ExportQuery>,
) -> Result<Json<ApiResponse<ExportResult>>, AppError> {
    let format = params.format;
    let result = tokio::task::spawn_blocking(move || {
        crate::service::export::export_text(&db, &id, &format)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}
