//! Block 数据访问层（Repository）
//!
//! 集中管理所有 blocks 表的 SQL 操作。
//! 职责：接收参数 → 执行 SQL → 返回结果。
//! 不做任何业务判断（校验、计算等由 service 层负责）。
//!
//! ## 设计原则
//! - 接收 `&Connection`（非 `&Db`），加锁由 service 层负责
//! - 返回 `Result<T, rusqlite::Error>`，错误转换由 service 层负责
//! - 函数命名遵循 `find_xxx` / `insert_xxx` / `update_xxx` 模式

use rusqlite::{params, Connection};

use crate::model::Block;

// ─── 参数结构体 ────────────────────────────────────────────────

/// INSERT 操作所需的全部字段
///
/// 将 16 个独立参数打包成一个结构体，提高可读性。
/// service 层构建此结构体后传给 `insert_block()`。
pub(crate) struct InsertBlockParams {
    pub(crate) id: String,
    pub(crate) parent_id: String,
    pub(crate) document_id: String,
    pub(crate) position: String,
    pub(crate) block_type: String,      // JSON 序列化后的 BlockType
    pub(crate) content_type: String,    // "markdown" / "empty" / "query"
    pub(crate) content: Vec<u8>,        // BLOB
    pub(crate) properties: String,      // JSON 字符串
    pub(crate) version: u64,            // 通常为 1
    pub(crate) status: String,          // "normal"
    pub(crate) schema_version: u32,     // 通常为 1
    pub(crate) author: String,          // "system"
    pub(crate) owner_id: Option<String>,
    pub(crate) encrypted: bool,         // false → 0
    pub(crate) created: String,         // ISO 8601
    pub(crate) modified: String,        // ISO 8601
}

// ─── 单行读取（Read Single）────────────────────────────────────

/// 按 ID 查询 Block（排除已删除的）
///
/// `SELECT * FROM blocks WHERE id = ? AND status != 'deleted'`
pub fn find_by_id(conn: &Connection, id: &str) -> Result<Block, rusqlite::Error> {
    conn.query_row(
        "SELECT * FROM blocks WHERE id = ?1 AND status != 'deleted'",
        [id],
        Block::from_row,
    )
}

/// 按 ID 查询 Block（不过滤状态，包含已删除的）
///
/// `SELECT * FROM blocks WHERE id = ?`
pub fn find_by_id_raw(conn: &Connection, id: &str) -> Result<Block, rusqlite::Error> {
    conn.query_row(
        "SELECT * FROM blocks WHERE id = ?1",
        [id],
        Block::from_row,
    )
}

/// 按 ID 查询已删除的 Block
///
/// `SELECT * FROM blocks WHERE id = ? AND status = 'deleted'`
pub fn find_deleted(conn: &Connection, id: &str) -> Result<Block, rusqlite::Error> {
    conn.query_row(
        "SELECT * FROM blocks WHERE id = ?1 AND status = 'deleted'",
        [id],
        Block::from_row,
    )
}

/// 检查 Block 是否存在且未删除
///
/// `SELECT EXISTS(SELECT 1 FROM blocks WHERE id = ? AND status != 'deleted')`
pub fn exists_normal(conn: &Connection, id: &str) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM blocks WHERE id = ?1 AND status != 'deleted')",
        [id],
        |row| row.get(0),
    )
}

/// 获取 Block 的状态
///
/// `SELECT status FROM blocks WHERE id = ?`
pub fn get_status(conn: &Connection, id: &str) -> Result<String, rusqlite::Error> {
    conn.query_row(
        "SELECT status FROM blocks WHERE id = ?1",
        [id],
        |row| row.get(0),
    )
}

/// 获取 Block 的版本号
///
/// `SELECT version FROM blocks WHERE id = ?`
pub fn get_version(conn: &Connection, id: &str) -> Result<u64, rusqlite::Error> {
    conn.query_row(
        "SELECT version FROM blocks WHERE id = ?1",
        [id],
        |row| row.get(0),
    )
}

/// 获取指定兄弟 Block 的 position（需同时满足 parent_id 和 status）
///
/// `SELECT position FROM blocks WHERE id = ? AND parent_id = ? AND status != 'deleted'`
pub fn get_position(
    conn: &Connection,
    id: &str,
    parent_id: &str,
) -> Result<String, rusqlite::Error> {
    conn.query_row(
        "SELECT position FROM blocks
         WHERE id = ?1 AND parent_id = ?2 AND status != 'deleted'",
        params![id, parent_id],
        |row| row.get(0),
    )
}

// ─── 多行读取（Read Multi）────────────────────────────────────

