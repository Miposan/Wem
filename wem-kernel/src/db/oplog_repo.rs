//! Oplog 与 Snapshot 数据访问层
//!
//! 集中管理 oplog 表和 snapshots 表的所有 SQL 操作。
//! 与 blocks 表的 repository 分离，职责清晰。
//!
//! 设计原则同 repository.rs：
//! - 接收 `&Connection`，加锁由 service 层负责
//! - 返回 `Result<T, rusqlite::Error>`，错误转换由 service 层负责

use rusqlite::{params, Connection};

use crate::model::oplog::{Action, Operation, Snapshot};

// ─── Oplog 写入 ────────────────────────────────────────────────

/// 插入一条 oplog 记录
///
/// `INSERT INTO oplog (block_id, action, data, prev_version, new_version, timestamp) VALUES (...)`
///
/// 返回自动生成的 op_id
pub fn insert_oplog(
    conn: &Connection,
    block_id: &str,
    action: &Action,
    data: &str,
    prev_version: u64,
    new_version: u64,
    timestamp: &str,
) -> Result<i64, rusqlite::Error> {
    conn.execute(
        "INSERT INTO oplog (block_id, action, data, prev_version, new_version, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![block_id, action.as_str(), data, prev_version, new_version, timestamp],
    )?;
    Ok(conn.last_insert_rowid())
}

// ─── Oplog 查询 ────────────────────────────────────────────────

/// 查询 Block 的变更历史
///
/// `SELECT * FROM oplog WHERE block_id = ? ORDER BY op_id DESC LIMIT ?`
pub fn find_block_history(
    conn: &Connection,
    block_id: &str,
    limit: u32,
) -> Result<Vec<Operation>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT op_id, block_id, action, data, prev_version, new_version, timestamp
         FROM oplog WHERE block_id = ?1
         ORDER BY op_id DESC
         LIMIT ?2",
    )?;
    let ops: Vec<Operation> = stmt
        .query_map(params![block_id, limit], operation_from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ops)
}

/// 查询 Block 在指定版本区间内的 oplog（用于回放）
///
/// `SELECT * FROM oplog WHERE block_id = ? AND new_version > ? AND new_version <= ? ORDER BY op_id ASC`
pub fn find_oplog_range(
    conn: &Connection,
    block_id: &str,
    from_version: u64,
    to_version: u64,
) -> Result<Vec<Operation>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT op_id, block_id, action, data, prev_version, new_version, timestamp
         FROM oplog
         WHERE block_id = ?1 AND new_version > ?2 AND new_version <= ?3
         ORDER BY op_id ASC",
    )?;
    let ops: Vec<Operation> = stmt
        .query_map(params![block_id, from_version, to_version], operation_from_row)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ops)
}

/// 获取 Block 的 oplog 总数
///
/// `SELECT COUNT(*) FROM oplog WHERE block_id = ?`
pub fn count_block_oplog(
    conn: &Connection,
    block_id: &str,
) -> Result<i64, rusqlite::Error> {
    conn.query_row(
        "SELECT COUNT(*) FROM oplog WHERE block_id = ?1",
        [block_id],
        |row| row.get(0),
    )
}

/// 获取 Block 距离上次快照以来的 oplog 数量
///
/// 通过查询 snapshots 表中最近快照的 version，
/// 然后统计 oplog 中 new_version > 该 version 的记录数。
/// 如果没有快照，则统计所有 oplog。
pub fn count_oplog_since_last_snapshot(
    conn: &Connection,
    block_id: &str,
) -> Result<i64, rusqlite::Error> {
    // 查最近快照的 version
    let last_snap_version: Option<u64> = conn
        .query_row(
            "SELECT version FROM snapshots WHERE block_id = ?1 ORDER BY version DESC LIMIT 1",
            [block_id],
            |row| row.get(0),
        )
        .ok();

    let count = match last_snap_version {
        Some(v) => conn.query_row(
            "SELECT COUNT(*) FROM oplog WHERE block_id = ?1 AND new_version > ?2",
            params![block_id, v],
            |row| row.get(0),
        )?,
        None => count_block_oplog(conn, block_id)?,
    };
    Ok(count)
}

// ─── Snapshot 写入 ─────────────────────────────────────────────

