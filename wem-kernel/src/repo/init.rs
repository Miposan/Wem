//! 数据库初始化
//!
//! 负责数据库的创建、建表、迁移和种子数据。
//! 与运行时数据访问（repo/mod.rs）彻底分离。

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::AppError;
use crate::block_system::model::ROOT_ID;

use super::schema;

/// 数据库连接的线程安全包装
pub type Db = Arc<Mutex<Connection>>;

/// 获取数据库锁，自动恢复被毒化的 Mutex
///
/// 如果某个线程在持有锁期间 panic，Mutex 会被标记为"毒化"，
/// 后续 `.lock().unwrap()` 会 panic。这里恢复毒化锁以保证服务可用。
pub fn lock_db(db: &Db) -> std::sync::MutexGuard<'_, Connection> {
    match db.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("数据库 Mutex 被毒化，恢复中...");
            poisoned.into_inner()
        }
    }
}

/// 初始化文件数据库
pub fn init_db(path: &str) -> Result<Db, AppError> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Internal(format!("创建数据库目录失败: {}", e)))?;
    }

    let conn = Connection::open(path)
        .map_err(|e| AppError::Internal(format!("打开数据库失败: {}", e)))?;

    init_connection(&conn, true)?;

    println!("📦 数据库初始化完成: {}", path);
    Ok(Arc::new(Mutex::new(conn)))
}

/// 创建内存数据库
///
/// 复用 `init_connection` 的完整流程，但使用 `:memory:` SQLite。
/// 供 CLI 和测试使用。
pub fn init_memory_db() -> Db {
    let conn = Connection::open_in_memory()
        .expect("内存数据库创建失败");

    init_connection(&conn, false).expect("初始化内存数据库失败");

    Arc::new(Mutex::new(conn))
}

/// 对单个连接执行完整的初始化流程
///
/// PRAGMA → 建表 → 建索引 → 迁移 → 种子数据。
/// `enable_pragmas`: 文件数据库开启 WAL 等优化，内存数据库跳过。
fn init_connection(conn: &Connection, enable_pragmas: bool) -> Result<(), AppError> {
    if enable_pragmas {
        conn.execute_batch(schema::PRAGMAS)
            .map_err(|e| AppError::Internal(format!("设置 PRAGMA 失败: {}", e)))?;
    }

    conn.execute_batch(schema::CREATE_BLOCKS_TABLE)
        .map_err(|e| AppError::Internal(format!("建表失败: {}", e)))?;
    for idx_sql in schema::CREATE_BLOCKS_INDEXES {
        conn.execute_batch(idx_sql)
            .map_err(|e| AppError::Internal(format!("建索引失败: {}", e)))?;
    }

    if enable_pragmas {
        conn.execute_batch("ALTER TABLE batches RENAME TO operations").ok();
        conn.execute_batch("ALTER TABLE operations RENAME COLUMN operation_id TO editor_id").ok();
        conn.execute_batch("ALTER TABLE changes RENAME COLUMN batch_id TO operation_id").ok();
    }

    conn.execute_batch(schema::CREATE_OPERATIONS_TABLE)
        .map_err(|e| AppError::Internal(format!("建 operations 表失败: {}", e)))?;
    for idx_sql in schema::CREATE_OPERATIONS_INDEXES {
        conn.execute_batch(idx_sql)
            .map_err(|e| AppError::Internal(format!("建 operations 索引失败: {}", e)))?;
    }

    conn.execute_batch(schema::CREATE_CHANGES_TABLE)
        .map_err(|e| AppError::Internal(format!("建 changes 表失败: {}", e)))?;
    for idx_sql in schema::CREATE_CHANGES_INDEXES {
        conn.execute_batch(idx_sql)
            .map_err(|e| AppError::Internal(format!("建 changes 索引失败: {}", e)))?;
    }

    conn.execute_batch(schema::CREATE_SNAPSHOTS_TABLE)
        .map_err(|e| AppError::Internal(format!("建 snapshots 表失败: {}", e)))?;
    for idx_sql in schema::CREATE_SNAPSHOTS_INDEXES {
        conn.execute_batch(idx_sql)
            .map_err(|e| AppError::Internal(format!("建 snapshots 索引失败: {}", e)))?;
    }

    conn.execute_batch(schema::CREATE_AGENT_SESSIONS_TABLE)
        .map_err(|e| AppError::Internal(format!("建 agent_sessions 表失败: {}", e)))?;
    for idx_sql in schema::CREATE_AGENT_SESSIONS_INDEXES {
        conn.execute_batch(idx_sql)
            .map_err(|e| AppError::Internal(format!("建 agent_sessions 索引失败: {}", e)))?;
    }

    conn.execute_batch(schema::CREATE_AGENT_MESSAGES_TABLE)
        .map_err(|e| AppError::Internal(format!("建 agent_messages 表失败: {}", e)))?;
    for idx_sql in schema::CREATE_AGENT_MESSAGES_INDEXES {
        conn.execute_batch(idx_sql)
            .map_err(|e| AppError::Internal(format!("建 agent_messages 索引失败: {}", e)))?;
    }

    ensure_root_block(conn)?;

    Ok(())
}

/// 确保全局根块 "/" 存在（幂等）
fn ensure_root_block(conn: &Connection) -> Result<(), AppError> {
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM blocks WHERE id = ?1",
            [ROOT_ID],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if exists {
        return Ok(());
    }

    let now = crate::util::now_iso();

    conn.execute(
        "INSERT INTO blocks (
            id, parent_id, document_id, position, block_type,
            content, properties, version, status, schema_version,
            author, encrypted, created, modified
        ) VALUES (?1, ?1, ?1, ?2, ?3, X'', '{}', 1, 'normal', 1, 'system', 0, ?4, ?4)",
        rusqlite::params![
            ROOT_ID,
            "a0",
            "{\"type\":\"document\"}",
            now,
        ],
    )
    .map_err(|e| AppError::Internal(format!("创建根块失败: {}", e)))?;

    println!("🌳 全局根块 [/] 已创建 (id={})", ROOT_ID);
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn init_test_db() -> Db {
        init_memory_db()
    }
}
