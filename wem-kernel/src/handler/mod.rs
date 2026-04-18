//! HTTP 处理层
//!
//! Axum route handlers，解析 HTTP 请求 → 调用 service 层 → 返回响应。
//!
//! 模块划分（按路由前缀对齐）：
//! - `document` — `/api/v1/documents/*`  文档 CRUD + 导入/导出 + 跨文档嫁接
//! - `block`    — `/api/v1/blocks/*`     Block CRUD + 移动 + 恢复 + 拆分/合并 + 批量
//! - `oplog`    — `/api/v1/documents/*`  操作日志查询/Undo/Redo（per-document）
//! - `event`    — `/api/v1/health`       健康检查 + SSE 实时事件

pub mod block;
pub mod document;
pub mod event;
pub mod oplog;

// 统一重导出，供 main.rs 路由注册使用
pub use document::{
    create_document, list_documents, get_document, get_document_children, delete_document,
    move_document_tree, import_text, export_text,
};
pub use block::{
    create_block, get_block, update_block, delete_block, move_block, move_heading_tree,
    restore_block, split_block, merge_block, batch_blocks,
};
pub use event::{health, document_events};
pub use oplog::{get_block_history, undo, redo};
