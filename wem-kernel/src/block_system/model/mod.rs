//! 数据模型层
//!
//! 定义 Block、BlockType 等核心数据结构。
//! 操作日志模型见 `oplog` 子模块。
//! 请求/响应 DTO 见 `crate::dto` 模块。

pub mod block;
pub mod event;
pub mod oplog;

// 重导出常用类型，外部用 `model::Block` 即可
pub use block::{generate_block_id, Block, BlockStatus, BlockType, ParseWarning, ROOT_ID};
