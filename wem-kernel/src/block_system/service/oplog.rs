//! 操作日志（Oplog）业务逻辑层
//!
//! 基于 Operation-based 操作日志，为文档提供 undo/redo 能力。
//!
//! 提供：
//! - `record_operation()` — 每次 Block 操作后记录 operation + changes（含自动 GC）
//! - `undo()` — 撤销最近一次操作（恢复 before 快照）
//! - `redo()` — 重做最近被撤销的操作（恢复 after 快照）
//! - `get_history()` — 查询操作日志（分页）
//! - `get_block_history()` — 查询单个 Block 的变更日志

use crate::error::AppError;
use crate::block_system::model::event::BlockEvent;
use crate::block_system::model::oplog::{
    Action, Operation, BlockSnapshot, Change, ChangeSummary, ChangeType, HistoryEntry, UndoRedoResult,
};
use crate::block_system::model::Block;
use crate::repo::{block_repo, oplog_repo, Db};
use crate::util::now_iso;
use super::event;

/// Operation 容量上限：只保留最近 N 次操作记录
///
/// 超出此上限后，`record_operation()` 会自动清理最老的 operation。
/// changes 表通过 FOREIGN KEY ON DELETE CASCADE 自动级联删除。
pub const MAX_OPERATIONS: usize = 1000;

// ─── 记录操作 ──────────────────────────────────────────────────

/// 记录一次 Operation
///
/// 在 Block CRUD 操作成功后、同一个数据库锁内调用。
/// 自动写入 Operation 和所有 Change 记录，并在超出容量上限时清理最老的 operation。
pub fn record_operation(
    conn: &rusqlite::Connection,
    op: &Operation,
    changes: &[Change],
) -> Result<(), AppError> {
    // 写入 Operation
    oplog_repo::insert_operation(conn, op)
        .map_err(|e| AppError::Internal(format!("写入 operation 失败: {}", e)))?;

    // 写入 Changes
    if !changes.is_empty() {
        oplog_repo::insert_changes(conn, changes)
            .map_err(|e| AppError::Internal(format!("写入 changes 失败: {}", e)))?;
    }

    // 自动 GC：超出容量上限时清理最老的 operation
    gc_operations(conn)?;

    Ok(())
}

/// 操作容量 GC
fn gc_operations(conn: &rusqlite::Connection) -> Result<(), AppError> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))
        .map_err(|e| AppError::Internal(format!("统计 operation 数量失败: {}", e)))?;

    if count as usize > MAX_OPERATIONS {
        oplog_repo::cleanup_old_operations(conn, MAX_OPERATIONS)
            .map_err(|e| AppError::Internal(format!("GC operation 失败: {}", e)))?;
    }

    Ok(())
}

// ─── Undo / Redo ──────────────────────────────────────────────

pub fn undo(db: &Db, document_id: &str) -> Result<UndoRedoResult, AppError> {
    let result = undo_redo_core(db, document_id, UndoRedoMode::Undo)?;
    emit_batch_events(&result.affected_document_ids);
    Ok(result)
}

pub fn redo(db: &Db, document_id: &str) -> Result<UndoRedoResult, AppError> {
    let result = undo_redo_core(db, document_id, UndoRedoMode::Redo)?;
    emit_batch_events(&result.affected_document_ids);
    Ok(result)
}

enum UndoRedoMode {
    Undo,
    Redo,
}

fn undo_redo_core(
    db: &Db,
    document_id: &str,
    mode: UndoRedoMode,
) -> Result<UndoRedoResult, AppError> {
    let conn = crate::repo::lock_db(db);
    let no_op_msg = match mode {
        UndoRedoMode::Undo => "没有可撤销的操作",
        UndoRedoMode::Redo => "没有可重做的操作",
    };
    let use_before = matches!(mode, UndoRedoMode::Undo);
    let mark_undone = use_before;

    super::helpers::run_in_transaction(&conn, || {
        let op = match mode {
            UndoRedoMode::Undo => oplog_repo::find_latest_undoable_operation(&conn, document_id),
            UndoRedoMode::Redo => oplog_repo::find_latest_redoable_operation(&conn, document_id),
        }
        .map_err(|e| AppError::Internal(format!("查询 operation 失败: {}", e)))?
        .ok_or_else(|| AppError::BadRequest(no_op_msg.to_string()))?;

        let changes = oplog_repo::find_operation_changes(&conn, &op.id)
            .map_err(|e| AppError::Internal(format!("查询 operation changes 失败: {}", e)))?;

        let (affected_block_ids, affected_document_ids) =
            restore_snapshots(&conn, &changes, use_before)?;

        oplog_repo::set_operation_undone(&conn, &op.id, mark_undone)
            .map_err(|e| AppError::Internal(format!("标记 operation undone 失败: {}", e)))?;

        Ok(UndoRedoResult {
            operation_id: op.id,
            affected_block_ids,
            affected_document_ids,
            action: op.action.as_str().to_string(),
        })
    })
}

