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
    /// 新属性（不传则不更新 properties）
    pub properties: Option<HashMap<String, String>>,
    /// 属性更新模式："merge"（合并，默认）或 "replace"（替换全部）
    #[serde(default = "default_properties_mode")]
    pub properties_mode: String,
    /// 乐观锁版本号（必填，必须与服务器当前 version 匹配）
    pub version: u64,
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
    /// 乐观锁版本号（必填）
    pub version: u64,
}

fn default_properties_mode() -> String {
    "merge".to_string()
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
