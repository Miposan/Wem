//! Block 数据模型
//!
//! Block 是系统中唯一的实体。Document、Paragraph、Heading 都是 Block。
//! 这个文件定义了 Block 的完整结构、类型枚举和 ID 生成器。
//!
//! 参考 01-block-model.md §1~§6

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json;
use std::collections::HashMap;

// ─── 自定义序列化：Vec<u8> ↔ JSON 字符串 ────────────────────────

/// content 字段的序列化模块
///
/// 数据库中 content 是 BLOB（Vec<u8>），但 API 返回时需要是可读字符串。
/// MVP 阶段所有内容都是 UTF-8 文本，后续加密场景会改为 base64。
mod content_serde {
    use super::*;

    pub fn serialize<S: Serializer>(data: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        // 将 bytes 转为 UTF-8 字符串（丢失的字符用 � 替代）
        let string = String::from_utf8_lossy(data);
        s.serialize_str(&string)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        Ok(s.into_bytes())
    }
}

// ─── Block 主结构 ─────────────────────────────────────────────

/// Block — 系统中唯一的实体
///
/// 每个字段对应 blocks 表中的一列。
/// 所有 Block（文档、段落、标题、列表……）都用同一个结构。
///
/// ```
/// Block {
///     id: "20260402103000123Ab5",   // 20 位 ID
///     parent_id: "20260402103000...", // 父块
///     block_type: Paragraph,          // 类型
///     content: "Hello world",         // 内容
///     position: "a1",                 // 排序位置
///     ...
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// 20 位 Block ID（14 位时间戳 + 3 位毫秒 + 3 位随机）
    pub id: String,
    /// 父块 ID（Document 根节点指向自身）
    pub parent_id: String,
    /// 所属文档 ID（文档块指向自身，内容块指向文档根块）
    pub document_id: String,
    /// Fractional Index 排序位置（字典序字符串）
    pub position: String,
    /// Block 类型（Paragraph、Heading、Document 等）
    pub block_type: BlockType,
    /// 内容格式（Markdown / Empty / Query）
    pub content_type: ContentType,
    /// 块内容（格式取决于 content_type）
    #[serde(with = "content_serde", default)]
    pub content: Vec<u8>,
    /// 自定义属性（JSON key-value）
    #[serde(default)]
    pub properties: HashMap<String, String>,
    /// 乐观锁版本号（每次更新 +1）
    pub version: u64,
    /// 状态：normal / draft / deleted
    pub status: BlockStatus,
    /// 格式版本号（支持未来格式迁移）
    pub schema_version: u32,
    /// 是否加密
    pub encrypted: bool,
    /// 创建时间（ISO 8601 格式）
    pub created: String,
    /// 最后修改时间（ISO 8601 格式）
    pub modified: String,
    /// 创建者（不可变，格式：user:{id} / agent:{id}:{name} / system）
    pub author: String,
    /// 当前所有者 user_id（可通过 chown 转让，MVP 阶段为 None）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
}

impl Block {
    /// 从数据库行构造 Block
    ///
    /// 查询必须使用 `SELECT *` 或包含全部 16 个字段。
    /// 枚举类型（BlockType、ContentType、BlockStatus）会从字符串自动解析。
    pub fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        // 需要特殊转换的字段先读取原始字符串
        let block_type_str: String = row.get("block_type")?;
        let content_type_str: String = row.get("content_type")?;
        let status_str: String = row.get("status")?;

        Ok(Block {
            id: row.get("id")?,
            parent_id: row.get("parent_id")?,
            document_id: row.get("document_id")?,
            position: row.get("position")?,
            block_type: serde_json::from_str(&block_type_str)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e)))?,
            content_type: ContentType::from_str(&content_type_str).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, content_type_str)),
                )
            })?,
            content: row.get("content")?,
            properties: {
                let json: String = row.get("properties")?;
                serde_json::from_str(&json).unwrap_or_default()
            },
            version: row.get("version")?,
            status: BlockStatus::from_str(&status_str).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, status_str)),
                )
            })?,
            schema_version: row.get("schema_version")?,
            encrypted: row.get::<_, i32>("encrypted")? != 0,
            created: row.get("created")?,
            modified: row.get("modified")?,
            author: row.get("author")?,
            owner_id: row.get("owner_id")?,
        })
    }
}

// ─── BlockType 枚举 ──────────────────────────────────────────

