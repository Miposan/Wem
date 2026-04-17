//! 请求体 DTO
//!
//! 所有写操作（POST/PUT/DELETE）的请求体类型。
//! 这些类型只用于 HTTP 层反序列化，不涉及数据库内部结构。

use serde::Deserialize;
use std::collections::HashMap;

use crate::model::{BlockType, ContentType};

// ─── Block CRUD 请求 ──────────────────────────────────────────

/// 创建 Block 请求
///
/// `POST /api/v1/blocks`
#[derive(Debug, Deserialize)]
pub struct CreateBlockReq {
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
/// `PUT /api/v1/blocks/{id}`
#[derive(Debug, Deserialize)]
pub struct UpdateBlockReq {
    /// 新内容（不传则不更新 content）
    pub content: Option<String>,
    /// 新 Block 类型（不传则不更新 block_type）
    ///
    /// 用于 Markdown 快捷键场景：输入 `## ` 将 paragraph 变为 heading、
    /// 输入 ``` 触发 codeBlock 等。
    pub block_type: Option<BlockType>,
    /// 新属性（不传则不更新 properties）
    pub properties: Option<HashMap<String, String>>,
    /// 属性更新模式："merge"（合并，默认）或 "replace"（替换全部）
    #[serde(default = "default_properties_mode")]
    pub properties_mode: String,
}

/// 移动 Block 请求
///
/// `POST /api/v1/blocks/{id}/move`
#[derive(Debug, Deserialize)]
pub struct MoveBlockReq {
    /// 目标父块 ID（可选，不传则不改变父块）
    pub target_parent_id: Option<String>,
    /// 移到此 Block 之前（可选）
    pub before_id: Option<String>,
    /// 移到此 Block 之后（可选）
    pub after_id: Option<String>,
}

fn default_properties_mode() -> String {
    "merge".to_string()
}

// ─── Split / Merge 意图 API ──────────────────────────────────

/// 拆分 Block 请求
///
/// `POST /api/v1/blocks/{id}/split`
///
/// 前端在光标处切割文本后，将 `content_before` 和 `content_after` 发送给后端，
/// 后端原子性地完成「更新当前块 + 创建新块」两步操作。
#[derive(Debug, Deserialize)]
pub struct SplitReq {
    /// 光标前的内容（用于更新当前块）
    pub content_before: String,
    /// 光标后的内容（用于创建新块）
    pub content_after: String,
    /// 新块的类型（可选，不传则默认为 paragraph）
    /// 例：在 heading 中 Enter → 新块应为 paragraph
    pub new_block_type: Option<BlockType>,
}

/// 合并 Block 请求
///
/// `POST /api/v1/blocks/{id}/merge`
///
/// 将当前块的内容追加到前一个兄弟块末尾，然后删除当前块。
/// 原子操作，无需前端分别调用 update + delete。
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // direction / prev_content 预留给未来扩展
pub struct MergeReq {
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
        #[serde(default = "default_properties_mode")]
        properties_mode: String,
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