/// 列出所有根文档（id = parent_id 且 status = 'normal'）
///
/// `SELECT * FROM blocks WHERE id = parent_id AND status = 'normal' ORDER BY position ASC`
/// 查询全局根块的直接子文档（即"根文档"列表）
///
/// 根文档 = parent_id = ROOT_ID 且 block_type = Document 的 Block。
/// 不包含全局根块自身，也不包含其他类型的根级块（如段落等）。
pub fn find_root_documents(conn: &Connection) -> Result<Vec<Block>, rusqlite::Error> {
    let root_id = crate::model::ROOT_ID;
    let mut stmt = conn.prepare(
        "SELECT * FROM blocks
         WHERE parent_id = ?1 AND id != ?1 AND status = 'normal'
           AND JSON_EXTRACT(block_type, '$.type') = 'document'
         ORDER BY position ASC",
    )?;
    let blocks: Vec<Block> = stmt
        .query_map([root_id], Block::from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(blocks)
}

/// 分页列出根文档
///
/// cursor 基于 position（游标分页），传入 None 从头开始。
/// 调用方应传入 `limit + 1` 来判断是否还有更多数据。
pub fn find_root_documents_paginated(
    conn: &Connection,
    cursor: Option<&str>,
    fetch_limit: u32,
) -> Result<Vec<Block>, rusqlite::Error> {
    let root_id = crate::model::ROOT_ID;
    let blocks: Vec<Block> = if let Some(cursor) = cursor {
        let mut stmt = conn.prepare(
            "SELECT * FROM blocks
             WHERE parent_id = ?1 AND id != ?1 AND status = 'normal'
               AND JSON_EXTRACT(block_type, '$.type') = 'document'
               AND position > ?2
             ORDER BY position ASC
             LIMIT ?3",
        )?;
        stmt.query_map(params![root_id, cursor, fetch_limit], Block::from_row)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT * FROM blocks
             WHERE parent_id = ?1 AND id != ?1 AND status = 'normal'
               AND JSON_EXTRACT(block_type, '$.type') = 'document'
             ORDER BY position ASC
             LIMIT ?2",
        )?;
        stmt.query_map(params![root_id, fetch_limit], Block::from_row)?
            .filter_map(|r| r.ok())
            .collect()
    };
    Ok(blocks)
}

/// 查询 Block 的所有后代（不含自身，排除已删除）
///
/// 使用 document_id 等值查询替代递归 CTE，性能从 O(n·log n) 降为 O(n)。
/// 需要文档块和所有内容块的 document_id 正确维护。
pub fn find_descendants(
    conn: &Connection,
    document_id: &str,
) -> Result<Vec<Block>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT * FROM blocks
         WHERE document_id = ?1 AND id != ?1 AND status != 'deleted'
         ORDER BY position ASC",
    )?;
    let blocks: Vec<Block> = stmt
        .query_map([document_id], Block::from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(blocks)
}

/// 分页查询子块列表
///
/// - `cursor = None`：从头开始，`SELECT ... WHERE parent_id = ? ORDER BY position ASC LIMIT ?`
/// - `cursor = Some(pos)`：从 pos 之后开始，`... AND position > ? ...`
///
/// 调用方应传入 `limit + 1` 来判断是否还有更多数据。
pub fn find_children_paginated(
    conn: &Connection,
    parent_id: &str,
    cursor: Option<&str>,
    fetch_limit: u32,
) -> Result<Vec<Block>, rusqlite::Error> {
    let blocks: Vec<Block> = if let Some(cursor) = cursor {
        let mut stmt = conn.prepare(
            "SELECT * FROM blocks
             WHERE parent_id = ?1 AND status != 'deleted' AND position > ?2
             ORDER BY position ASC
             LIMIT ?3",
        )?;
        stmt.query_map(params![parent_id, cursor, fetch_limit], Block::from_row)?
            .filter_map(|r| r.ok())
            .collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT * FROM blocks
             WHERE parent_id = ?1 AND status != 'deleted'
             ORDER BY position ASC
             LIMIT ?2",
        )?;
        stmt.query_map(params![parent_id, fetch_limit], Block::from_row)?
            .filter_map(|r| r.ok())
            .collect()
    };
    Ok(blocks)
}

// ─── 位置查询（Position）──────────────────────────────────────

/// 获取指定父块下最大的 position（用于追加到末尾）
///
/// `SELECT MAX(position) FROM blocks WHERE parent_id = ? AND status != 'deleted'`
pub fn get_max_position(
    conn: &Connection,
    parent_id: &str,
) -> Result<Option<String>, rusqlite::Error> {
    // MAX 聚合函数始终返回一行：有数据时返回最大值，无数据时返回 NULL。
    // rusqlite 中 NULL → Ok(None)
    conn.query_row(
        "SELECT MAX(position) FROM blocks
         WHERE parent_id = ?1 AND status != 'deleted'",
        [parent_id],
        |row| row.get(0),
    )
}

