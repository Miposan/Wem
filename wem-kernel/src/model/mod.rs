//! 数据模型层
//!
//! 定义 Block、BlockType、ContentType 等核心数据结构。
//! Oplog 和 Snapshot 模型见 `oplog` 子模块。
//! 请求/响应 DTO 见 `crate::api` 模块。
//! 参考 01-block-model.md, 05-oplog.md

pub mod block;
pub mod oplog;

// 重导出常用类型，外部用 `model::Block` 即可
pub use block::{generate_block_id, Block, BlockStatus, BlockType, ContentType, ROOT_ID};
// 仅重导出外部实际通过 model:: 引用的类型
pub use oplog::ParseWarning;
