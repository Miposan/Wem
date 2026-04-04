//! Oplog 业务逻辑层
//!
//! 提供：
//! - `record_op()` — 每次 Block 操作后记录 oplog + 自动快照检查
//! - `get_block_history()` — 查询 Block 的变更历史
//! - `get_version_content()` — 回放 oplog 还原任意版本
//! - `rollback_block()` — 回滚到指定版本
//! - `create_snapshot()` / `maybe_create_snapshot()` — 快照管理
//!
//! 参考 05-oplog.md §1~§6

use crate::db::{block_repo as repo, oplog_repo, Db};
use crate::error::AppError;
use crate::model::Block;
use crate::model::oplog::{
    Action, CreateOpData, DeleteOpData, HistoryEntry, RollbackResult, Snapshot,
    SnapshotReason, SnapshotResult, UpdateOpData, VersionContent,
};

/// 快照触发阈值：距上次快照的操作数
const SNAPSHOT_OP_THRESHOLD: i64 = 50;

// ─── 记录操作 ──────────────────────────────────────────────────

/// 记录一条操作日志
///
/// 在 Block CRUD 成功后调用。自动递增 block version 并写入 oplog。
/// 同时检查是否需要创建快照。
///
/// **注意**：调用方必须在同一个 `conn` 事务/锁内调用此函数，
/// 确保 oplog 与 block 数据的一致性。
pub fn record_op(
    conn: &rusqlite::Connection,
    block_id: &str,
    action: &Action,
    data: &str,
    prev_version: u64,
    new_version: u64,
    timestamp: &str,
) -> Result<(), AppError> {
    // 写入 oplog
    oplog_repo::insert_oplog(conn, block_id, action, data, prev_version, new_version, timestamp)
        .map_err(|e| AppError::Internal(format!("写入 oplog 失败: {}", e)))?;

    // 检查是否需要自动创建快照
    let _ = maybe_create_snapshot(conn, block_id, new_version, timestamp);

    Ok(())
}

// ─── 查询历史 ──────────────────────────────────────────────────

