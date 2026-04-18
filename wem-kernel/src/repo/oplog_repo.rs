//! 操作日志数据访问层
//!
//! 集中管理 batches 表和 changes 表的所有 SQL 操作。
//! 采用 batch-based 操作日志架构：
//! - 每次用户操作 = 一个 Batch
//! - 每个 Batch 包含 N 个 Change（每个受影响 Block 一条）
//! - undo/redo 通过标记 batch.undone 实现无损操作

use rusqlite::{params, Connection};

use crate::model::oplog::{
    Action, Batch, BlockSnapshot, Change, ChangeType,
};

// ─── Batch 写入 ────────────────────────────────────────────────

/// 插入一条 Batch 记录
///
/// 返回自动生成的 id（UUID v7，由调用方生成）
pub fn insert_batch(
    conn: &Connection,
    batch: &Batch,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO batches (id, document_id, action, description, timestamp, undone)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            batch.id,
            batch.document_id,
            batch.action.as_str(),
            batch.description,
            batch.timestamp,
            batch.undone as i32,
        ],
    )?;
    Ok(())
}

// ─── Change 写入 ───────────────────────────────────────────────

/// 批量插入 Change 记录
///
/// 在一个事务内插入一个 Batch 的所有 Change。
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
            "INSERT INTO changes (batch_id, block_id, change_type, before_data, after_data)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                change.batch_id,
                change.block_id,
                change.change_type.as_str(),
                before_json,
                after_json,
            ],
        )?;
    }
    Ok(())
}

// ─── Batch 查询 ────────────────────────────────────────────────

/// 查找指定文档下最近一个可撤销的 Batch（undone = 0）
///
/// undo 操作使用：找到最近的未撤销 batch，标记为 undone。
pub fn find_latest_undoable_batch(
    conn: &Connection,
    document_id: &str,
) -> Result<Option<Batch>, rusqlite::Error> {
    match conn.query_row(
        "SELECT id, document_id, action, description, timestamp, undone
         FROM batches
         WHERE document_id = ?1 AND undone = 0
         ORDER BY timestamp DESC
         LIMIT 1",
        params![document_id],
        batch_from_row,
    ) {
        Ok(batch) => Ok(Some(batch)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 查找指定文档下最近一个可重做的 Batch（undone = 1）
///
/// redo 操作使用：找到最近的已撤销 batch，标记为未撤销。
pub fn find_latest_redoable_batch(
    conn: &Connection,
    document_id: &str,
) -> Result<Option<Batch>, rusqlite::Error> {
    match conn.query_row(
        "SELECT id, document_id, action, description, timestamp, undone
         FROM batches
         WHERE document_id = ?1 AND undone = 1
         ORDER BY timestamp DESC
         LIMIT 1",
        params![document_id],
        batch_from_row,
    ) {
        Ok(batch) => Ok(Some(batch)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 标记 Batch 为已撤销/已重做
///
/// `UPDATE batches SET undone = ? WHERE id = ?`
pub fn set_batch_undone(
    conn: &Connection,
    batch_id: &str,
    undone: bool,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE batches SET undone = ?1 WHERE id = ?2",
        params![undone as i32, batch_id],
    )?;
    Ok(())
}

/// 查询指定文档的 Batch 列表（分页）
///
/// 按 timestamp DESC 排序，支持 limit + offset 分页。
pub fn find_batches(
    conn: &Connection,
    document_id: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<Batch>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, document_id, action, description, timestamp, undone
         FROM batches
         WHERE document_id = ?1
         ORDER BY timestamp DESC
         LIMIT ?2 OFFSET ?3",
    )?;
    let batches: Vec<Batch> = stmt
        .query_map(params![document_id, limit, offset], batch_from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(batches)
}

// ─── Change 查询 ───────────────────────────────────────────────

/// 查询 Batch 的所有 Change
///
/// undo/redo 核心操作：获取 batch 下所有变更记录。
pub fn find_batch_changes(
    conn: &Connection,
    batch_id: &str,
) -> Result<Vec<Change>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, batch_id, block_id, change_type, before_data, after_data
         FROM changes
         WHERE batch_id = ?1
         ORDER BY id",
    )?;
    let changes: Vec<Change> = stmt
        .query_map(params![batch_id], change_from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(changes)
}

/// 查询 Block 的变更历史（跨 Batch）
///
/// 按 change.id DESC 排序，支持 limit 分页。
pub fn find_block_history(
    conn: &Connection,
    block_id: &str,
    limit: u32,
) -> Result<Vec<Change>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, batch_id, block_id, change_type, before_data, after_data
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

/// 清理旧的已撤销 Batch 及其 Change
///
/// 保留最近 `keep` 个 batch（不论是否已撤销），删除更早的。
/// 通常在定时任务中调用。
pub fn cleanup_old_batches(
    conn: &Connection,
    keep: usize,
) -> Result<u64, rusqlite::Error> {
    // 找到要保留的最小 timestamp
    let min_keep_ts: Option<String> = conn
        .query_row(
            "SELECT timestamp FROM batches ORDER BY timestamp DESC LIMIT 1 OFFSET ?1",
            params![keep as i64 - 1],
            |row| row.get(0),
        )
        .ok();

    let Some(min_ts) = min_keep_ts else {
        return Ok(0); // batch 数 <= keep，无需清理
    };

    // changes 通过 FOREIGN KEY ON DELETE CASCADE 自动删除
    let rows = conn.execute(
        "DELETE FROM batches WHERE timestamp < ?1",
        params![min_ts],
    )?;
    Ok(rows as u64)
}

// ─── Row 映射函数 ──────────────────────────────────────────────

fn batch_from_row(row: &rusqlite::Row<'_>) -> Result<Batch, rusqlite::Error> {
    let action_str: String = row.get(2)?;
    let action = Action::from_str_lossy(&action_str)
        .ok_or_else(|| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(std::io::Error::other(format!("unknown action: {action_str}")))))?;

    Ok(Batch {
        id: row.get(0)?,
        document_id: row.get(1)?,
        action,
        description: row.get(3)?,
        timestamp: row.get(4)?,
        undone: row.get::<_, i32>(5)? != 0,
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
        batch_id: row.get(1)?,
        block_id: row.get(2)?,
        change_type,
        before,
        after,
    })
}
