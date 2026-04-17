//! 请求体 DTO
//!
//! 所有 RPC 端点的请求体类型（全 POST）。
//! 这些类型只用于 HTTP 层反序列化，不涉及数据库内部结构。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::model::{BlockType, ContentType};

// ─── 枚举类型 ──────────────────────────────────────────────────

/// 属性更新模式
///
/// - `merge`：将新属性合并到已有属性（默认）
/// - `replace`：用新属性完全替换已有属性
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PropertiesMode {
    #[default]
    Merge,
    Replace,
}

// ─── Block CRUD 请求 ──────────────────────────────────────────

/// 创建 Block 请求
///
/// `POST /api/v1/blocks`
#[derive(Debug, Deserialize)]
pub struct CreateBlockReq {
    /// 操作 ID（前端生成，用于 SSE 回声去重）
    pub operation_id: Option<String>,
    /// 父块 ID（必填）
    pub parent_id: String,
    /// Block 类型（必填）
    pub block_type: BlockType,
    /// 内容格式（可选，不传则根据 block_type 自动推断）
    pub content_type: Option<ContentType>,
    /// 块内容（可选，默认为空字符串）
    #[serde(default)]
    pub content: String,
    /// 自定义属性（可选，默认为空）
    #[serde(default)]
    pub properties: HashMap<String, String>,
    /// 插在指定 Block 之后（可选，不传则追加到末尾）
    pub after_id: Option<String>,
}

/// 更新 Block 请求
///
/// `POST /api/v1/blocks/update`
#[derive(Debug, Deserialize)]
pub struct UpdateBlockReq {
    /// 操作 ID（前端生成，用于 SSE 回声去重）
    pub operation_id: Option<String>,
    /// Block ID
    pub id: String,
    /// 新内容（不传则不更新 content）
    pub content: Option<String>,
    /// 新 Block 类型（不传则不更新 block_type）
    ///
    /// 用于 Markdown 快捷键场景：输入 `## ` 将 paragraph 变为 heading、
    /// 输入 ``` 触发 codeBlock 等。
    pub block_type: Option<BlockType>,
    /// 新属性（不传则不更新 properties）
    pub properties: Option<HashMap<String, String>>,
    /// 属性更新模式：merge（合并，默认）或 replace（替换全部）
    #[serde(default)]
    pub properties_mode: PropertiesMode,
}

/// 移动 Block 请求
///
/// `POST /api/v1/blocks/move`
#[derive(Debug, Deserialize)]
pub struct MoveBlockReq {
    /// 操作 ID（前端生成，用于 SSE 回声去重）
    pub operation_id: Option<String>,
    /// Block ID
    pub id: String,
    /// 目标父块 ID（可选）
    ///
    /// 不传时后端按优先级推导：
    /// 1. 若有 before_id/after_id → 从该兄弟块的 parent_id 推导
    /// 2. 否则保持当前父块不变
    pub target_parent_id: Option<String>,
    /// 移到此 Block 之前（可选）
    pub before_id: Option<String>,
    /// 移到此 Block 之后（可选）
    pub after_id: Option<String>,
}

// ─── Split / Merge 意图 API ──────────────────────────────────

/// 拆分 Block 请求
///
/// `POST /api/v1/blocks/split`
///
/// 前端在光标处切割文本后，将 `content_before` 和 `content_after` 发送给后端，
/// 后端原子性地完成「更新当前块 + 创建新块」两步操作。
#[derive(Debug, Deserialize)]
pub struct SplitReq {
    /// 操作 ID（前端生成，用于 SSE 回声去重）
    pub operation_id: Option<String>,
    /// Block ID
    pub id: String,
    /// 光标前的内容（用于更新当前块）
    pub content_before: String,
    /// 光标后的内容（用于创建新块）
    pub content_after: String,
    /// 新块的类型（可选，不传则默认为 paragraph）
    /// 例：在 heading 中 Enter → 新块应为 paragraph
    pub new_block_type: Option<BlockType>,
    /// 是否将新块嵌套为当前块的子块（而非兄弟）
    /// heading 的 Enter 需要将新段落作为 heading 的第一个子块
    pub nest_under_parent: Option<bool>,
}

/// 合并 Block 请求
///
/// `POST /api/v1/blocks/merge`
///
/// 将当前块的内容追加到前一个兄弟块末尾，然后删除当前块。
/// 原子操作，无需前端分别调用 update + delete。
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // direction / prev_content 预留给未来扩展
pub struct MergeReq {
    /// 操作 ID（前端生成，用于 SSE 回声去重）
    pub operation_id: Option<String>,
    /// Block ID
    pub id: String,
    /// 合并方向（预留扩展，当前仅支持 "previous"）
    #[serde(default = "default_merge_direction")]
    pub direction: String,
    /// 前一个兄弟块的当前内容（前端提供，用于校验）
    pub prev_content: Option<String>,
}

fn default_merge_direction() -> String {
    "previous".to_string()
}

// ─── Document 请求 ────────────────────────────────────────────

/// 创建文档请求体
///
/// `POST /api/v1/documents`
#[derive(Debug, Deserialize)]
pub struct CreateDocumentReq {
    /// 操作 ID（前端生成，用于 SSE 回声去重）
    pub operation_id: Option<String>,
    /// 文档标题
    pub title: String,
    /// 父文档 ID（不传 = 根文档）
    pub parent_id: Option<String>,
    /// 插在指定文档之后（可选）
    pub after_id: Option<String>,
}

// ─── 文本导入/导出请求 ────────────────────────────────────────

