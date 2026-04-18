//! 操作日志数据访问层
//!
//! 集中管理 operations 表和 changes 表的所有 SQL 操作。
//! 采用 Operation-based 操作日志架构：
//! - 每次用户操作 = 一个 Operation
//! - 每个 Operation 包含 N 个 Change（每个受影响 Block 一条）
//! - undo/redo 通过标记 operation.undone 实现无损操作

use rusqlite::{params, Connection};

use crate::model::oplog::{
    Action, Operation, BlockSnapshot, Change, ChangeType,
};

// ─── Operation 写入 ────────────────────────────────────────────

/// 插入一条 Operation 记录
pub fn insert_operation(
    conn: &Connection,
    op: &Operation,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO operations (id, document_id, action, description, timestamp, undone, editor_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            op.id,
            op.document_id,
            op.action.as_str(),
            op.description,
            op.timestamp,
            op.undone as i32,
            op.editor_id,
        ],
    )?;
    Ok(())
}

// ─── Change 写入 ───────────────────────────────────────────────

/// 批量插入 Change 记录
///
/// 在一个事务内插入一个 Operation 的所有 Change。
/// 调用方应确保已在事务中。
pub fn insert_changes(
    conn: &Connection,
    changes: &[Change],
) -> Result<(), rusqlite::Error> {
    for change in changes {
        let before_json = change
            .before
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap_or_default());
        let after_json = change
            .after
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap_or_default());

        conn.execute(
            "INSERT INTO changes (operation_id, block_id, change_type, before_data, after_data)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                change.operation_id,
                change.block_id,
                change.change_type.as_str(),
                before_json,
                after_json,
            ],
        )?;
    }
    Ok(())
}

// ─── Operation 查询 ────────────────────────────────────────────

/// 查找指定文档下最近一个可撤销的 Operation（undone = 0）
pub fn find_latest_undoable_operation(
    conn: &Connection,
    document_id: &str,
) -> Result<Option<Operation>, rusqlite::Error> {
    match conn.query_row(
        "SELECT id, document_id, action, description, timestamp, undone, editor_id
         FROM operations
         WHERE document_id = ?1 AND undone = 0
         ORDER BY timestamp DESC
         LIMIT 1",
        params![document_id],
        operation_from_row,
    ) {
        Ok(op) => Ok(Some(op)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 查找指定文档下最近一个可重做的 Operation（undone = 1）
pub fn find_latest_redoable_operation(
    conn: &Connection,
    document_id: &str,
) -> Result<Option<Operation>, rusqlite::Error> {
    match conn.query_row(
        "SELECT id, document_id, action, description, timestamp, undone, editor_id
         FROM operations
         WHERE document_id = ?1 AND undone = 1
         ORDER BY timestamp ASC
         LIMIT 1",
        params![document_id],
        operation_from_row,
    ) {
        Ok(op) => Ok(Some(op)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 标记 Operation 为已撤销/已重做
pub fn set_operation_undone(
    conn: &Connection,
    operation_id: &str,
    undone: bool,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE operations SET undone = ?1 WHERE id = ?2",
        params![undone as i32, operation_id],
    )?;
    Ok(())
}

/// 查询指定文档的 Operation 列表（分页）
pub fn find_operations(
    conn: &Connection,
    document_id: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<Operation>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, document_id, action, description, timestamp, undone, editor_id
         FROM operations
         WHERE document_id = ?1
         ORDER BY timestamp DESC
         LIMIT ?2 OFFSET ?3",
    )?;
    let ops: Vec<Operation> = stmt
        .query_map(params![document_id, limit, offset], operation_from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ops)
}

// ─── Change 查询 ───────────────────────────────────────────────

/// 查询 Operation 的所有 Change
pub fn find_operation_changes(
    conn: &Connection,
    operation_id: &str,
) -> Result<Vec<Change>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, operation_id, block_id, change_type, before_data, after_data
         FROM changes
         WHERE operation_id = ?1
         ORDER BY id",
    )?;
    let changes: Vec<Change> = stmt
        .query_map(params![operation_id], change_from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(changes)
}

/// 查询 Block 的变更历史（跨 Operation）
pub fn find_block_history(
    conn: &Connection,
    block_id: &str,
    limit: u32,
) -> Result<Vec<Change>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, operation_id, block_id, change_type, before_data, after_data
         FROM changes
         WHERE block_id = ?1
         ORDER BY id DESC
         LIMIT ?2",
    )?;
    let changes: Vec<Change> = stmt
        .query_map(params![block_id, limit], change_from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(changes)
}

// ─── GC 清理 ───────────────────────────────────────────────────

/// 清理旧的 Operation 及其 Change
///
/// 保留最近 `keep` 个 operation（不论是否已撤销），删除更早的。
pub fn cleanup_old_operations(
    conn: &Connection,
    keep: usize,
) -> Result<u64, rusqlite::Error> {
    let min_keep_ts: Option<String> = conn
        .query_row(
            "SELECT timestamp FROM operations ORDER BY timestamp DESC LIMIT 1 OFFSET ?1",
            params![keep as i64 - 1],
            |row| row.get(0),
        )
        .ok();

    let Some(min_ts) = min_keep_ts else {
        return Ok(0);
    };

    // changes 通过 FOREIGN KEY ON DELETE CASCADE 自动删除
    let rows = conn.execute(
        "DELETE FROM operations WHERE timestamp < ?1",
        params![min_ts],
    )?;
    Ok(rows as u64)
}

// ─── Row 映射函数 ──────────────────────────────────────────────

fn operation_from_row(row: &rusqlite::Row<'_>) -> Result<Operation, rusqlite::Error> {
    let action_str: String = row.get(2)?;
    let action = Action::from_str_lossy(&action_str)
        .ok_or_else(|| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(std::io::Error::other(format!("unknown action: {action_str}")))))?;

    Ok(Operation {
        id: row.get(0)?,
        document_id: row.get(1)?,
        action,
        description: row.get(3)?,
        timestamp: row.get(4)?,
        undone: row.get::<_, i32>(5)? != 0,
        editor_id: row.get(6)?,
    })
}

fn change_from_row(row: &rusqlite::Row<'_>) -> Result<Change, rusqlite::Error> {
    let change_type_str: String = row.get(3)?;
    let change_type = ChangeType::from_str_lossy(&change_type_str)
        .ok_or_else(|| rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(std::io::Error::other(format!("unknown change_type: {change_type_str}")))))?;

    let before_json: Option<String> = row.get(4)?;
    let after_json: Option<String> = row.get(5)?;

    let before = before_json
        .and_then(|json| serde_json::from_str::<BlockSnapshot>(&json).ok());
    let after = after_json
        .and_then(|json| serde_json::from_str::<BlockSnapshot>(&json).ok());

    Ok(Change {
        id: row.get(0)?,
        operation_id: row.get(1)?,
        block_id: row.get(2)?,
        change_type,
        before,
        after,
    })
}
