//! 操作日志（Oplog）业务逻辑层
//!
//! 基于 batch-based 操作日志，为文档提供 undo/redo 能力。
//!
//! 提供：
//! - `record_batch()` — 每次 Block 操作后记录 batch + changes（含自动 GC）
//! - `undo()` — 撤销最近一次操作（恢复 before 快照）
//! - `redo()` — 重做最近被撤销的操作（恢复 after 快照）
//! - `get_history()` — 查询操作日志（分页）
//! - `get_block_history()` — 查询单个 Block 的变更日志

use crate::error::AppError;
use crate::model::oplog::{
    Action, Batch, BlockSnapshot, Change, ChangeSummary, ChangeType, HistoryEntry, UndoRedoResult,
};
use crate::model::Block;
use crate::repo::{block_repo, oplog_repo, Db};
use crate::util::now_iso;

/// Batch 容量上限：只保留最近 N 次操作记录
///
/// 超出此上限后，`record_batch()` 会自动清理最老的 batch。
/// changes 表通过 FOREIGN KEY ON DELETE CASCADE 自动级联删除。
pub const MAX_BATCHES: usize = 1000;

// ─── 记录操作 ──────────────────────────────────────────────────

/// 记录一次 Batch 操作
///
/// 在 Block CRUD 操作成功后、同一个数据库锁内调用。
/// 自动写入 Batch 和所有 Change 记录，并在超出容量上限时清理最老的 batch。
///
/// # 参数
/// - `conn`: 已持有的数据库连接（调用方负责加锁）
/// - `batch`: Batch 元数据（id、action、description、timestamp）
/// - `changes`: 变更列表（每个受影响 Block 一条）
pub fn record_batch(
    conn: &rusqlite::Connection,
    batch: &Batch,
    changes: &[Change],
) -> Result<(), AppError> {
    // 写入 Batch
    oplog_repo::insert_batch(conn, batch)
        .map_err(|e| AppError::Internal(format!("写入 batch 失败: {}", e)))?;

    // 写入 Changes
    if !changes.is_empty() {
        oplog_repo::insert_changes(conn, changes)
            .map_err(|e| AppError::Internal(format!("写入 changes 失败: {}", e)))?;
    }

    // 自动 GC：超出容量上限时清理最老的 batch
    gc_batches(conn)?;

    Ok(())
}

/// 批次容量 GC
///
/// 当 batch 总量超过 `MAX_BATCHES` 时，删除最老的 batch。
/// changes 通过 FOREIGN KEY ON DELETE CASCADE 自动级联删除。
///
/// 注意：当前 GC 仍然是全局清理（不按文档），
/// 因为 document-scoped GC 需要额外索引且收益有限。
fn gc_batches(conn: &rusqlite::Connection) -> Result<(), AppError> {
    // 全局统计：这里不按 document_id 过滤
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM batches", [], |row| row.get(0))
        .map_err(|e| AppError::Internal(format!("统计 batch 数量失败: {}", e)))?;

    if count as usize > MAX_BATCHES {
        oplog_repo::cleanup_old_batches(conn, MAX_BATCHES)
            .map_err(|e| AppError::Internal(format!("GC batch 失败: {}", e)))?;
    }

    Ok(())
}

// ─── Undo ──────────────────────────────────────────────────────

/// 撤销最近一次操作
///
/// 流程：
/// 1. 找到最近的 undone = false 的 Batch
/// 2. 获取该 Batch 的所有 Change
/// 3. 对每个 Change，恢复 before 快照到 blocks 表
/// 4. 标记 Batch 为 undone = true
///
/// 返回受影响的 Block ID 和 Document ID（用于 SSE 广播）
pub fn undo(db: &Db, document_id: &str) -> Result<UndoRedoResult, AppError> {
    let conn = crate::repo::lock_db(db);

    // 1. 找到指定文档最近的未撤销 Batch
    let batch = oplog_repo::find_latest_undoable_batch(&conn, document_id)
        .map_err(|e| AppError::Internal(format!("查询可撤销 batch 失败: {}", e)))?
        .ok_or_else(|| AppError::BadRequest("没有可撤销的操作".to_string()))?;

    // 2. 获取所有 Change
    let changes = oplog_repo::find_batch_changes(&conn, &batch.id)
        .map_err(|e| AppError::Internal(format!("查询 batch changes 失败: {}", e)))?;

    // 3. 恢复 before 快照
    let (affected_block_ids, affected_document_ids) =
        restore_snapshots(&conn, &changes, true)?;

    // 4. 标记为已撤销
    oplog_repo::set_batch_undone(&conn, &batch.id, true)
        .map_err(|e| AppError::Internal(format!("标记 batch undone 失败: {}", e)))?;

    Ok(UndoRedoResult {
        batch_id: batch.id,
        affected_block_ids,
        affected_document_ids,
        action: batch.action.as_str().to_string(),
    })
}

// ─── Redo ──────────────────────────────────────────────────────