/// 导入文本请求
///
/// `POST /api/v1/blocks/import`
#[derive(Debug, Deserialize)]
pub struct ImportTextReq {
    /// 源格式（"markdown" 或 "md"）
    pub format: String,
    /// 文本内容
    pub content: String,
    /// 目标父块 ID（不传则挂到全局根块下，成为根文档）
    pub parent_id: Option<String>,
    /// 插在指定 Block 之后（可选，不传则追加到末尾）
    pub after_id: Option<String>,
    /// 自定义文档标题（可选，默认从内容中第一个标题推断）
    pub title: Option<String>,
}

// ─── 批量操作请求 ─────────────────────────────────────────────

/// 批量操作请求
///
/// `POST /api/v1/blocks/batch`
///
/// 单次最多 50 条操作，按数组顺序在同一事务内执行。
/// `create` 操作可指定 `temp_id`，后续操作可用 `temp_id` 引用该块。
///
/// 参考 03-api-rest.md §3 "批量操作"
#[derive(Debug, Deserialize)]
pub struct BatchReq {
    /// 操作 ID（前端生成，用于 SSE 回声去重）
    pub operation_id: Option<String>,
    /// 操作列表（上限 50 条）
    pub operations: Vec<BatchOp>,
}

/// 单条批量操作
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BatchOp {
    /// 创建新 Block
    Create {
        /// 临时 ID，后续操作可用此 ID 引用（映射到真实 ID 由内核维护）
        temp_id: String,
        /// 父块 ID（必填，可以是之前的 temp_id）
        parent_id: String,
        /// Block 类型
        block_type: BlockType,
        /// 内容格式（可选）
        content_type: Option<ContentType>,
        /// 块内容（可选）
        #[serde(default)]
        content: String,
        /// 自定义属性（可选）
        #[serde(default)]
        properties: HashMap<String, String>,
        /// 插在指定 Block 之后（可选）
        after_id: Option<String>,
    },
    /// 更新已有 Block
    Update {
        /// Block ID（可以是之前的 temp_id）
        block_id: String,
        /// 新内容（可选）
        content: Option<String>,
        /// 新属性（可选）
        properties: Option<HashMap<String, String>>,
        /// 属性更新模式
        #[serde(default)]
        properties_mode: PropertiesMode,
    },
    /// 软删除 Block
    Delete {
        /// Block ID
        block_id: String,
    },
    /// 移动 Block
    Move {
        /// Block ID
        block_id: String,
        /// 目标父块 ID（可选）
        target_parent_id: Option<String>,
        /// 移到此 Block 之前（可选）
        before_id: Option<String>,
        /// 移到此 Block 之后（可选）
        after_id: Option<String>,
    },
}

// ─── RPC 查询/只读请求（GET → POST） ─────────────────────────

/// 获取文档请求
///
/// `POST /api/v1/documents/get`
#[derive(Debug, Deserialize)]
pub struct GetDocumentReq {
    pub id: String,
}

/// 获取文档子文档请求
///
/// `POST /api/v1/documents/children`
#[derive(Debug, Deserialize)]
pub struct GetChildrenReq {
    pub id: String,
}

/// 删除文档请求
///
/// `POST /api/v1/documents/delete`
#[derive(Debug, Deserialize)]
pub struct DeleteDocumentReq {
    /// 操作 ID（前端生成，用于 SSE 回声去重）
    pub operation_id: Option<String>,
    pub id: String,
}

/// 导出文档请求
///
/// `POST /api/v1/documents/export`
#[derive(Debug, Deserialize)]
pub struct ExportReq {
    pub id: String,
    /// 目标格式（默认 "markdown"）
    #[serde(default = "default_export_format")]
    pub format: String,
}

fn default_export_format() -> String {
    "markdown".to_string()
}

/// 获取 Block 请求
///
/// `POST /api/v1/blocks/get`
#[derive(Debug, Deserialize)]
pub struct GetBlockReq {
    pub id: String,
    /// 是否包含已删除的 Block
    #[serde(default)]
    pub include_deleted: bool,
}

/// 删除 Block 请求
///
/// `POST /api/v1/blocks/delete`
#[derive(Debug, Deserialize)]
pub struct DeleteBlockReq {
    /// 操作 ID（前端生成，用于 SSE 回声去重）
    pub operation_id: Option<String>,
    pub id: String,
}

/// 恢复 Block 请求
///
/// `POST /api/v1/blocks/restore`
#[derive(Debug, Deserialize)]
pub struct RestoreReq {
    /// 操作 ID（前端生成，用于 SSE 回声去重）
    pub operation_id: Option<String>,
    pub id: String,
}

/// 获取历史记录请求
///
/// `POST /api/v1/blocks/history`
#[derive(Debug, Deserialize)]
pub struct GetHistoryReq {
    pub id: String,
    /// 返回条数（默认 50，最大 500）
    #[serde(default = "default_history_limit")]
    pub limit: u32,
}

fn default_history_limit() -> u32 {
    50
}

/// 获取指定版本请求
///
/// `POST /api/v1/blocks/version`
#[derive(Debug, Deserialize)]
pub struct GetVersionReq {
    pub id: String,
    pub version: u64,
}

/// 回滚 Block 请求
///
/// `POST /api/v1/blocks/rollback`
#[derive(Debug, Deserialize)]
pub struct RollbackReq {
    pub id: String,
    /// 回滚到的目标版本号
    pub target_version: u64,
}

/// 创建快照请求
///
/// `POST /api/v1/blocks/snapshot`
#[derive(Debug, Deserialize)]
pub struct SnapshotReq {
    pub id: String,
}
