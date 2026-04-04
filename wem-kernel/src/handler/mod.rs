//! HTTP 处理层
//!
//! Axum route handlers，解析 HTTP 请求 → 调用 service 层 → 返回响应。

pub mod block;
pub mod oplog;

// 重导出 handler 函数，供 main.rs 路由注册使用
pub use block::{
    health, get_root, create_document, list_documents, get_document, get_document_tree, delete_document,
    create_block, get_block, update_block, delete_block, move_block, restore_block, get_children,
    import_text, export_text, batch_blocks,
};

// 重导出 oplog handler 函数
pub use oplog::{
    get_block_history, get_block_version, rollback_block,
    create_snapshot as create_block_snapshot,
};