/// 插入或更新快照
///
/// 使用 `INSERT ... ON CONFLICT DO UPDATE` 保证原子性。
/// 参考 05-oplog.md §4.4
pub fn upsert_snapshot(
    conn: &Connection,
    snap: &Snapshot,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO snapshots (block_id, version, block_type, content_type, content, properties, parent_id, position, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (block_id, version) DO UPDATE SET
            block_type = excluded.block_type,
            content_type = excluded.content_type,
            content = excluded.content,
            properties = excluded.properties,
            parent_id = excluded.parent_id,
            position = excluded.position,
            timestamp = excluded.timestamp",
        params![
            snap.block_id,
            snap.version,
            snap.block_type,
            snap.content_type,
            snap.content,
            snap.properties,
            snap.parent_id,
            snap.position,
            snap.timestamp,
        ],
    )?;
    Ok(())
}

// ─── Snapshot 查询 ─────────────────────────────────────────────

/// 查询 Block <= 目标 version 的最近快照
///
/// `SELECT * FROM snapshots WHERE block_id = ? AND version <= ? ORDER BY version DESC LIMIT 1`
pub fn find_snapshot_at_or_before(
    conn: &Connection,
    block_id: &str,
    version: u64,
) -> Result<Option<Snapshot>, rusqlite::Error> {
    match conn.query_row(
        "SELECT block_id, version, block_type, content_type, content, properties, parent_id, position, timestamp
         FROM snapshots
         WHERE block_id = ?1 AND version <= ?2
         ORDER BY version DESC
         LIMIT 1",
        params![block_id, version],
        snapshot_from_row,
    ) {
        Ok(snap) => Ok(Some(snap)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 查询 Block 的最近快照
///
/// `SELECT * FROM snapshots WHERE block_id = ? ORDER BY version DESC LIMIT 1`
pub fn find_latest_snapshot(
    conn: &Connection,
    block_id: &str,
) -> Result<Option<Snapshot>, rusqlite::Error> {
    match conn.query_row(
        "SELECT block_id, version, block_type, content_type, content, properties, parent_id, position, timestamp
         FROM snapshots
         WHERE block_id = ?1
         ORDER BY version DESC
         LIMIT 1",
        [block_id],
        snapshot_from_row,
    ) {
        Ok(snap) => Ok(Some(snap)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// 获取 Block 的快照数量
///
/// `SELECT COUNT(*) FROM snapshots WHERE block_id = ?`
pub fn count_snapshots(
    conn: &Connection,
    block_id: &str,
) -> Result<i64, rusqlite::Error> {
    conn.query_row(
        "SELECT COUNT(*) FROM snapshots WHERE block_id = ?1",
        [block_id],
        |row| row.get(0),
    )
}

/// 清理旧快照，只保留每个 Block 最近 N 个
///
/// 用于 GC：每个 Block 最多保留 `keep` 个快照。
pub fn cleanup_old_snapshots(
    conn: &Connection,
    block_id: &str,
    keep: usize,
) -> Result<u64, rusqlite::Error> {
    // 找到要保留的最小 version
    let min_keep_version: Option<u64> = conn.query_row(
        "SELECT version FROM snapshots WHERE block_id = ?1 ORDER BY version DESC LIMIT 1 OFFSET ?2",
        params![block_id, keep as i64 - 1],
        |row| row.get(0),
    ).ok();

    let Some(min_ver) = min_keep_version else {
        return Ok(0); // 快照数 <= keep，无需清理
    };

    let rows = conn.execute(
        "DELETE FROM snapshots WHERE block_id = ?1 AND version < ?2",
        params![block_id, min_ver],
    )?;
    Ok(rows as u64)
}

// ─── Row 映射函数 ──────────────────────────────────────────────

/// 将一行 oplog 结果映射为 Operation 结构体
fn operation_from_row(row: &rusqlite::Row<'_>) -> Result<Operation, rusqlite::Error> {
    let action_str: String = row.get(2)?;
    let action = Action::from_str_lossy(&action_str).unwrap_or(Action::Update);

    Ok(Operation {
        op_id: row.get(0)?,
        block_id: row.get(1)?,
        action,
        data: row.get(3)?,
        prev_version: row.get(4)?,
        new_version: row.get(5)?,
        timestamp: row.get(6)?,
    })
}

/// 将一行 snapshots 结果映射为 Snapshot 结构体
fn snapshot_from_row(row: &rusqlite::Row<'_>) -> Result<Snapshot, rusqlite::Error> {
    Ok(Snapshot {
        block_id: row.get(0)?,
        version: row.get(1)?,
        block_type: row.get(2)?,
        content_type: row.get(3)?,
        content: row.get(4)?,
        properties: row.get(5)?,
        parent_id: row.get(6)?,
        position: row.get(7)?,
        timestamp: row.get(8)?,
    })
}
