//! 共享工具函数
//!
//! 事务管理、ID 推导、校验、reparent、序列化等基础设施。
//! 只依赖 repo 层（L0），不依赖任何 L2+ 模块。

use std::collections::HashMap;

use crate::api::request::PropertiesMode;
use crate::error::AppError;
use crate::block_system::model::{Block, BlockType};
use crate::repo::block_repo as repo;
use crate::util::now_iso;

use super::position;

// ─── 事务 ───────────────────────────────────────────────────────

/// 事务提交或回滚
pub(crate) fn finish_tx<T>(
    conn: &rusqlite::Connection,
    result: &Result<T, AppError>,
) -> Result<(), AppError> {
    match result {
        Ok(_) => conn.execute_batch("COMMIT")
            .map_err(|e| AppError::Internal(format!("提交事务失败: {}", e))),
        Err(_) => { let _ = conn.execute_batch("ROLLBACK"); Ok(()) }
    }
}

/// 在事务内执行闭包，自动 BEGIN IMMEDIATE / COMMIT / ROLLBACK
pub(crate) fn run_in_transaction<T, F>(
    conn: &rusqlite::Connection,
    f: F,
) -> Result<T, AppError>
where
    F: FnOnce() -> Result<T, AppError>,
{
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| AppError::Internal(format!("开启事务失败: {}", e)))?;
    let result = f();
    finish_tx(conn, &result)?;
    result
}

// ─── ID 推导 ────────────────────────────────────────────────────

/// 从父块推断 document_id
///
/// - Document 类型 → document_id = parent.id（文档块自身就是文档根）
/// - 其他 → 继承 parent.document_id
pub(crate) fn derive_document_id(parent: &Block) -> String {
    if matches!(parent.block_type, BlockType::Document) {
        parent.id.clone()
    } else {
        parent.document_id.clone()
    }
}

/// 从 parent_id 推断 document_id（不加载完整 Block）
pub(crate) fn derive_document_id_from_parent(
    conn: &rusqlite::Connection,
    parent_id: &str,
) -> Result<String, AppError> {
    let parent = repo::find_by_id(conn, parent_id)
        .map_err(|e| AppError::Internal(format!("查询 parent {} 失败: {}", parent_id, e)))?;
    Ok(derive_document_id(&parent))
}

// ─── 移动辅助 ───────────────────────────────────────────────────

/// 从 before_id / after_id 推导目标父块 id
pub(crate) fn resolve_target_parent(
    conn: &rusqlite::Connection,
    before_id: &Option<String>,
    after_id: &Option<String>,
    current_parent_id: &str,
) -> Result<String, AppError> {
    let sibling_id = before_id.as_deref().or(after_id.as_deref());
    match sibling_id {
        Some(sid) => {
            let sibling = repo::find_by_id(conn, sid)
                .map_err(|_| AppError::NotFound(format!(
                    "定位块 {} 不存在或已删除", sid
                )))?;
            Ok(sibling.parent_id.clone())
        }
        None => Ok(current_parent_id.to_string()),
    }
}

/// 循环引用检测：target_parent 不能是 id 本身，也不能在其后代中
pub(crate) fn validate_no_cycle(
    conn: &rusqlite::Connection,
    id: &str,
    target_parent_id: &str,
    current_parent_id: &str,
) -> Result<(), AppError> {
    if target_parent_id == current_parent_id {
        return Ok(());
    }
    if target_parent_id == id {
        return Err(AppError::BadRequest("不能将 Block 移动到自身下".to_string()));
    }
    let descendant_ids = repo::find_descendant_ids(conn, id)
        .map_err(|e| AppError::Internal(format!("查询后代失败: {}", e)))?;
    if descendant_ids.contains(&target_parent_id.to_string()) {
        return Err(AppError::BadRequest(
            "不能将 Block 移动到自身后代的下方".to_string(),
        ));
    }
    let parent_exists = repo::exists_normal(conn, target_parent_id).unwrap_or(false);
    if !parent_exists {
        return Err(AppError::BadRequest(format!(
            "目标父块 {} 不存在或已删除", target_parent_id
        )));
    }
    Ok(())
}

/// 将 anchor 块的所有直系子块 reparent 到 new_parent_id
pub(crate) fn reparent_children_to(
    conn: &rusqlite::Connection,
    anchor_parent_id: &str,
    anchor_position: &str,
    anchor_child_ids: &[String],
    new_parent_id: &str,
    update_document_id: bool,
) -> Result<(), AppError> {
    if anchor_child_ids.is_empty() {
        return Ok(());
    }

    let new_document_id = if update_document_id {
        Some(derive_document_id_from_parent(conn, new_parent_id)?)
    } else {
        None
    };

    let siblings_after = repo::find_siblings_after(conn, anchor_parent_id, anchor_position)
        .map_err(|e| AppError::Internal(format!("查询后续兄弟失败: {}", e)))?;

    let next_bound = siblings_after.first().map(|s| s.position.clone());

    let mut pos = match &next_bound {
        Some(nb) => position::generate_between(anchor_position, nb)?,
        None => position::generate_after(anchor_position),
    };

    let now = now_iso();
    for child_id in anchor_child_ids {
        if let Some(ref doc_id) = new_document_id {
            repo::update_parent_position_document_id(
                conn, child_id, new_parent_id, &pos, doc_id, &now,
            )
        } else {
            repo::update_parent_position(
                conn, child_id, new_parent_id, &pos, &now,
            )
        }
        .map_err(|e| AppError::Internal(format!("reparent 子块 {} 失败: {}", child_id, e)))?;
        pos = match &next_bound {
            Some(nb) => position::generate_between(&pos, nb)?,
            None => position::generate_after(&pos),
        };
    }

    Ok(())
}

// ─── 序列化 ─────────────────────────────────────────────────────

/// 安全 JSON 序列化
pub(crate) fn to_json<T: serde::Serialize>(val: &T) -> String {
    serde_json::to_string(val).unwrap_or_else(|e| {
        tracing::error!("序列化失败（不应发生）: {}", e);
        "{}".to_string()
    })
}

/// 合并或替换属性
pub(crate) fn merge_properties(
    current: &HashMap<String, String>,
    new_props: Option<&HashMap<String, String>>,
    mode: &PropertiesMode,
) -> HashMap<String, String> {
    match new_props {
        Some(np) if *mode == PropertiesMode::Replace => np.clone(),
        Some(np) => {
            let mut merged = current.clone();
            merged.extend(np.clone());
            merged
        }
        None => current.clone(),
    }
}
