//! Block System — 自包含的笔记/块操作系统
//!
//! 一个完整的块级笔记系统，包含从数据模型到 HTTP 接口的所有层次：
//!
//! - `model`   — 数据模型（Block, BlockType, Event, Oplog）
//! - `parser`  — 文本解析/序列化（Markdown ↔ Block 树）
//! - `service` — 业务逻辑（CRUD, 事务, 位置计算, undo/redo）
//! - `handler` — HTTP 路由处理层

pub mod model;
pub mod parser;
pub mod service;
pub mod handler;

// ─── 便捷 re-export（向后兼容） ─────────────────────────────────
pub use model::{Block, BlockStatus, BlockType, ParseWarning, ROOT_ID, generate_block_id};
pub use service::block;
pub use service::document;
pub use service::event;
pub use service::oplog;
pub use service::position;
pub use service::traits::ExportDepth;