/// Block 类型
///
/// 分为四类：
/// - **容器块**：包含子 Block（Document、Heading、Blockquote 等）
/// - **叶子块**：有文本内容（Paragraph、CodeBlock、MathBlock、ThematicBreak）
/// - **资源块**：有 URL（Image、Audio、Video、Iframe）
/// - **特殊块**：Embed、AttributeView、Widget
///
/// 序列化为 JSON 格式，用 `type` 字段区分：
/// ```json
/// {"type": "paragraph"}
/// {"type": "heading", "level": 2}
/// {"type": "codeBlock", "language": "rust"}
/// {"type": "image", "url": "assets/abc.png"}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BlockType {
    // ─── 容器块（包含子 Block，content 为空）─────────────────
    /// 文档（根节点），可以有子块，content 存标题文本
    Document,
    /// 标题，level: 1-6，子块为该标题下的内容
    Heading { level: u8 },
    /// 引用块
    Blockquote,
    /// 列表，ordered 表示有序/无序
    List { ordered: bool },
    /// 列表项
    ListItem,
    /// 提示块（Callout）
    Callout,

    // ─── 叶子块（有文本内容）──────────────────────────────
    /// 段落（最常用的块类型）
    Paragraph,
    /// 代码块，language 指定语言
    CodeBlock { language: String },
    /// 数学公式块
    MathBlock,
    /// 分割线
    ThematicBreak,

    // ─── 资源块（有 URL，无文本内容）─────────────────────────
    /// 图片
    Image { url: String },
    /// 音频
    Audio { url: String },
    /// 视频
    Video { url: String },
    /// 嵌入网页
    Iframe { url: String },

    // ─── 特殊块（未来扩展）────────────────────────────────
    /// 嵌入查询块，content 存查询语句
    Embed,
    /// 数据库/表格视图
    AttributeView { av_id: String },
    /// 自定义组件块
    Widget,
}

impl BlockType {
    /// 根据 BlockType 推断默认 ContentType
    ///
    /// - Document + 叶子块 → Markdown
    /// - 容器块（除 Document）+ 资源块 → Empty
    /// - Embed → Query
    pub fn default_content_type(&self) -> ContentType {
        match self {
            Self::Document
            | Self::Paragraph
            | Self::CodeBlock { .. }
            | Self::MathBlock => ContentType::Markdown,
            Self::ThematicBreak => ContentType::Empty,
            Self::Embed => ContentType::Query,
            _ => ContentType::Empty,
        }
    }

}

// ─── ContentType 枚举 ─────────────────────────────────────────

/// 内容格式
///
/// - Markdown：叶子块和 Document 的文本内容（wem Markdown 方言）
/// - Empty：容器块（除 Document 外）和资源块均无文本内容
/// - Query：Embed 块，content 存查询语句
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Markdown,
    Empty,
    Query,
}

impl ContentType {
    /// 转为数据库存储字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Empty => "empty",
            Self::Query => "query",
        }
    }

    /// 从数据库字符串解析
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "markdown" => Some(Self::Markdown),
            "empty" => Some(Self::Empty),
            "query" => Some(Self::Query),
            _ => None,
        }
    }
}

// ─── BlockStatus 枚举 ────────────────────────────────────────

/// Block 状态
///
/// 状态流转：
/// - Normal → Deleted（用户删除）
/// - Deleted → Normal（用户恢复）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlockStatus {
    #[default]
    Normal,
    Deleted,
}

impl BlockStatus {
    /// 转为数据库存储字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Deleted => "deleted",
        }
    }

    /// 从数据库字符串解析
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "normal" => Some(Self::Normal),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

// ─── ID 生成器 ────────────────────────────────────────────────

/// 生成 20 位 Block ID
///
/// 格式：`YYYYMMDDHHmmss`（14 位）+ 毫秒（3 位）+ 随机字符（3 位）= 20 位
///
/// 示例：`20260402103000123Ab5`
/// - `20260402103000`：2026-04-02 10:30:00
/// - `123`：毫秒部分
/// - `Ab5`：3 位随机字母数字
///
/// 参考 01-block-model.md §2
pub fn generate_block_id() -> String {
    use chrono::Utc;
    use rand::Rng;

    let now = Utc::now();
    let ts = now.format("%Y%m%d%H%M%S").to_string(); // 14 位时间戳
    let ms = (now.timestamp_millis() % 1000) as u32; // 毫秒部分

    // 3 位随机字母数字字符
    let rand_chars: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(3)
        .map(char::from)
        .collect();

    format!("{}{:03}{}", ts, ms, rand_chars) // 20 位
}

// ─── 全局常量 ────────────────────────────────────────────────

/// 全局根块的固定 ID
///
/// 根块 "/" 是所有文档的唯一挂载点，类似于文件系统的根目录。
/// - `id = ROOT_ID`, `parent_id = ROOT_ID`
/// - 系统初始化时自动创建，不可删除
/// - 所有根文档（无显式 parent_id 的文档）都挂在这个根块下
pub const ROOT_ID: &str = "00000000000000000000";

// ─── 解析警告 ─────────────────────────────────────────────────

/// 解析过程中产生的警告
///
/// 定义在 model 层，供 parser 产出、api 层引用。
#[derive(Debug, Clone, Serialize)]
pub struct ParseWarning {
    /// 行号（1-based，0 表示未知）
    pub line: usize,
    /// 警告类型标识
    pub warning_type: String,
    /// 人类可读描述
    pub message: String,
    /// 采取的操作（如 `"auto_fixed"`）
    pub action: String,
}
