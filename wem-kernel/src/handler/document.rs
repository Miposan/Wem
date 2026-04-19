//! Document HTTP 处理层
//!
//! 文档级操作：CRUD + 导入/导出 + 跨文档嫁接。
//! 所有路由前缀：`/api/v1/documents/*`。

use axum::{extract::State, Json};

use crate::api::request::{
    CreateDocumentReq, DeleteDocumentReq, ExportReq, GetChildrenReq, GetDocumentReq,
    ImportTextReq, MoveDocumentTreeReq,
};
use crate::api::response::{
    DeleteResult, DocumentChildrenResult, DocumentContentResult, ExportResult, ImportResult,
};
use crate::error::{AppError, ApiResponse};
use crate::model::Block;
use crate::repo::Db;
use crate::service::block_system::{block, document};

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
    let title = req.title;
    let parent_id = req.parent_id;
    let after_id = req.after_id;
    let editor_id = req.editor_id;

    let doc = tokio::task::spawn_blocking(move || {
        document::create_document(&db, title, parent_id, after_id, editor_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

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
    let id = req.id;
    let result = tokio::task::spawn_blocking(move || {
        block::delete_tree(&db, &id, req.editor_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}

// ─── Move Document Tree ────────────────────────────────────────

/// POST /api/v1/documents/move-document-tree
///
/// 移动 Document 子树（跨文档嫁接）
pub async fn move_document_tree(
    State(db): State<Db>,
    Json(req): Json<MoveDocumentTreeReq>,
) -> Result<Json<ApiResponse<Block>>, AppError> {
    let blk = tokio::task::spawn_blocking(move || document::move_document_tree(&db, req))
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(blk))))
}

// ─── Import / Export ───────────────────────────────────────────

/// POST /api/v1/documents/import
///
/// 导入 Markdown 等格式文本，解析为 Block 树并插入数据库
pub async fn import_text(
    State(db): State<Db>,
    Json(req): Json<ImportTextReq>,
) -> Result<Json<ApiResponse<ImportResult>>, AppError> {
    let result = tokio::task::spawn_blocking(move || {
        crate::service::document::import_text(&db, req)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

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
        crate::service::document::export_text(&db, &id, &format)
    })
    .await
    .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;

    Ok(Json(ApiResponse::ok(Some(result))))
}
