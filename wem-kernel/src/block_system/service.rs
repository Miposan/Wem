//! Block System — 业务逻辑层
//!
//! 核心分层：
//! - `traits`    — trait 定义 + 类型（L1 基础设施）
//! - `helpers`   — 事务 + 工具函数（L1 基础设施）
//! - `heading`   — Heading 类型的 BlockTypeOps 实现（L2 类型特化）
//! - `document`  — Document 类型的 BlockTypeOps 实现 + 文档编排（L2 类型特化）
//! - `paragraph` — Paragraph 类型的 BlockTypeOps 实现 + split/merge（L2 类型特化）
//! - `block`     — 通用 CRUD + Move + 分派 + re-export（L3 整合层）
//! - `position`  — Fractional Index 位置计算
//! - `oplog`     — 操作日志
//! - `event`     — 块变更事件通知
//! - `batch`     — 批量操作

pub mod batch;
pub mod block;
pub mod document;
pub mod event;
pub mod heading;
pub mod list;
pub mod oplog;
pub mod paragraph;
pub mod position;
pub mod traits;

pub(crate) mod helpers;
