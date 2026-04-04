//! 数据模型层
//!
//! 定义 Block、BlockType、ContentType 等核心数据结构。
//! 请求/响应 DTO 见 `crate::api` 模块。
//! 参考 01-block-model.md

pub mod block;

// 重导出常用类型，外部用 `model::Block` 即可
pub use block::{generate_block_id, Block, BlockStatus, BlockType, ContentType};
