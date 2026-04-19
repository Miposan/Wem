//! Block System — 块操作的统一子系统
//!
//! 提供块结构的完整生命周期管理。
//!
//! 架构分层：
//! - block.rs    — 通用操作 + BlockTypeOps trait + 类型分派 + 导入/导出
//! - heading.rs  — Heading 类型的 BlockTypeOps 实现
//! - document.rs — Document 类型的 BlockTypeOps 实现
//! - paragraph.rs — Paragraph 类型的 BlockTypeOps 实现 + split/merge
//! - position.rs — Fractional Index 位置计算
//! - oplog.rs    — 操作日志
//! - event.rs    — 块变更事件通知

pub mod block;
pub mod document;
pub mod event;
pub mod heading;
pub mod oplog;
pub mod paragraph;
pub mod position;
