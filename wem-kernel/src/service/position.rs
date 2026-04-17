//! Position 计算辅助函数
//!
//! 集中管理 Fractional Indexing 相关的位置计算逻辑：
//! - 新建 Block 时的插入位置
//! - 移动 Block 时的目标位置
//! - 兄弟 Block 的位置查询
//!
//! 核心函数 `calculate_position` 统一处理所有场景，
//! `calculate_insert_position` / `calculate_move_position` 是它的语义化封装。

use crate::repo::block_repo as repo;
use crate::error::AppError;
use crate::util::fractional;

// ─── 核心计算 ──────────────────────────────────────────────────

/// 在指定父块内，根据前后邻居计算新的 fractional index position。
///
/// - 同时指定 after 和 before → 插在两者之间
/// - 只指定 after → 插在其后（如有后继兄弟则插在中间）
/// - 只指定 before → 插在其前（如有前驱兄弟则插在中间）
/// - 都不指定 → 追加到末尾
///
/// `after` / `before` 是兄弟块的 position 值（已验证属于 target_parent）。
fn calculate_position(
    parent_id: &str,
    after: Option<&str>,
    before: Option<&str>,
) -> Result<String, AppError> {
    match (after, before) {
        (Some(ap), Some(bp)) => {
            if ap >= bp {
                return Err(AppError::BadRequest(
                    "after 位置必须在 before 位置之前".to_string(),
                ));
            }
            Ok(fractional::generate_between(ap, bp))
        }
        (Some(ap), None) => {
            // 理论上应该查 after 的下一个兄弟来 generate_between，
            // 但调用方已处理了 sibling 查询，这里不可能走到。
            // 此分支保留作为安全兜底。
            Ok(fractional::generate_after(ap))
        }
        (None, Some(bp)) => {
            Ok(fractional::generate_before(bp))
        }
        (None, None) => {
            // 调用方已处理了空列表 → generate_first 的场景
            // 此处为兜底，不应走到
            Ok(fractional::generate_first())
        }
    }
}

// ─── 语义化封装 ────────────────────────────────────────────────

/// 计算新 Block 的插入 position。
///
/// - 有 after_id → 插在 after_id 之后（如有后继兄弟则插入两者之间）
/// - 无 after_id → 追加到末尾
pub(crate) fn calculate_insert_position(
    conn: &rusqlite::Connection,
    parent_id: &str,
    after_id: Option<&str>,
) -> Result<String, AppError> {
    match after_id {
        Some(aid) => {
            let after_pos = resolve_sibling_position(conn, aid, parent_id)?;

            let next_pos = repo::get_next_sibling_position(conn, parent_id, &after_pos)
                .map_err(|e| AppError::Internal(format!("查询后继兄弟失败: {}", e)))?;

            calculate_position(
                parent_id,
                Some(&after_pos),
                next_pos.as_deref(),
            )
        }
        None => {
            let max_pos = repo::get_max_position(conn, parent_id)
                .map_err(|e| AppError::Internal(format!("查询最大 position 失败: {}", e)))?;

            match max_pos {
                Some(mp) => Ok(fractional::generate_after(&mp)),
                None => Ok(fractional::generate_first()),
            }
        }
    }
}

/// 计算移动操作的新 position。
///
/// 支持三种定位方式（优先级从高到低）：
/// 1. 同时指定 before_id 和 after_id → 插在两者之间
/// 2. 只指定 after_id → 插在之后
/// 3. 只指定 before_id → 插在之前
/// 4. 都不指定 → 追加到末尾
pub(crate) fn calculate_move_position(
    conn: &rusqlite::Connection,
    target_parent_id: &str,
    before_id: Option<&str>,
    after_id: Option<&str>,
) -> Result<String, AppError> {
    match (before_id, after_id) {
        (Some(bid), Some(aid)) => {
            let after_pos = resolve_sibling_position(conn, aid, target_parent_id)?;
            let before_pos = resolve_sibling_position(conn, bid, target_parent_id)?;
            calculate_position(target_parent_id, Some(&after_pos), Some(&before_pos))
        }
        (Some(bid), None) => {
            let before_pos = resolve_sibling_position(conn, bid, target_parent_id)?;

            let prev_pos = repo::get_prev_sibling_position(conn, target_parent_id, &before_pos)
                .map_err(|e| AppError::Internal(format!("查询前驱兄弟失败: {}", e)))?;

            calculate_position(
                target_parent_id,
                prev_pos.as_deref(),
                Some(&before_pos),
            )
        }
        (None, Some(aid)) => {
            let after_pos = resolve_sibling_position(conn, aid, target_parent_id)?;

            let next_pos = repo::get_next_sibling_position(conn, target_parent_id, &after_pos)
                .map_err(|e| AppError::Internal(format!("查询后继兄弟失败: {}", e)))?;

            calculate_position(
                target_parent_id,
                Some(&after_pos),
                next_pos.as_deref(),
            )
        }
        (None, None) => {
            let max_pos = repo::get_max_position(conn, target_parent_id)
                .map_err(|e| AppError::Internal(format!("查询最大 position 失败: {}", e)))?;

            match max_pos {
                Some(mp) => Ok(fractional::generate_after(&mp)),
                None => Ok(fractional::generate_first()),
            }
        }
    }
}

// ─── 辅助函数 ──────────────────────────────────────────────────

/// 获取指定兄弟 Block 的 position。
///
/// 验证该 Block 是 target_parent 的子块且未删除。
fn resolve_sibling_position(
    conn: &rusqlite::Connection,
    sibling_id: &str,
    target_parent_id: &str,
) -> Result<String, AppError> {
    repo::get_position(conn, sibling_id, target_parent_id)
        .map_err(|_| {
            AppError::BadRequest(format!(
                "Block {} 不是 {} 的有效子块",
                sibling_id, target_parent_id
            ))
        })
}
