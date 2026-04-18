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
    pub document_id: String,
    pub version: u64,
    pub cascade_count: u32,
}

/// 恢复 Block 响应
///
/// `POST /api/v1/blocks/{id}/restore`
#[derive(Debug, Serialize)]
pub struct RestoreResult {
    pub id: String,
    pub document_id: String,
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

/// 文档直系子文档响应（侧边栏导航用）
///
/// `GET /api/v1/documents/{id}/children`
///
/// 只返回该文档的直系子文档列表（一层），用于侧边栏按需加载。
/// 用户展开某个子文档时，再请求该子文档的 /children 获取下一层。
#[derive(Debug, Serialize)]
pub struct DocumentChildrenResult {
    /// 直系子文档列表（仅 document 类型，按 position ASC 排序）
    pub children: Vec<Block>,
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

// ─── Split / Merge 意图 API 响应 ────────────────────────────

/// 拆分 Block 响应
///
/// `POST /api/v1/blocks/{id}/split`
///
/// 返回更新后的原块和新创建的块。
/// 前端可直接用这两个 Block 替换 UI 中的原块 + 插入新块。
#[derive(Debug, Serialize)]
pub struct SplitResult {
    /// 更新后的原块（content = content_before）
    pub updated_block: Block,
    /// 新创建的块（content = content_after，位于原块之后）
    pub new_block: Block,
}

/// 合并 Block 响应
///
/// `POST /api/v1/blocks/{id}/merge`
///
/// 返回合并后的块和被删除的块 ID。
/// 前端可直接用 merged_block 替换前一个块，并移除 deleted_block_id 对应的 DOM。
#[derive(Debug, Serialize)]
pub struct MergeResult {
    /// 合并后的前一个兄弟块（content = prev.content + current.content）
    pub merged_block: Block,
    /// 被删除的块 ID
    pub deleted_block_id: String,
}

// ─── Oplog 操作响应 ────────────────────────────────────────────

/// Block 变更历史响应
///
/// `POST /api/v1/documents/history`
#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub entries: Vec<crate::model::oplog::HistoryEntry>,
}

/// Undo/Redo 响应
///
/// `POST /api/v1/undo` / `POST /api/v1/redo`
#[derive(Debug, Serialize)]
pub struct UndoRedoResponse {
    pub result: crate::model::oplog::UndoRedoResult,
}
