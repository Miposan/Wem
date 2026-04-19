//! 业务逻辑层
//!
//! 块系统（block_system）提供块结构的完整生命周期管理。

pub mod block_system;

// Re-export block_system 子模块（保持旧路径可用，渐进式迁移）
pub use block_system::event;
pub use block_system::oplog;
pub use block_system::position;

// Re-export content 和 document 的公共 API（handler 层使用）
pub use block_system::block;
pub use block_system::document;
