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

    // 建表（IF NOT EXISTS：新库才生效，旧库保持原样）
    conn.execute_batch(schema::CREATE_BLOCKS_TABLE)
        .map_err(|e| AppError::Internal(format!("建表失败: {}", e)))?;

    // 补列：旧库可能缺少新增字段，逐个尝试添加，已存在则跳过
    apply_schema_patches(&conn);

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

/// 补列：对已有数据库尝试添加新增字段
///
/// 每次 DDL 新增字段时，在这里追加一条 `ALTER TABLE ... ADD COLUMN`。
/// SQLite 不支持 `IF NOT EXISTS` 语法用于列，所以用 "try and ignore" 策略——
/// 列已存在时 SQLite 会报错 `duplicate column name`，我们静默跳过即可。
///
/// 新建的数据库不会走到这里（`CREATE TABLE IF NOT EXISTS` 已经包含全部字段），
/// 但执行一下也无害。
fn apply_schema_patches(conn: &Connection) {
    // ---- document_id (v2) ----
    // 所属文档 ID：文档块指向自身，内容块指向文档根块
    let _ = conn.execute_batch(
        "ALTER TABLE blocks ADD COLUMN document_id TEXT NOT NULL DEFAULT '';"
    );
    // 回填：让已有数据拥有合理的 document_id
    // 1. 根块 → 指向自身
    let _ = conn.execute(
        "UPDATE blocks SET document_id = id WHERE id = ?1 AND document_id = ''",
        [ROOT_ID],
    );
    // 2. 文档块（parent_id = ROOT_ID 的 document 类型）→ 指向自身
    let _ = conn.execute_batch(
        "UPDATE blocks SET document_id = id \
         WHERE parent_id = '/' AND id != '/' AND document_id = '' \
         AND json_extract(block_type, '$.type') = 'document';"
    );
    // 3. 内容块 → 指向其所属文档（沿 parent_id 链找到文档块）
    let _ = conn.execute_batch(
        "UPDATE blocks SET document_id = parent_id \
         WHERE document_id = '' AND parent_id != '/';"
    );

    // ---- 未来新增字段在此追加 ----
    // let _ = conn.execute_batch(
    //     "ALTER TABLE blocks ADD COLUMN new_field TEXT NOT NULL DEFAULT '';"
    // );
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
            id, parent_id, document_id, position, block_type, content_type,
            content, properties, version, status, schema_version,
            author, encrypted, created, modified
        ) VALUES (?1, ?1, ?1, ?2, ?3, ?4, X'', '{}', 1, 'normal', 1, 'system', 0, ?5, ?5)",
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