/// 重做最近被撤销的操作
///
/// 流程：
/// 1. 找到最近的 undone = true 的 Batch
/// 2. 获取该 Batch 的所有 Change
/// 3. 对每个 Change，恢复 after 快照到 blocks 表
/// 4. 标记 Batch 为 undone = false
pub fn redo(db: &Db, document_id: &str) -> Result<UndoRedoResult, AppError> {
    let conn = crate::repo::lock_db(db);

    // 1. 找到指定文档最近的已撤销 Batch
    let batch = oplog_repo::find_latest_redoable_batch(&conn, document_id)
        .map_err(|e| AppError::Internal(format!("查询可重做 batch 失败: {}", e)))?
        .ok_or_else(|| AppError::BadRequest("没有可重做的操作".to_string()))?;

    // 2. 获取所有 Change
    let changes = oplog_repo::find_batch_changes(&conn, &batch.id)
        .map_err(|e| AppError::Internal(format!("查询 batch changes 失败: {}", e)))?;

    // 3. 恢复 after 快照
    let (affected_block_ids, affected_document_ids) =
        restore_snapshots(&conn, &changes, false)?;

    // 4. 标记为未撤销
    oplog_repo::set_batch_undone(&conn, &batch.id, false)
        .map_err(|e| AppError::Internal(format!("标记 batch undone 失败: {}", e)))?;

    Ok(UndoRedoResult {
        batch_id: batch.id,
        affected_block_ids,
        affected_document_ids,
        action: batch.action.as_str().to_string(),
    })
}

// ─── 查询历史 ──────────────────────────────────────────────────

