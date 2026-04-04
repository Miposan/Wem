//! 响应体 DTO
//!
//! 所有读操作的响应类型。
//! 这些类型用于 service 层返回结构化结果，handler 层包装为 `ApiResponse<T>`。

use serde::Serialize;

use crate::model::Block;
use crate::parser::types::ParseWarning;

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

// ─── 导入/导出响应 ────────────────────────────────────────────

/// 导入文本响应
///
/// `POST /api/v1/blocks/import`
#[derive(Debug, Serialize)]
pub struct ImportResult {
    /// 导入后创建的文档根 Block
    pub root: Block,
    /// 创建的 Block 总数（含根文档）
    pub blocks_imported: usize,
    /// 解析过程中的警告
    pub warnings: Vec<ParseWarning>,
}

/// 导出文本响应
///
/// `GET /api/v1/documents/{id}/export?format=markdown`
#[derive(Debug, Serialize)]
pub struct ExportResult {
    /// 序列化后的文本内容
    pub content: String,
    /// 推荐文件名（如 `"Hello.md"`）
    pub filename: Option<String>,
    /// 导出的 Block 数量
    pub blocks_exported: usize,
    /// 降级处理的 BlockType 列表
    pub lossy_types: Vec<String>,
}