/// 获取 Block 的变更历史
pub fn get_block_history(
    db: &Db,
    block_id: &str,
    limit: u32,
) -> Result<Vec<HistoryEntry>, AppError> {
    let conn = db.lock().unwrap();

    // 验证 Block 存在
    let _block = repo::find_by_id_raw(&conn, block_id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在", block_id)))?;

    let ops = oplog_repo::find_block_history(&conn, block_id, limit)
        .map_err(|e| AppError::Internal(format!("查询历史失败: {}", e)))?;

    let entries: Vec<HistoryEntry> = ops
        .into_iter()
        .map(|op| {
            let data_value: serde_json::Value = serde_json::from_str(&op.data)
                .unwrap_or(serde_json::Value::Null);
            HistoryEntry {
                op_id: op.op_id,
                block_id: op.block_id,
                action: op.action.as_str().to_string(),
                data: data_value,
                prev_version: op.prev_version,
                new_version: op.new_version,
                timestamp: op.timestamp,
            }
        })
        .collect();

    Ok(entries)
}

// ─── 版本回放 ──────────────────────────────────────────────────

/// 获取 Block 在指定版本的完整内容
///
/// 策略：快照 + 回放
/// 1. 找到 <= target_version 的最近快照
/// 2. 从快照版本到目标版本之间的所有 Update/Create oplog
/// 3. 应用 oplog 得到目标版本的完整内容
///
/// 对于简化版：直接返回当前 block 内容 + 历史元数据。
/// 真正的回放需要解析 Update oplog 中的 content/properties。
pub fn get_version_content(
    db: &Db,
    block_id: &str,
    version: u64,
) -> Result<VersionContent, AppError> {
    let conn = db.lock().unwrap();

    // 验证 Block 存在（含已删除）
    let current = repo::find_by_id_raw(&conn, block_id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在", block_id)))?;

    // 1. 找到 <= version 的最近快照
    let snapshot = oplog_repo::find_snapshot_at_or_before(&conn, block_id, version)
        .map_err(|e| AppError::Internal(format!("查询快照失败: {}", e)))?;

    let (mut content, mut properties, source) = match snapshot {
        Some(ref snap) => {
            // 从快照恢复基础内容
            let content_str = String::from_utf8_lossy(&snap.content).to_string();
            (content_str, snap.properties.clone(), format!("snapshot@v{}", snap.version))
        }
        None => {
            // 没有快照，从 oplog 链头部开始
            (String::new(), "{}".to_string(), "empty".to_string())
        }
    };

    // 2. 查询快照版本到目标版本之间的 oplog
    let snap_version = snapshot.as_ref().map(|s| s.version).unwrap_or(0);
    let _ = source; // source 用于 API 响应
    let ops = oplog_repo::find_oplog_range(&conn, block_id, snap_version, version)
        .map_err(|e| AppError::Internal(format!("查询 oplog 区间失败: {}", e)))?;

    // 3. 回放 oplog — 只应用 Update 和 Create 的 content/properties
    let applied_count = ops.len();
    for op in &ops {
        match op.action {
            Action::Create | Action::Update => {
                if let Ok(update_data) = serde_json::from_str::<UpdateOpData>(&op.data) {
                    content = update_data.content;
                    properties = update_data.properties;
                } else if let Ok(create_data) = serde_json::from_str::<CreateOpData>(&op.data) {
                    content = create_data.content;
                    properties = create_data.properties;
                }
            }
            Action::Delete => {
                // Delete 操作记录了删除前的快照，可用于恢复
                if let Ok(delete_data) = serde_json::from_str::<DeleteOpData>(&op.data) {
                    content = delete_data.snapshot.content;
                    properties = delete_data.snapshot.properties;
                }
            }
            _ => {}
        }
    }

    // 4. 构建 VersionContent
    //    使用 current block 的结构字段，替换 content 和 properties
    let mut version_block = current.clone();
    version_block.version = version;
    version_block.content = content.into_bytes();
    version_block.properties = serde_json::from_str(&properties).unwrap_or_default();

    Ok(VersionContent {
        version,
        block: version_block,
        source: format!("{}+{}ops", source, applied_count),
    })
}

// ─── 回滚 ─────────────────────────────────────────────────────

/// 回滚 Block 到指定版本
///
/// 1. 获取目标版本的完整内容
/// 2. 用目标版本的内容更新当前 Block（version 递增）
/// 3. 记录一条 Update oplog（标记 is_rollback=true）
///
/// 参考 05-oplog.md §5
pub fn rollback_block(
    db: &Db,
    block_id: &str,
    target_version: u64,
    current_version: u64,
) -> Result<RollbackResult, AppError> {
    let conn = db.lock().unwrap();

    // 1. 验证当前 Block
    let current = repo::find_by_id(&conn, block_id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", block_id)))?;

    if current_version != current.version {
        return Err(AppError::VersionConflict(current.version));
    }

    if target_version >= current.version {
        return Err(AppError::BadRequest(
            "目标版本必须小于当前版本".to_string(),
        ));
    }

    if target_version == 0 {
        return Err(AppError::BadRequest("版本号不能为 0".to_string()));
    }

    // 2. 获取目标版本内容（使用快照 + 回放）
    let target_content = reconstruct_version(&conn, block_id, target_version)?;

    // 3. 更新 Block 内容（乐观锁）
    let new_version = current.version + 1;
    let now = now_iso();
    let content_bytes = target_content.content;
    let properties_json = serde_json::to_string(&target_content.properties).unwrap_or_default();

    let rows = repo::update_content_and_props(
        &conn,
        block_id,
        &content_bytes,
        &properties_json,
        &now,
        current.version,
    )
    .map_err(|e| AppError::Internal(format!("回滚更新失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::VersionConflict(current.version));
    }

    // 4. 记录 oplog（标记为回滚）
    let rollback_data = UpdateOpData {
        content: String::from_utf8_lossy(&content_bytes).to_string(),
        properties: properties_json,
        is_rollback: Some(true),
        rollback_to: Some(target_version),
    };
    let data_json = serde_json::to_string(&rollback_data).unwrap_or_default();

    record_op(
        &conn,
        block_id,
        &Action::Update,
        &data_json,
        current.version,
        new_version,
        &now,
    )?;

    Ok(RollbackResult {
        id: block_id.to_string(),
        prev_version: current.version,
        new_version,
        rollback_to_version: target_version,
    })
}

// ─── 快照管理 ──────────────────────────────────────────────────

/// 手动创建快照
pub fn create_snapshot(
    db: &Db,
    block_id: &str,
) -> Result<SnapshotResult, AppError> {
    let conn = db.lock().unwrap();

    let block = repo::find_by_id_raw(&conn, block_id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在", block_id)))?;

    let now = now_iso();
    let snap = block_to_snapshot(&block, &now);

    oplog_repo::upsert_snapshot(&conn, &snap)
        .map_err(|e| AppError::Internal(format!("创建快照失败: {}", e)))?;

    // 清理旧快照，最多保留 2 个
    let _ = oplog_repo::cleanup_old_snapshots(&conn, block_id, 2);

    Ok(SnapshotResult {
        block_id: block_id.to_string(),
        version: block.version,
        reason: SnapshotReason::Manual.as_str().to_string(),
    })
}

/// 检查并自动创建快照
///
/// 触发条件（满足任一）：
/// 1. 距上次快照的操作数 >= SNAPSHOT_OP_THRESHOLD (50)
///
/// TODO: 时间阈值（7 天）需要在 snapshots 表记录 created_at 并在每次检查时计算
fn maybe_create_snapshot(
    conn: &rusqlite::Connection,
    block_id: &str,
    _current_version: u64,
    timestamp: &str,
) -> Result<bool, AppError> {
    // 检查操作数阈值
    let ops_since = oplog_repo::count_oplog_since_last_snapshot(conn, block_id)
        .map_err(|e| AppError::Internal(format!("查询 oplog 计数失败: {}", e)))?;

    if ops_since < SNAPSHOT_OP_THRESHOLD {
        return Ok(false);
    }

    // 获取当前 Block 内容
    let block = match repo::find_by_id_raw(conn, block_id) {
        Ok(b) => b,
        Err(_) => return Ok(false), // Block 不存在，跳过
    };

    let snap = block_to_snapshot(&block, timestamp);

    oplog_repo::upsert_snapshot(conn, &snap)
        .map_err(|e| AppError::Internal(format!("自动快照失败: {}", e)))?;

    // 清理旧快照，最多保留 2 个
    let _ = oplog_repo::cleanup_old_snapshots(conn, block_id, 2);

    Ok(true)
}

// ─── 辅助函数 ──────────────────────────────────────────────────

/// 生成当前时间的 ISO 8601 字符串（毫秒精度）
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// 将 Block 转为 Snapshot
fn block_to_snapshot(block: &Block, timestamp: &str) -> Snapshot {
    Snapshot {
        block_id: block.id.clone(),
        version: block.version,
        block_type: serde_json::to_string(&block.block_type).unwrap_or_default(),
        content_type: block.content_type.as_str().to_string(),
        content: block.content.clone(),
        properties: serde_json::to_string(&block.properties).unwrap_or_default(),
        parent_id: block.parent_id.clone(),
        position: block.position.clone(),
        timestamp: timestamp.to_string(),
    }
}

/// 重建指定版本的 Block 内容（内部函数，调用方已持有锁）
///
/// 快照 + 回放策略
fn reconstruct_version(
    conn: &rusqlite::Connection,
    block_id: &str,
    target_version: u64,
) -> Result<Block, AppError> {
    // 找到 <= target_version 的最近快照
    let snapshot = oplog_repo::find_snapshot_at_or_before(conn, block_id, target_version)
        .map_err(|e| AppError::Internal(format!("查询快照失败: {}", e)))?;

    let (mut content, mut properties, snap_version) = match snapshot {
        Some(snap) => (
            String::from_utf8_lossy(&snap.content).to_string(),
            snap.properties,
            snap.version,
        ),
        None => (String::new(), "{}".to_string(), 0),
    };

    // 查询区间 oplog
    let ops = oplog_repo::find_oplog_range(conn, block_id, snap_version, target_version)
        .map_err(|e| AppError::Internal(format!("查询 oplog 区间失败: {}", e)))?;

    // 回放
    for op in &ops {
        match op.action {
            Action::Create | Action::Update => {
                if let Ok(update_data) = serde_json::from_str::<UpdateOpData>(&op.data) {
                    content = update_data.content;
                    properties = update_data.properties;
                } else if let Ok(create_data) = serde_json::from_str::<CreateOpData>(&op.data) {
                    content = create_data.content;
                    properties = create_data.properties;
                }
            }
            Action::Delete => {
                if let Ok(delete_data) = serde_json::from_str::<DeleteOpData>(&op.data) {
                    content = delete_data.snapshot.content;
                    properties = delete_data.snapshot.properties;
                }
            }
            _ => {}
        }
    }

    // 获取 Block 的结构字段（parent_id, position 等）
    let current = repo::find_by_id_raw(conn, block_id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在", block_id)))?;

    // 组装目标版本的 Block
    let mut result = current;
    result.version = target_version;
    result.content = content.into_bytes();
    result.properties = serde_json::from_str(&properties).unwrap_or_default();

    Ok(result)
}
