//! 文本格式转换模块
//!
//! 提供 [`BlockParser`] / [`BlockSerializer`] trait 和工厂函数，
//! 实现 Markdown 等文本格式与 Block 树之间的双向转换。
//!
//! ## 扩展方式
//! 1. 创建 `src/parser/<format>.rs`
//! 2. 让格式类型同时实现 [`BlockParser`] + [`BlockSerializer`]
//! 3. 在工厂函数中注册新格式
//!
//! 参考：`Note/15-markdown-parser.md`

pub mod markdown;
pub mod types;

use std::collections::HashMap;

use crate::error::AppError;
use crate::block_system::model::Block;

use types::{ParseOptions, ParseResult, SerializeResult};

// ─── BlockParser trait ───────────────────────────────────────

/// 解析器 trait —— 文本 → Block 树
pub trait BlockParser {
    /// 将文本解析为 Block 树
    fn parse(&self, text: &str, options: &ParseOptions) -> Result<ParseResult, AppError>;
}

// ─── BlockSerializer trait ───────────────────────────────────

/// 序列化器 trait —— Block 树 → 文本
pub trait BlockSerializer {
    /// 将 Block 树序列化为文本
    fn serialize(
        &self,
        root: &Block,
        children_map: &HashMap<String, Vec<Block>>,
    ) -> Result<SerializeResult, AppError>;
}

// ─── 工厂函数 ─────────────────────────────────────────────────

/// 根据格式名称获取对应的解析器
///
/// ```rust,ignore
/// let parser = get_parser("markdown")?;
/// let result = parser.parse("# Hello", &ParseOptions::default())?;
/// ```
pub fn get_parser(format: &str) -> Result<Box<dyn BlockParser>, AppError> {
    match format {
        "markdown" | "md" => Ok(Box::new(markdown::MarkdownFormat::new())),
        _ => Err(AppError::BadRequest(format!(
            "不支持的源格式: '{}'. 当前支持: markdown",
            format
        ))),
    }
}

/// 根据格式名称获取对应的序列化器
///
/// ```rust,ignore
/// let serializer = get_serializer("markdown")?;
/// let result = serializer.serialize(&root, &children_map)?;
/// ```
pub fn get_serializer(format: &str) -> Result<Box<dyn BlockSerializer>, AppError> {
    match format {
        "markdown" | "md" => Ok(Box::new(markdown::MarkdownFormat::new())),
        _ => Err(AppError::BadRequest(format!(
            "不支持的目标格式: '{}'. 当前支持: markdown",
            format
        ))),
    }
}
