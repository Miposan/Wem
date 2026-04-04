//! 响应体 DTO
//!
//! 所有读操作的响应类型。
//! 这些类型用于 service 层返回结构化结果，handler 层包装为 `ApiResponse<T>`。

use std::collections::HashMap;

use serde::Serialize;

use crate::model::{Block, ParseWarning};

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

/// Block 嵌套节点（渲染树）
///
/// 将 Block 及其子节点以树形结构返回，前端可直接递归渲染。
#[derive(Debug, Serialize)]
pub struct BlockNode {
    #[serde(flatten)]
    pub block: Block,
    /// 子节点（容器块才有子节点，叶子块为空数组）
    pub children: Vec<BlockNode>,
}

/// 文档内容响应（编辑器用）
///
/// `GET /api/v1/documents/{id}`
///
/// 返回文档块 + 嵌套的内容块树（paragraph、heading、codeBlock 等），
/// 用于编辑器直接递归渲染文档正文。不含子文档。
#[derive(Debug, Serialize)]
pub struct DocumentContentResult {
    /// 文档块本身（包含文档元信息和标题）
    pub document: Block,
    /// 嵌套的内容块树（非 document 类型的后代，按 position 排序）
    pub blocks: Vec<BlockNode>,
    /// 是否有更多 Block 未返回（大文档截断场景）
    pub has_more: bool,
}

/// 文档树子节点响应（侧边栏导航用）
///
/// `GET /api/v1/documents/{id}/tree`
///
/// 只返回该文档的直系子文档列表（一层），用于侧边栏按需加载。
/// 用户展开某个子文档时，再请求该子文档的 /tree 获取下一层。
#[derive(Debug, Serialize)]
pub struct DocumentTreeResult {
    /// 文档块本身
    pub document: Block,
    /// 直系子文档列表（仅 document 类型，按 position ASC 排序）
    pub children: Vec<Block>,
}

/// 文档列表响应
///
/// `GET /api/v1/documents`
#[derive(Debug, Serialize)]
pub struct DocumentListResponse {
    pub blocks: Vec<Block>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
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

/// 批量操作响应
///
/// `POST /api/v1/blocks/batch`
#[derive(Debug, Serialize)]
pub struct BatchResult {
    /// 临时 ID → 真实 ID 的映射（仅 create 操作产生）
    pub id_map: HashMap<String, String>,
    /// 每条操作的执行结果
    pub results: Vec<BatchOpResult>,
}

/// 单条批量操作结果
#[derive(Debug, Serialize)]
pub struct BatchOpResult {
    /// 操作类型：create / update / delete / move
    pub action: String,
    /// 涉及的 Block ID（create 时为真实 ID）
    pub block_id: String,
    /// 成功后的版本号
    pub version: Option<u64>,
    /// 错误信息（成功时为 None）
    pub error: Option<String>,
}

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

// ─── Oplog 操作响应 ────────────────────────────────────────────

/// Block 变更历史响应
///
/// `GET /api/v1/blocks/{id}/history`
#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub entries: Vec<crate::model::oplog::HistoryEntry>,
}

/// 版本内容响应
///
/// `GET /api/v1/blocks/{id}/versions/{version}`
#[derive(Debug, Serialize)]
pub struct VersionResponse {
    pub version: crate::model::oplog::VersionContent,
}

/// 回滚响应
///
/// `POST /api/v1/blocks/{id}/rollback`
#[derive(Debug, Serialize)]
pub struct RollbackResponse {
    pub result: crate::model::oplog::RollbackResult,
}

/// 快照创建响应
///
/// `POST /api/v1/blocks/{id}/snapshot`
#[derive(Debug, Serialize)]
pub struct SnapshotResponse {
    pub result: crate::model::oplog::SnapshotResult,
}
