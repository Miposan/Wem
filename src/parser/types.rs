//! Parser 共享类型
//!
//! 定义所有解析器/序列化器共用的输入选项、警告、降级信息和结果类型。

use serde::{Deserialize, Serialize};
use crate::model::Block;

// ─── 解析选项 ─────────────────────────────────────────────────

/// 解析器通用选项（预留扩展）
///
/// 目前无实际字段，保留结构体以便将来添加格式特定的选项。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ParseOptions {}

// ─── 解析警告 ─────────────────────────────────────────────────

/// 解析过程中产生的警告
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

// ─── 解析结果 ─────────────────────────────────────────────────

/// 解析器输出
#[derive(Debug, Clone)]
pub struct ParseResult {
    /// 根 Block（Document 或第一个 Block）
    pub root: Block,
    /// 所有后代 Block（扁平列表，按树遍历顺序）
    pub children: Vec<Block>,
    /// 解析过程中的警告
    pub warnings: Vec<ParseWarning>,
    /// 创建的 Block 总数（root + children）
    pub blocks_created: usize,
}

// ─── 序列化降级信息 ───────────────────────────────────────────

/// 记录序列化过程中的降级处理
///
/// 某些 [`BlockType`](crate::model::BlockType) 在目标格式中无原生对应，会做降级处理。
#[derive(Debug, Clone, Serialize)]
pub struct LossyInfo {
    /// 被降级的 BlockType 名称
    pub block_type: String,
    /// 降级方式描述
    pub reason: String,
}

// ─── 序列化结果 ──────────────────────────────────────────────

/// 序列化器输出
#[derive(Debug, Clone)]
pub struct SerializeResult {
    /// 序列化后的文本内容
    pub content: String,
    /// 推荐文件名
    pub filename: Option<String>,
    /// 导出的 Block 数量
    pub blocks_exported: usize,
    /// 降级处理的 BlockType 列表
    pub lossy_types: Vec<String>,
}