/// 获取指定 position 之后的下一个兄弟 position
///
/// `SELECT position FROM blocks WHERE parent_id = ? AND status != 'deleted' AND position > ? ORDER BY position ASC LIMIT 1`
pub fn get_next_sibling_position(
    conn: &Connection,
    parent_id: &str,
    after_pos: &str,
) -> Result<Option<String>, rusqlite::Error> {
    // 没有匹配行时 query_row 返回 Err(QueryReturnedNoRow)
    // 转为 Ok(None) 表示「没有后继兄弟」
    match conn.query_row(
        "SELECT position FROM blocks
         WHERE parent_id = ?1 AND status != 'deleted' AND position > ?2
         ORDER BY position ASC LIMIT 1",
        params![parent_id, after_pos],
        |row| row.get(0),
    ) {
        Ok(pos) => Ok(Some(pos)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 获取指定 position 之前的前一个兄弟 position
///
/// `SELECT position FROM blocks WHERE parent_id = ? AND status != 'deleted' AND position < ? ORDER BY position DESC LIMIT 1`
pub fn get_prev_sibling_position(
    conn: &Connection,
    parent_id: &str,
    before_pos: &str,
) -> Result<Option<String>, rusqlite::Error> {
    match conn.query_row(
        "SELECT position FROM blocks
         WHERE parent_id = ?1 AND status != 'deleted' AND position < ?2
         ORDER BY position DESC LIMIT 1",
        params![parent_id, before_pos],
        |row| row.get(0),
    ) {
        Ok(pos) => Ok(Some(pos)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 获取指定 position 之前的前一个兄弟完整 Block
///
/// `SELECT * FROM blocks WHERE parent_id = ? AND status != 'deleted' AND position < ? ORDER BY position DESC LIMIT 1`
pub fn find_prev_sibling(
    conn: &Connection,
    parent_id: &str,
    before_pos: &str,
) -> Result<Option<Block>, rusqlite::Error> {
    match conn.query_row(
        "SELECT * FROM blocks
         WHERE parent_id = ?1 AND status != 'deleted' AND position < ?2
         ORDER BY position DESC LIMIT 1",
        params![parent_id, before_pos],
        Block::from_row,
    ) {
        Ok(block) => Ok(Some(block)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 查找同 parent 下 position 大于指定值的所有兄弟块（按 position ASC）
///
/// 用于 heading 层级自动嵌套：将 heading 之后的所有低级别块 reparent 为其子块。
pub fn find_siblings_after(
    conn: &Connection,
    parent_id: &str,
    after_pos: &str,
) -> Result<Vec<Block>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT * FROM blocks
         WHERE parent_id = ?1 AND status != 'deleted' AND position > ?2
         ORDER BY position ASC",
    )?;
    let blocks: Vec<Block> = stmt
        .query_map(params![parent_id, after_pos], Block::from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(blocks)
}

/// 查找指定块的所有直系子块（按 position ASC，不分页）
///
/// 用于 heading 降级时将子块提升为兄弟。
pub fn find_children(
    conn: &Connection,
    parent_id: &str,
) -> Result<Vec<Block>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT * FROM blocks
         WHERE parent_id = ?1 AND status != 'deleted'
         ORDER BY position ASC",
    )?;
    let blocks: Vec<Block> = stmt
        .query_map(params![parent_id], Block::from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(blocks)
}

// ─── 写入（Write）────────────────────────────────────────────

/// 插入一条 Block 记录
///
/// `INSERT INTO blocks (id, parent_id, ..., created, modified) VALUES (?, ?, ..., ?, ?)`
pub fn insert_block(conn: &Connection, p: &InsertBlockParams) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO blocks (
            id, parent_id, document_id, position,
            block_type, content_type, content, properties,
            version, status, schema_version,
            author, owner_id, encrypted,
            created, modified
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            p.id,
            p.parent_id,
            p.document_id,
            p.position,
            p.block_type,
            p.content_type,
            p.content,
            p.properties,
            p.version,
            p.status,
            p.schema_version,
            p.author,
            p.owner_id,
            p.encrypted as i32,
            p.created,
            p.modified,
        ],
    )?;
    Ok(())
}

/// 更新 Block 内容、属性、类型
///
/// `UPDATE blocks SET content=?, properties=?, block_type=?, content_type=?, modified=?, version=version+1 WHERE id=?`
///
/// 如果 `block_type` 为 None，则不更新 block_type 和 content_type 字段。
/// 返回受影响行数（0 表示不存在）
pub fn update_block_fields(
    conn: &Connection,
    id: &str,
    content: &[u8],
    properties: &str,
    block_type: Option<&str>,
    content_type: Option<&str>,
    modified: &str,
) -> Result<u64, rusqlite::Error> {
    let rows = match (block_type, content_type) {
        (Some(bt), Some(ct)) => conn.execute(
            "UPDATE blocks SET content = ?1, properties = ?2, block_type = ?3, content_type = ?4, modified = ?5, version = version + 1
             WHERE id = ?6",
            params![content, properties, bt, ct, modified, id],
        )?,
        _ => conn.execute(
            "UPDATE blocks SET content = ?1, properties = ?2, modified = ?3, version = version + 1
             WHERE id = ?4",
            params![content, properties, modified, id],
        )?,
    };
    Ok(rows as u64)
}

/// 更新 Block 内容和属性（不含 block_type）
///
/// `UPDATE blocks SET content=?, properties=?, modified=?, version=version+1 WHERE id=?`
///
/// 返回受影响行数（0 表示不存在）
pub fn update_content_and_props(
    conn: &Connection,
    id: &str,
    content: &[u8],
    properties: &str,
    modified: &str,
) -> Result<u64, rusqlite::Error> {
    let rows = conn.execute(
        "UPDATE blocks SET content = ?1, properties = ?2, modified = ?3, version = version + 1
         WHERE id = ?4",
        params![content, properties, modified, id],
    )?;
    Ok(rows as u64)
}

/// 更新 Block 状态（无条件，仅 WHERE id = ?）
///
/// `UPDATE blocks SET status=?, modified=?, version=version+1 WHERE id=?`
///
/// 返回受影响行数
pub fn update_status(
    conn: &Connection,
    id: &str,
    status: &str,
    modified: &str,
) -> Result<u64, rusqlite::Error> {
    let rows = conn.execute(
        "UPDATE blocks SET status = ?1, modified = ?2, version = version + 1
         WHERE id = ?3",
        params![status, modified, id],
    )?;
    Ok(rows as u64)
}

/// 更新 Block 状态（带 status != ? 条件，防止重复操作）
///
/// `UPDATE blocks SET status=?, modified=?, version=version+1 WHERE id=? AND status != ?`
///
/// 返回受影响行数
pub fn update_status_if_not(
    conn: &Connection,
    id: &str,
    status: &str,
    modified: &str,
    not_status: &str,
) -> Result<u64, rusqlite::Error> {
    let rows = conn.execute(
        "UPDATE blocks SET status = ?1, modified = ?2, version = version + 1
         WHERE id = ?3 AND status != ?4",
        params![status, modified, id, not_status],
    )?;
    Ok(rows as u64)
}

/// 更新 Block 的 parent_id 和 position（移动操作）
///
/// `UPDATE blocks SET parent_id=?, position=?, modified=?, version=version+1 WHERE id=?`
///
/// 返回受影响行数（0 表示不存在）
pub fn update_parent_position(
    conn: &Connection,
    id: &str,
    parent_id: &str,
    position: &str,
    modified: &str,
) -> Result<u64, rusqlite::Error> {
    let rows = conn.execute(
        "UPDATE blocks SET parent_id = ?1, position = ?2, modified = ?3, version = version + 1
         WHERE id = ?4",
        params![parent_id, position, modified, id],
    )?;
    Ok(rows as u64)
}

/// 批量更新状态（带 status 过滤条件，单条 WHERE IN）
///
/// `UPDATE blocks SET status=?, modified=?, version=version+1 WHERE id IN (...) AND status=?`
///
/// 返回成功更新的总行数
pub fn batch_update_status_if(
    conn: &Connection,
    ids: &[String],
    status: &str,
    modified: &str,
    current_status: &str,
) -> Result<u64, rusqlite::Error> {
    if ids.is_empty() {
        return Ok(0);
    }
    // 构建动态占位符: "?1, ?2, ?3, ..."
    let placeholders: Vec<&str> = (1..=ids.len()).map(|_| "?").collect();
    let sql = format!(
        "UPDATE blocks SET status = ?1, modified = ?2, version = version + 1 \
         WHERE id IN ({}) AND status = ?{}",
        placeholders.join(", "),
        ids.len() + 3
    );
    // 参数: [status, modified, id1, id2, ..., current_status]
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(ids.len() + 3);
    params_vec.push(Box::new(status.to_string()));
    params_vec.push(Box::new(modified.to_string()));
    for id in ids {
        params_vec.push(Box::new(id.clone()));
    }
    params_vec.push(Box::new(current_status.to_string()));
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = conn.execute(&sql, param_refs.as_slice())?;
    Ok(rows as u64)
}

/// 批量更新状态（带 status != ? 过滤条件，单条 WHERE IN）
///
/// `UPDATE blocks SET status=?, modified=?, version=version+1 WHERE id IN (...) AND status != ?`
///
/// 返回成功更新的总行数
pub fn batch_update_status_if_not(
    conn: &Connection,
    ids: &[String],
    status: &str,
    modified: &str,
    not_status: &str,
) -> Result<u64, rusqlite::Error> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<&str> = (1..=ids.len()).map(|_| "?").collect();
    let sql = format!(
        "UPDATE blocks SET status = ?1, modified = ?2, version = version + 1 \
         WHERE id IN ({}) AND status != ?{}",
        placeholders.join(", "),
        ids.len() + 3
    );
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(ids.len() + 3);
    params_vec.push(Box::new(status.to_string()));
    params_vec.push(Box::new(modified.to_string()));
    for id in ids {
        params_vec.push(Box::new(id.clone()));
    }
    params_vec.push(Box::new(not_status.to_string()));
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = conn.execute(&sql, param_refs.as_slice())?;
    Ok(rows as u64)
}

// ─── CTE 递归查询（Recursive）────────────────────────────────

/// 获取 Block 的所有后代 ID（不含自身，仅未删除的）
///
/// ```sql
/// WITH RECURSIVE descendants(did) AS (
///     SELECT id FROM blocks WHERE parent_id = ? AND status != 'deleted'
///     UNION ALL
///     SELECT b.id FROM blocks b
///         INNER JOIN descendants d ON b.parent_id = d.did
///     WHERE b.status != 'deleted'
/// )
/// SELECT did FROM descendants
/// ```
pub fn find_descendant_ids(
    conn: &Connection,
    block_id: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE descendants(did) AS (
            SELECT id FROM blocks WHERE parent_id = ?1 AND status != 'deleted'
            UNION ALL
            SELECT b.id FROM blocks b
                INNER JOIN descendants d ON b.parent_id = d.did
            WHERE b.status != 'deleted'
        )
        SELECT did FROM descendants",
    )?;
    let ids: Vec<String> = stmt
        .query_map([block_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// 获取 Block 的所有后代 ID（含自身，仅未删除的）
///
/// 用于删除操作：需要将自身也标记为 deleted。
///
/// ```sql
/// WITH RECURSIVE descendants(did) AS (
///     SELECT id FROM blocks WHERE id = ? AND status != 'deleted'
///     UNION ALL
///     SELECT b.id FROM blocks b
///         INNER JOIN descendants d ON b.parent_id = d.did
///     WHERE b.status != 'deleted'
/// )
/// SELECT did FROM descendants
/// ```
pub fn find_descendant_ids_include_self(
    conn: &Connection,
    block_id: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE descendants(did) AS (
            SELECT id FROM blocks WHERE id = ?1 AND status != 'deleted'
            UNION ALL
            SELECT b.id FROM blocks b
                INNER JOIN descendants d ON b.parent_id = d.did
            WHERE b.status != 'deleted'
        )
        SELECT did FROM descendants",
    )?;
    let ids: Vec<String> = stmt
        .query_map([block_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// 获取 Block 的所有已删除后代 ID（不含自身）
///
/// 用于恢复操作：找到自身 + 所有已删除的后代（恢复被级联删除的子块）。
///
/// ```sql
/// WITH RECURSIVE descendants(did) AS (
///     SELECT id FROM blocks WHERE id = ?
///     UNION ALL
///     SELECT b.id FROM blocks b
///         INNER JOIN descendants d ON b.parent_id = d.did
///     WHERE b.status = 'deleted'
/// )
/// SELECT did FROM descendants WHERE did != ?
/// ```
pub fn find_deleted_descendant_ids(
    conn: &Connection,
    block_id: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "WITH RECURSIVE descendants(did) AS (
            SELECT id FROM blocks WHERE id = ?1
            UNION ALL
            SELECT b.id FROM blocks b
                INNER JOIN descendants d ON b.parent_id = d.did
            WHERE b.status = 'deleted'
        )
        SELECT did FROM descendants WHERE did != ?1",
    )?;
    let ids: Vec<String> = stmt
        .query_map([block_id], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// 检查 target_id 是否是 block_id 的后代（循环引用检测）
///
/// ```sql
/// WITH RECURSIVE descendants(did) AS (
///     SELECT id FROM blocks WHERE parent_id = ? AND status != 'deleted'
///     UNION ALL
///     SELECT b.id FROM blocks b
///         INNER JOIN descendants d ON b.parent_id = d.did
///     WHERE b.status != 'deleted'
/// )
/// SELECT EXISTS(SELECT 1 FROM descendants WHERE did = ?)
/// ```
pub fn check_is_descendant(
    conn: &Connection,
    block_id: &str,
    target_id: &str,
) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "WITH RECURSIVE descendants(did) AS (
            SELECT id FROM blocks WHERE parent_id = ?1 AND status != 'deleted'
            UNION ALL
            SELECT b.id FROM blocks b
                INNER JOIN descendants d ON b.parent_id = d.did
            WHERE b.status != 'deleted'
        )
        SELECT EXISTS(SELECT 1 FROM descendants WHERE did = ?2)",
        params![block_id, target_id],
        |row| row.get(0),
    )
}

// ─── 单元测试 ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::tests::init_test_db;

    /// 辅助：构造一个 InsertBlockParams
    ///
    /// 提供合理的默认值，只需指定 id 和 parent_id。
    fn make_params(id: &str, parent_id: &str, document_id: &str, position: &str) -> InsertBlockParams {
        InsertBlockParams {
            id: id.to_string(),
            parent_id: parent_id.to_string(),
            document_id: document_id.to_string(),
            position: position.to_string(),
            block_type: r#"{"type":"paragraph"}"#.to_string(),
            content_type: "markdown".to_string(),
            content: b"hello".to_vec(),
            properties: "{}".to_string(),
            version: 1,
            status: "normal".to_string(),
            schema_version: 1,
            author: "system".to_string(),
            owner_id: None,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
        }
    }

    /// 辅助：构造 Document 类型的 InsertBlockParams
    fn make_doc_params(id: &str, parent_id: &str, position: &str) -> InsertBlockParams {
        InsertBlockParams {
            id: id.to_string(),
            parent_id: parent_id.to_string(),
            document_id: id.to_string(), // 文档块的 document_id 指向自身
            position: position.to_string(),
            block_type: r#"{"type":"document"}"#.to_string(),
            content_type: "markdown".to_string(),
            content: b"My Doc".to_vec(),
            properties: r#"{"title":"My Doc"}"#.to_string(),
            version: 1,
            status: "normal".to_string(),
            schema_version: 1,
            author: "system".to_string(),
            owner_id: None,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
        }
    }

    // ── 查询根块 ─────────────────────────────────────────

    #[test]
    fn find_by_id_root_block() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let root = find_by_id(&conn, crate::model::ROOT_ID).unwrap();
        assert_eq!(root.id, crate::model::ROOT_ID);
        assert_eq!(root.parent_id, crate::model::ROOT_ID);
        assert_eq!(root.position, "a0");
    }

    #[test]
    fn find_by_id_nonexistent_returns_error() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let result = find_by_id(&conn, "nonexistent000000000");
        assert!(result.is_err());
    }

    // ── exists / status / version ────────────────────────

    #[test]
    fn exists_normal_true_for_root() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        assert!(exists_normal(&conn, crate::model::ROOT_ID).unwrap());
    }

    #[test]
    fn exists_normal_false_for_nonexistent() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        assert!(!exists_normal(&conn, "nonexistent000000000").unwrap());
    }

    #[test]
    fn get_status_of_root_is_normal() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        assert_eq!(get_status(&conn, crate::model::ROOT_ID).unwrap(), "normal");
    }

    #[test]
    fn get_version_of_root_is_one() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        assert_eq!(get_version(&conn, crate::model::ROOT_ID).unwrap(), 1);
    }

    // ── insert + find_by_id ──────────────────────────────

    #[test]
    fn insert_and_find_block() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let p = make_params("test_block_000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1");
        insert_block(&conn, &p).unwrap();

        let block = find_by_id(&conn, "test_block_000001").unwrap();
        assert_eq!(block.id, "test_block_000001");
        assert_eq!(block.parent_id, crate::model::ROOT_ID);
        assert_eq!(block.position, "a1");
        assert_eq!(block.content, b"hello");
        assert_eq!(block.version, 1);
    }

    #[test]
    fn insert_duplicate_id_fails() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let p = make_params("dup_block_0000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1");
        insert_block(&conn, &p).unwrap();

        let p2 = make_params("dup_block_0000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a2");
        assert!(insert_block(&conn, &p2).is_err());
    }

    // ── find_by_id_raw (包含已删除) ──────────────────────

    #[test]
    fn find_by_id_raw_includes_deleted() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let p = make_params("raw_test_blk_00001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1");
        insert_block(&conn, &p).unwrap();

        // 软删除
        update_status(&conn, "raw_test_blk_00001", "deleted", "2026-01-02T00:00:00.000Z").unwrap();

        // find_by_id 排除已删除
        assert!(find_by_id(&conn, "raw_test_blk_00001").is_err());

        // find_by_id_raw 包含已删除
        let block = find_by_id_raw(&conn, "raw_test_blk_00001").unwrap();
        assert_eq!(block.id, "raw_test_blk_00001");
    }

    // ── find_deleted ─────────────────────────────────────

    #[test]
    fn find_deleted_returns_deleted_block() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let p = make_params("del_test_blk_00001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1");
        insert_block(&conn, &p).unwrap();
        update_status(&conn, "del_test_blk_00001", "deleted", "2026-01-02T00:00:00.000Z").unwrap();

        let block = find_deleted(&conn, "del_test_blk_00001").unwrap();
        assert_eq!(block.id, "del_test_blk_00001");
    }

    #[test]
    fn find_deleted_returns_error_for_normal() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let p = make_params("normal_test_00001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1");
        insert_block(&conn, &p).unwrap();

        assert!(find_deleted(&conn, "normal_test_00001").is_err());
    }

    // ── update_content_and_props（乐观锁）─────────────────

    #[test]
    fn update_content_and_props_success() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let p = make_params("upd_test_blk_00001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1");
        insert_block(&conn, &p).unwrap();

        let rows = update_content_and_props(
            &conn,
            "upd_test_blk_00001",
            b"updated",
            r#"{"key":"value"}"#,
            "2026-01-02T00:00:00.000Z",
        ).unwrap();
        assert_eq!(rows, 1);

        let block = find_by_id_raw(&conn, "upd_test_blk_00001").unwrap();
        assert_eq!(block.content, b"updated");
        assert_eq!(block.version, 2);
    }

    // ── update_status / update_status_if_not ──────────────

    #[test]
    fn update_status_changes_status() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let p = make_params("status_blk_000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1");
        insert_block(&conn, &p).unwrap();

        let rows = update_status(&conn, "status_blk_000001", "deleted", "2026-01-02T00:00:00.000Z").unwrap();
        assert_eq!(rows, 1);
        assert_eq!(get_status(&conn, "status_blk_000001").unwrap(), "deleted");
        assert_eq!(get_version(&conn, "status_blk_000001").unwrap(), 2);
    }

    #[test]
    fn update_status_if_not_skips_same_status() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let p = make_params("skip_blk_00000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1");
        insert_block(&conn, &p).unwrap();

        // 状态已经是 normal，再次设为 normal 应跳过
        let rows = update_status_if_not(
            &conn, "skip_blk_00000001", "normal", "2026-01-02T00:00:00.000Z", "normal",
        ).unwrap();
        assert_eq!(rows, 0); // 未更新
        assert_eq!(get_version(&conn, "skip_blk_00000001").unwrap(), 1); // 版本不变
    }

    // ── update_parent_position（移动）────────────────────

    #[test]
    fn update_parent_position_success() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        // 创建两个文档
        let doc1 = make_doc_params("move_doc_00000001", crate::model::ROOT_ID, "a1");
        let doc2 = make_doc_params("move_doc_00000002", crate::model::ROOT_ID, "a2");
        insert_block(&conn, &doc1).unwrap();
        insert_block(&conn, &doc2).unwrap();

        // 将 doc2 移动到 doc1 下
        let rows = update_parent_position(
            &conn, "move_doc_00000002", "move_doc_00000001", "a0",
            "2026-01-02T00:00:00.000Z",
        ).unwrap();
        assert_eq!(rows, 1);

        let block = find_by_id(&conn, "move_doc_00000002").unwrap();
        assert_eq!(block.parent_id, "move_doc_00000001");
        assert_eq!(block.position, "a0");
        assert_eq!(block.version, 2);
    }

    // ── position 查询 ────────────────────────────────────

    #[test]
    fn get_max_position_returns_highest() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        // 根块下已有 a0
        insert_block(&conn, &make_params("pos_blk_00000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1")).unwrap();
        insert_block(&conn, &make_params("pos_blk_00000002", crate::model::ROOT_ID, crate::model::ROOT_ID, "a2")).unwrap();

        let max = get_max_position(&conn, crate::model::ROOT_ID).unwrap();
        assert_eq!(max, Some("a2".to_string()));
    }

    #[test]
    fn get_max_position_empty_parent_returns_none() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        // 新创建的空文档（无子块）
        insert_block(&conn, &make_doc_params("empty_doc_000001", crate::model::ROOT_ID, "a1")).unwrap();

        let max = get_max_position(&conn, "empty_doc_000001").unwrap();
        assert_eq!(max, None);
    }

    #[test]
    fn test_get_next_sibling_position() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        insert_block(&conn, &make_params("sib_a_000000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1")).unwrap();
        insert_block(&conn, &make_params("sib_a_000000002", crate::model::ROOT_ID, crate::model::ROOT_ID, "a3")).unwrap();

        let next = get_next_sibling_position(&conn, crate::model::ROOT_ID, "a1").unwrap();
        assert_eq!(next, Some("a3".to_string()));

        let none = get_next_sibling_position(&conn, crate::model::ROOT_ID, "a3").unwrap();
        assert_eq!(none, None);
    }

    #[test]
    fn test_get_prev_sibling_position() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        insert_block(&conn, &make_params("sib_b_000000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1")).unwrap();
        insert_block(&conn, &make_params("sib_b_000000002", crate::model::ROOT_ID, crate::model::ROOT_ID, "a3")).unwrap();

        let prev = get_prev_sibling_position(&conn, crate::model::ROOT_ID, "a3").unwrap();
        assert_eq!(prev, Some("a1".to_string()));

        // a1 的前驱是根块 a0（根块自身 parent_id=ROOT_ID）
        let root_prev = get_prev_sibling_position(&conn, crate::model::ROOT_ID, "a1").unwrap();
        assert_eq!(root_prev, Some("a0".to_string()));

        // a0 之前没有更小的了
        let none = get_prev_sibling_position(&conn, crate::model::ROOT_ID, "a0").unwrap();
        assert_eq!(none, None);
    }

    // ── find_root_documents ──────────────────────────────

    #[test]
    fn find_root_documents_lists_only_docs() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        // 插入两个文档和一个段落
        insert_block(&conn, &make_doc_params("rootdoc_00000001", crate::model::ROOT_ID, "a1")).unwrap();
        insert_block(&conn, &make_doc_params("rootdoc_00000002", crate::model::ROOT_ID, "a3")).unwrap();
        insert_block(&conn, &make_params("para_00000000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a2")).unwrap();

        let docs = find_root_documents(&conn).unwrap();
        // 只返回 Document 类型，段落虽 parent_id 也是 ROOT_ID 但被过滤掉
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].id, "rootdoc_00000001");
        assert_eq!(docs[1].id, "rootdoc_00000002");
    }

    // ── find_descendants（递归 CTE）──────────────────────

    #[test]
    fn find_descendants_three_levels() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        // root → doc → heading → para
        insert_block(&conn, &make_doc_params("tree_doc_0000001", crate::model::ROOT_ID, "a1")).unwrap();
        insert_block(&conn, &InsertBlockParams {
            id: "tree_hdg_0000001".to_string(),
            parent_id: "tree_doc_0000001".to_string(),
            document_id: "tree_doc_0000001".to_string(),
            position: "a0".to_string(),
            block_type: r#"{"type":"heading","level":2}"#.to_string(),
            ..make_params("tree_hdg_0000001", "tree_doc_0000001", "tree_doc_0000001", "a0")
        }).unwrap();
        insert_block(&conn, &make_params("tree_para_000001", "tree_hdg_0000001", "tree_doc_0000001", "a0")).unwrap();

        let descendants = find_descendants(&conn, "tree_doc_0000001").unwrap();
        assert_eq!(descendants.len(), 2); // heading + para

        let deep_desc = find_descendants(&conn, "tree_doc_0000001").unwrap();
        assert!(deep_desc.iter().any(|b| b.id == "tree_hdg_0000001"));
        assert!(deep_desc.iter().any(|b| b.id == "tree_para_000001"));
    }

    #[test]
    fn find_descendants_empty_for_leaf() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        insert_block(&conn, &make_params("leaf_blk_0000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1")).unwrap();

        let desc = find_descendants(&conn, "leaf_blk_0000001").unwrap();
        assert!(desc.is_empty());
    }

    // ── find_children_paginated ──────────────────────────

    #[test]
    fn find_children_paginated_basic() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        // 注意：根块自身也以 parent_id=ROOT_ID 存在，position="a0"
        insert_block(&conn, &make_params("page_a_00000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1")).unwrap();
        insert_block(&conn, &make_params("page_a_00000002", crate::model::ROOT_ID, crate::model::ROOT_ID, "a3")).unwrap();
        insert_block(&conn, &make_params("page_a_00000003", crate::model::ROOT_ID, crate::model::ROOT_ID, "a5")).unwrap();

        // 取前 2 个（含根块 a0）
        let page1 = find_children_paginated(&conn, crate::model::ROOT_ID, None, 2).unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].position, "a0"); // 根块自身
        assert_eq!(page1[1].position, "a1");

        // 从 a1 之后继续取
        let page2 = find_children_paginated(&conn, crate::model::ROOT_ID, Some("a1"), 2).unwrap();
        assert_eq!(page2.len(), 2);
        assert_eq!(page2[0].position, "a3");
        assert_eq!(page2[1].position, "a5");

        // a5 之后无更多
        let page3 = find_children_paginated(&conn, crate::model::ROOT_ID, Some("a5"), 2).unwrap();
        assert!(page3.is_empty());
    }

    #[test]
    fn find_children_paginated_empty_parent() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        insert_block(&conn, &make_doc_params("empty_par_00001", crate::model::ROOT_ID, "a1")).unwrap();

        let children = find_children_paginated(&conn, "empty_par_00001", None, 10).unwrap();
        assert!(children.is_empty());
    }

    // ── find_descendant_ids_include_self ──────────────────

    #[test]
    fn find_descendant_ids_include_self_with_children() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        insert_block(&conn, &make_doc_params("des_doc_00000001", crate::model::ROOT_ID, "a1")).unwrap();
        insert_block(&conn, &make_params("des_chd_00000001", "des_doc_00000001", "des_doc_00000001", "a0")).unwrap();
        insert_block(&conn, &make_params("des_chd_00000002", "des_doc_00000001", "des_doc_00000001", "a1")).unwrap();

        let ids = find_descendant_ids_include_self(&conn, "des_doc_00000001").unwrap();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"des_doc_00000001".to_string()));
        assert!(ids.contains(&"des_chd_00000001".to_string()));
        assert!(ids.contains(&"des_chd_00000002".to_string()));
    }

    // ── find_deleted_descendant_ids ───────────────────────

    #[test]
    fn find_deleted_descendant_ids_finds_cascade_deleted() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        insert_block(&conn, &make_doc_params("dd_doc_000000001", crate::model::ROOT_ID, "a1")).unwrap();
        insert_block(&conn, &make_params("dd_chd_000000001", "dd_doc_000000001", "dd_doc_000000001", "a0")).unwrap();
        insert_block(&conn, &make_params("dd_chd_000000002", "dd_doc_000000001", "dd_doc_000000001", "a1")).unwrap();

        // 级联删除：先删除 doc，再删除 children
        update_status(&conn, "dd_doc_000000001", "deleted", "2026-01-02T00:00:00.000Z").unwrap();
        update_status(&conn, "dd_chd_000000001", "deleted", "2026-01-02T00:00:00.000Z").unwrap();
        update_status(&conn, "dd_chd_000000002", "deleted", "2026-01-02T00:00:00.000Z").unwrap();

        // 从 doc 出发找已删除的后代
        let ids = find_deleted_descendant_ids(&conn, "dd_doc_000000001").unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"dd_chd_000000001".to_string()));
        assert!(ids.contains(&"dd_chd_000000002".to_string()));
    }

    // ── check_is_descendant ──────────────────────────────

    #[test]
    fn check_is_descendant_true() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        insert_block(&conn, &make_doc_params("anc_doc_00000001", crate::model::ROOT_ID, "a1")).unwrap();
        insert_block(&conn, &make_params("anc_chd_00000001", "anc_doc_00000001", "anc_doc_00000001", "a0")).unwrap();

        assert!(check_is_descendant(&conn, "anc_doc_00000001", "anc_chd_00000001").unwrap());
    }

    #[test]
    fn check_is_descendant_false() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        insert_block(&conn, &make_doc_params("anc_doc_00000002", crate::model::ROOT_ID, "a1")).unwrap();
        insert_block(&conn, &make_params("anc_chd_00000002", "anc_doc_00000002", "anc_doc_00000002", "a0")).unwrap();

        // ROOT 不是 doc 的后代
        assert!(!check_is_descendant(&conn, "anc_doc_00000002", crate::model::ROOT_ID).unwrap());
    }

    // ── batch_update_status_if ───────────────────────────

    #[test]
    fn batch_update_status_if_bulk_restore() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        let ids = vec!["bat_a_000000001".to_string(), "bat_a_000000002".to_string()];
        // 用 a2, a3 避免与根块 a0 冲突（唯一索引 idx_blocks_parent_pos）
        for (i, id) in ids.iter().enumerate() {
            insert_block(&conn, &InsertBlockParams {
                id: id.clone(),
                status: "deleted".to_string(),
                ..make_params(id, crate::model::ROOT_ID, crate::model::ROOT_ID, &format!("a{}", i + 2))
            }).unwrap();
        }

        let total = batch_update_status_if(
            &conn, &ids, "normal", "2026-01-02T00:00:00.000Z", "deleted",
        ).unwrap();
        assert_eq!(total, 2);

        assert_eq!(get_status(&conn, "bat_a_000000001").unwrap(), "normal");
        assert_eq!(get_status(&conn, "bat_a_000000002").unwrap(), "normal");
    }

    // ── get_position ─────────────────────────────────────

    #[test]
    fn get_position_of_child() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        insert_block(&conn, &make_params("gp_blk_000000001", crate::model::ROOT_ID, crate::model::ROOT_ID, "a3")).unwrap();

        let pos = get_position(&conn, "gp_blk_000000001", crate::model::ROOT_ID).unwrap();
        assert_eq!(pos, "a3");
    }

    #[test]
    fn get_position_wrong_parent_returns_error() {
        let db = init_test_db();
        let conn = db.lock().unwrap();

        insert_block(&conn, &make_params("gp_blk_000000002", crate::model::ROOT_ID, crate::model::ROOT_ID, "a1")).unwrap();

        // 用错误的 parent_id 查询
        assert!(get_position(&conn, "gp_blk_000000002", "wrong_parent").is_err());
    }
}
