//! 数据库运行时基础设施
//!
//! 提供 Db 类型别名和锁管理。
//! 初始化逻辑在 repo/init.rs，持久化操作在 repo/block_repo.rs / repo/oplog_repo.rs。

pub mod block_repo;
pub(crate) mod init;
pub mod oplog_repo;
mod schema;

pub use init::{Db, init_db, init_memory_db};

#[cfg(test)]
pub(crate) mod tests {
    pub use crate::repo::init::tests::init_test_db;
}

use rusqlite::Connection;

use crate::repo::init::Db as DbInner;

/// 获取数据库锁，自动恢复被毒化的 Mutex
///
/// 如果某个线程在持有锁期间 panic，Mutex 会被标记为"毒化"，
/// 后续 `.lock().unwrap()` 会 panic。这里恢复毒化锁以保证服务可用。
pub fn lock_db(db: &DbInner) -> std::sync::MutexGuard<'_, Connection> {
    match db.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("数据库 Mutex 被毒化，恢复中...");
            poisoned.into_inner()
        }
    }
}