/// 查询操作历史（分页）
pub fn get_history(
    db: &Db,
    document_id: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<HistoryEntry>, AppError> {
    let conn = crate::repo::lock_db(db);

    let batches = oplog_repo::find_batches(&conn, document_id, limit, offset)
        .map_err(|e| AppError::Internal(format!("查询历史失败: {}", e)))?;

    let mut entries = Vec::with_capacity(batches.len());
    for batch in batches {
        let changes = oplog_repo::find_batch_changes(&conn, &batch.id)
            .map_err(|e| AppError::Internal(format!("查询 batch changes 失败: {}", e)))?;

        entries.push(HistoryEntry {
            batch_id: batch.id,
            action: batch.action.as_str().to_string(),
            description: batch.description,
            timestamp: batch.timestamp,
            undone: batch.undone,
            changes: changes
                .into_iter()
                .map(|c| ChangeSummary {
                    block_id: c.block_id,
                    change_type: c.change_type.as_str().to_string(),
                })
                .collect(),
        });
    }

    Ok(entries)
}

/// 查询单个 Block 的变更历史
pub fn get_block_history(
    db: &Db,
    block_id: &str,
    limit: u32,
) -> Result<Vec<Change>, AppError> {
    let conn = crate::repo::lock_db(db);

    // 验证 Block 存在（含已删除）
    let _ = block_repo::find_by_id_raw(&conn, block_id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在", block_id)))?;

    oplog_repo::find_block_history(&conn, block_id, limit)
        .map_err(|e| AppError::Internal(format!("查询 Block 历史失败: {}", e)))
}

// ─── 内部辅助 ──────────────────────────────────────────────────

/// 恢复快照到 blocks 表
///
/// `use_before = true` 时恢复 before 快照（undo），
/// `use_before = false` 时恢复 after 快照（redo）。
///
/// 返回 (affected_block_ids, affected_document_ids)
fn restore_snapshots(
    conn: &rusqlite::Connection,
    changes: &[Change],
    use_before: bool,
) -> Result<(Vec<String>, Vec<String>), AppError> {
    let mut affected_block_ids = Vec::new();
    let mut affected_document_ids = Vec::new();

    for change in changes {
        let snapshot = if use_before {
            change.before.as_ref()
        } else {
            change.after.as_ref()
        };

        match snapshot {
            Some(snap) => {
                // 恢复快照：upsert block 到数据库
                restore_single_block(conn, &change.block_id, snap, &change.change_type)?;
                affected_block_ids.push(change.block_id.clone());
                if !affected_document_ids.contains(&snap.document_id) {
                    affected_document_ids.push(snap.document_id.clone());
                }
            }
            None => {
                // create 的 before 为 None（undo → 删除该 block）
                // delete 的 after 为 None（redo → 删除该 block）
                if use_before && change.change_type == ChangeType::Created {
                    // undo create → 软删除该 block
                    soft_delete_block(conn, &change.block_id)?;
                    affected_block_ids.push(change.block_id.clone());
                } else if !use_before && change.change_type == ChangeType::Deleted {
                    // redo delete → 软删除该 block
                    soft_delete_block(conn, &change.block_id)?;
                    affected_block_ids.push(change.block_id.clone());
                }
            }
        }
    }

    Ok((affected_block_ids, affected_document_ids))
}

/// 将快照恢复到 blocks 表
///
/// 根据 change_type 决定是 INSERT（新建恢复）还是 UPDATE（更新恢复）。
fn restore_single_block(
    conn: &rusqlite::Connection,
    block_id: &str,
    snap: &BlockSnapshot,
    change_type: &ChangeType,
) -> Result<(), AppError> {
    let now = now_iso();

    match change_type {
        ChangeType::Deleted => {
            // undo delete → 恢复已删除的 block（UPDATE status + 内容）
            conn.execute(
                "UPDATE blocks SET
                    parent_id = ?1, document_id = ?2, position = ?3,
                    block_type = ?4, content_type = ?5, content = ?6,
                    properties = ?7, status = ?8, modified = ?9
                 WHERE id = ?10",
                rusqlite::params![
                    snap.parent_id,
                    snap.document_id,
                    snap.position,
                    snap.block_type,
                    snap.content_type,
                    snap.content,
                    snap.properties,
                    snap.status,
                    now,
                    block_id,
                ],
            )
            .map_err(|e| AppError::Internal(format!("恢复 block 失败: {}", e)))?;
        }
        ChangeType::Created => {
            // redo create → 重新插入 block
            conn.execute(
                "INSERT INTO blocks (
                    id, parent_id, document_id, position, block_type, content_type,
                    content, properties, version, status, schema_version,
                    author, encrypted, created, modified
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, 1, 'system', 0, ?10, ?10)
                ON CONFLICT (id) DO UPDATE SET
                    parent_id = excluded.parent_id,
                    document_id = excluded.document_id,
                    position = excluded.position,
                    block_type = excluded.block_type,
                    content_type = excluded.content_type,
                    content = excluded.content,
                    properties = excluded.properties,
                    status = excluded.status,
                    modified = excluded.modified",
                rusqlite::params![
                    block_id,
                    snap.parent_id,
                    snap.document_id,
                    snap.position,
                    snap.block_type,
                    snap.content_type,
                    snap.content,
                    snap.properties,
                    snap.status,
                    now,
                ],
            )
            .map_err(|e| AppError::Internal(format!("重做 create block 失败: {}", e)))?;
        }
        _ => {
            // update / move / restored / reparented → 直接更新
            conn.execute(
                "UPDATE blocks SET
                    parent_id = ?1, document_id = ?2, position = ?3,
                    block_type = ?4, content_type = ?5, content = ?6,
                    properties = ?7, status = ?8, modified = ?9
                 WHERE id = ?10",
                rusqlite::params![
                    snap.parent_id,
                    snap.document_id,
                    snap.position,
                    snap.block_type,
                    snap.content_type,
                    snap.content,
                    snap.properties,
                    snap.status,
                    now,
                    block_id,
                ],
            )
            .map_err(|e| AppError::Internal(format!("恢复 block 快照失败: {}", e)))?;
        }
    }

    Ok(())
}

/// 软删除 block（undo create / redo delete 时使用）
fn soft_delete_block(
    conn: &rusqlite::Connection,
    block_id: &str,
) -> Result<(), AppError> {
    let now = now_iso();
    conn.execute(
        "UPDATE blocks SET status = 'deleted', modified = ?1 WHERE id = ?2",
        rusqlite::params![now, block_id],
    )
    .map_err(|e| AppError::Internal(format!("软删除 block 失败: {}", e)))?;
    Ok(())
}

/// 生成时间有序的唯一 ID（时间戳 + 随机后缀）
pub fn new_batch_id() -> String {
    let ts = chrono::Utc::now().timestamp_millis();
    format!("{ts:016x}-{:04x}", rand::random::<u16>())
}

// ─── 便捷构造函数 ──────────────────────────────────────────────

/// 创建一个新的 Batch（自动生成 ID 和时间戳）
pub fn new_batch(action: Action, description: Option<String>, document_id: &str) -> Batch {
    Batch {
        id: new_batch_id(),
        document_id: document_id.to_string(),
        action,
        description,
        timestamp: now_iso(),
        undone: false,
    }
}

/// 创建一个 Change 记录
pub fn new_change(
    batch_id: &str,
    block_id: &str,
    change_type: ChangeType,
    before: Option<BlockSnapshot>,
    after: Option<BlockSnapshot>,
) -> Change {
    Change {
        id: 0, // 自增，由数据库生成
        batch_id: batch_id.to_string(),
        block_id: block_id.to_string(),
        change_type,
        before,
        after,
    }
}

/// 从 Block 创建 before/after 变更对
///
/// 用于 update/move 等需要前后对比的操作。
pub fn block_change_pair(
    batch_id: &str,
    block_id: &str,
    change_type: ChangeType,
    before_block: &Block,
    after_block: &Block,
) -> Change {
    Change {
        id: 0,
        batch_id: batch_id.to_string(),
        block_id: block_id.to_string(),
        change_type,
        before: Some(BlockSnapshot::from_block(before_block)),
        after: Some(BlockSnapshot::from_block(after_block)),
    }
}
