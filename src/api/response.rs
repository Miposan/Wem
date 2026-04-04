//! 响应体 DTO
//!
//! 所有读操作的响应类型。
//! 这些类型用于 service 层返回结构化结果，handler 层包装为 `ApiResponse<T>`。

use serde::Serialize;

use crate::model::Block;

// ─── Block 操作响应 ────────────────────────────────────────────

/// 删除 Block 响应
///
/// `DELETE /api/v1/blocks/{id}` 和 `DELETE /api/v1/documents/{id}`
#[derive(Debug, Serialize)]
pub struct DeleteResult {
    pub id: String,
    pub version: u64,
    pub cascade_count: u32,
}

/// 恢复 Block 响应
///
/// `POST /api/v1/blocks/{id}/restore`
#[derive(Debug, Serialize)]
pub struct RestoreResult {
    pub id: String,
    pub version: u64,
    pub cascade_count: u32,
}

// ─── Document 操作响应 ────────────────────────────────────────

/// 文档树响应
///
/// `GET /api/v1/documents/{id}/tree`
#[derive(Debug, Serialize)]
pub struct DocumentTreeResult {
    pub root: Block,
    pub blocks: Vec<Block>,
}

/// 文档列表响应
///
/// `GET /api/v1/documents`
#[derive(Debug, Serialize)]
pub struct DocumentListResponse {
    pub blocks: Vec<Block>,
}

// ─── 子块查询响应 ─────────────────────────────────────────────

/// 子块列表响应（分页）
///
/// `GET /api/v1/blocks/{id}/children`
#[derive(Debug, Serialize)]
pub struct ChildrenResult {
    pub blocks: Vec<Block>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}
