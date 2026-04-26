//! 数据库运行时基础设施
//!
//! 提供 Db 类型别名和锁管理。
//! 初始化逻辑在 repo/init.rs，持久化操作在 repo/block_repo.rs / repo/oplog_repo.rs。

pub mod block_repo;
pub(crate) mod init;
pub mod oplog_repo;
pub mod session_repo;
mod schema;

pub use init::{Db, init_db, init_memory_db, lock_db};

#[cfg(test)]
pub(crate) mod tests {
    pub use crate::repo::init::tests::init_test_db;
}
