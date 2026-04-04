//! 数据库层
//!
//! SQLite 连接管理、建表 DDL、PRAGMA 配置。
//! 使用 `Arc<Mutex<Connection>>` 作为简单的线程安全包装（MVP 方案）。

pub mod oplog_repo;
pub mod block_repo;
mod schema;

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::AppError;

/// 重导出根块 ID（定义在 model 层）
pub use crate::model::ROOT_ID;

/// 数据库连接的线程安全包装
///
/// - `Arc`：允许多个 Axum handler 共享同一个连接
/// - `Mutex`：保证同一时刻只有一个线程在操作数据库
///
/// 后续可升级为连接池（如 r2d2-sqlite）。
pub type Db = Arc<Mutex<Connection>>;

/// 初始化数据库
///
/// 1. 确保数据库文件目录存在（如 `wem-data/`）
/// 2. 打开/创建 SQLite 文件
/// 3. 执行 PRAGMA 优化配置（WAL、缓存等）
/// 4. 执行建表 DDL 和索引
/// 5. 确保全局根块 "/" 存在（幂等）
///
/// 返回包装好的 `Db`，可注入到 Axum State 中。
pub fn init_db(path: &str) -> Result<Db, AppError> {
    // 确保父目录存在（如 wem-data/）
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Internal(format!("创建数据库目录失败: {}", e)))?;
    }

    // 打开数据库连接（如果文件不存在会自动创建）
    let conn = Connection::open(path)
        .map_err(|e| AppError::Internal(format!("打开数据库失败: {}", e)))?;

    // 执行 PRAGMA 配置
    conn.execute_batch(schema::PRAGMAS)
        .map_err(|e| AppError::Internal(format!("设置 PRAGMA 失败: {}", e)))?;

    // 建表
    conn.execute_batch(schema::CREATE_BLOCKS_TABLE)
        .map_err(|e| AppError::Internal(format!("建表失败: {}", e)))?;

    // 建索引
    for idx_sql in schema::CREATE_BLOCKS_INDEXES {
        conn.execute_batch(idx_sql)
            .map_err(|e| AppError::Internal(format!("建索引失败: {}", e)))?;
    }

    // 建 oplog 表 + 索引
    conn.execute_batch(schema::CREATE_OPLOG_TABLE)
        .map_err(|e| AppError::Internal(format!("建 oplog 表失败: {}", e)))?;
    for idx_sql in schema::CREATE_OPLOG_INDEXES {
        conn.execute_batch(idx_sql)
            .map_err(|e| AppError::Internal(format!("建 oplog 索引失败: {}", e)))?;
    }

    // 建 snapshots 表 + 索引
    conn.execute_batch(schema::CREATE_SNAPSHOTS_TABLE)
        .map_err(|e| AppError::Internal(format!("建 snapshots 表失败: {}", e)))?;
    for idx_sql in schema::CREATE_SNAPSHOTS_INDEXES {
        conn.execute_batch(idx_sql)
            .map_err(|e| AppError::Internal(format!("建 snapshots 索引失败: {}", e)))?;
    }

    // 确保全局根块存在（幂等：如已存在则跳过）
    ensure_root_block(&conn)?;

    println!("📦 数据库初始化完成: {}", path);
    Ok(Arc::new(Mutex::new(conn)))
}

/// 确保全局根块 "/" 存在
///
/// 根块是所有文档的挂载点，类似文件系统的根目录。
/// 首次初始化时创建，后续启动检测到已存在则跳过。
fn ensure_root_block(conn: &Connection) -> Result<(), AppError> {
    // 检查根块是否已存在
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

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    conn.execute(
        "INSERT INTO blocks (
            id, parent_id, position, block_type, content_type,
            content, properties, version, status, schema_version,
            author, encrypted, created, modified
        ) VALUES (?1, ?1, ?2, ?3, ?4, X'', '{}', 1, 'normal', 1, 'system', 0, ?5, ?5)",
        rusqlite::params![
            ROOT_ID,
            "a0",                                    // position: 第一个
            "{\"type\":\"document\"}",                // block_type: Document
            "markdown",                               // content_type
            now,
        ],
    )
    .map_err(|e| AppError::Internal(format!("创建根块失败: {}", e)))?;

    println!("🌳 全局根块 [/] 已创建 (id={})", ROOT_ID);
    Ok(())
}

// ─── 测试基础设施 ──────────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 创建内存数据库（用于单元测试）
    ///
    /// 复刻 `init_db` 的完整流程，但使用 `:memory:` SQLite：
    /// 1. 打开内存连接
    /// 2. 执行建表 DDL + 索引
    /// 3. 插入全局根块
    ///
    /// 每次调用返回独立的数据库实例，测试间互不干扰。
    /// 整个过程在微秒级完成，无需清理。
    pub fn init_test_db() -> Db {
        let conn = Connection::open_in_memory()
            .expect("内存数据库创建失败");

        conn.execute_batch(schema::CREATE_BLOCKS_TABLE)
            .expect("建表失败");

        for idx_sql in schema::CREATE_BLOCKS_INDEXES {
            conn.execute_batch(idx_sql)
                .expect("建索引失败");
        }

        ensure_root_block(&conn).expect("创建根块失败");

        Arc::new(Mutex::new(conn))
    }
}