fn emit_batch_events(document_ids: &[String]) {
    for doc_id in document_ids {
        event::EventBus::global().emit(BlockEvent::BlocksBatchChanged {
            document_id: doc_id.clone(),
            editor_id: None,
        });
    }
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

    let ops = oplog_repo::find_operations(&conn, document_id, limit, offset)
        .map_err(|e| AppError::Internal(format!("查询历史失败: {}", e)))?;

    let mut entries = Vec::with_capacity(ops.len());
    for op in ops {
        let changes = oplog_repo::find_operation_changes(&conn, &op.id)
            .map_err(|e| AppError::Internal(format!("查询 operation changes 失败: {}", e)))?;

        entries.push(HistoryEntry {
            operation_id: op.id,
            action: op.action.as_str().to_string(),
            description: op.description,
            timestamp: op.timestamp,
            undone: op.undone,
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
///
/// 返回涉及该 Block 的所有 Operation，按时间倒序排列。
/// 每个 Operation 对应一个 [`HistoryEntry`]，只包含与目标 Block 相关的 Change。
pub fn get_block_history(
    db: &Db,
    block_id: &str,
    limit: u32,
) -> Result<Vec<HistoryEntry>, AppError> {
    let conn = crate::repo::lock_db(db);

    // 验证 Block 存在（含已删除）
    let _ = block_repo::find_by_id_raw(&conn, block_id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", block_id)))?;

    let ops = oplog_repo::find_block_operations(&conn, block_id, limit)
        .map_err(|e| AppError::Internal(format!("查询 Block 历史失败: {}", e)))?;

    let mut entries = Vec::with_capacity(ops.len());
    for op in ops {
        let changes = oplog_repo::find_operation_changes(&conn, &op.id)
            .map_err(|e| AppError::Internal(format!("查询 operation changes 失败: {}", e)))?;

        // 只保留与目标 Block 相关的 changes
        let related: Vec<_> = changes
            .into_iter()
            .filter(|c| c.block_id == block_id)
            .collect();

        entries.push(HistoryEntry {
            operation_id: op.id,
            action: op.action.as_str().to_string(),
            description: op.description,
            timestamp: op.timestamp,
            undone: op.undone,
            changes: related
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
                restore_single_block(conn, &change.block_id, snap, &change.change_type)?;
                affected_block_ids.push(change.block_id.clone());
                if !affected_document_ids.contains(&snap.document_id) {
                    affected_document_ids.push(snap.document_id.clone());
                }
            }
            None => {
                if use_before && change.change_type == ChangeType::Created {
                    soft_delete_block(conn, &change.block_id)?;
                    affected_block_ids.push(change.block_id.clone());
                } else if !use_before && change.change_type == ChangeType::Deleted {
                    soft_delete_block(conn, &change.block_id)?;
                    affected_block_ids.push(change.block_id.clone());
                }
            }
        }
    }

    Ok((affected_block_ids, affected_document_ids))
}

/// 将快照恢复到 blocks 表
fn restore_single_block(
    conn: &rusqlite::Connection,
    block_id: &str,
    snap: &BlockSnapshot,
    change_type: &ChangeType,
) -> Result<(), AppError> {
    let now = now_iso();

    match change_type {
        ChangeType::Created => {
            block_repo::upsert_from_snapshot(conn, block_id, snap, &now)
                .map_err(|e| AppError::Internal(format!("重做 create block 失败: {}", e)))?;
        }
        _ => {
            block_repo::restore_from_snapshot(conn, block_id, snap, &now)
                .map_err(|e| AppError::Internal(format!("恢复 block 失败: {}", e)))?;
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
    block_repo::soft_delete_no_version(conn, block_id, &now)
        .map_err(|e| AppError::Internal(format!("软删除 block 失败: {}", e)))
}

/// 生成时间有序的唯一 ID（时间戳 + 随机后缀）
pub fn new_operation_id() -> String {
    let ts = chrono::Utc::now().timestamp_millis();
    format!("{ts:016x}-{:04x}", rand::random::<u16>())
}

// ─── 便捷构造函数 ──────────────────────────────────────────────

/// 创建一个新的 Operation（自动生成 ID 和时间戳）
pub fn new_operation(
    action: Action,
    document_id: &str,
    editor_id: Option<String>,
) -> Operation {
    Operation {
        id: new_operation_id(),
        document_id: document_id.to_string(),
        action,
        description: None,
        timestamp: now_iso(),
        undone: false,
        editor_id,
    }
}

/// 创建一个 Change 记录
pub fn new_change(
    operation_id: &str,
    block_id: &str,
    change_type: ChangeType,
    before: Option<BlockSnapshot>,
    after: Option<BlockSnapshot>,
) -> Change {
    Change {
        id: 0,
        operation_id: operation_id.to_string(),
        block_id: block_id.to_string(),
        change_type,
        before,
        after,
    }
}

/// 从 Block 创建 before/after 变更对
pub fn block_change_pair(
    operation_id: &str,
    block_id: &str,
    change_type: ChangeType,
    before_block: &Block,
    after_block: &Block,
) -> Change {
    Change {
        id: 0,
        operation_id: operation_id.to_string(),
        block_id: block_id.to_string(),
        change_type,
        before: Some(BlockSnapshot::from_block(before_block)),
        after: Some(BlockSnapshot::from_block(after_block)),
    }
}
