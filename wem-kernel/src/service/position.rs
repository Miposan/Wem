//! Position 计算辅助函数
//!
//! 集中管理 Fractional Indexing 相关的位置计算逻辑：
//! - 新建 Block 时的插入位置
//! - 移动 Block 时的目标位置
//! - 兄弟 Block 的位置查询

use crate::repo::block_repo as repo;
use crate::error::AppError;
use crate::util::fractional;

/// 计算新 Block 的插入 position
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
            // 获取 after_block 的 position
            let after_pos = repo::get_position(conn, aid, parent_id)
                .map_err(|_| {
                    AppError::BadRequest(format!(
                        "after_id {} 不是 {} 的有效子块",
                        aid, parent_id
                    ))
                })?;

            // 查找 after_pos 之后紧邻的兄弟（用于生成 between）
            let next_pos = repo::get_next_sibling_position(conn, parent_id, &after_pos)
                .map_err(|e| AppError::Internal(format!("查询后继兄弟失败: {}", e)))?;

            match next_pos {
                Some(np) => Ok(fractional::generate_between(&after_pos, &np)),
                None => Ok(fractional::generate_after(&after_pos)),
            }
        }
        None => {
            // 追加到末尾
            let max_pos = repo::get_max_position(conn, parent_id)
                .map_err(|e| AppError::Internal(format!("查询最大 position 失败: {}", e)))?;

            match max_pos {
                Some(mp) => Ok(fractional::generate_after(&mp)),
                None => Ok(fractional::generate_first()),
            }
        }
    }
}

/// 计算移动操作的新 position
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
        // 情况 1：同时指定 → 插在两者之间
        (Some(bid), Some(aid)) => {
            let after_pos = get_sibling_position(conn, aid, target_parent_id)?;
            let before_pos = get_sibling_position(conn, bid, target_parent_id)?;

            if after_pos >= before_pos {
                return Err(AppError::BadRequest(
                    "after_id 的位置必须在 before_id 之前".to_string(),
                ));
            }

            Ok(fractional::generate_between(&after_pos, &before_pos))
        }

        // 情况 2：只指定 before_id → 插在之前
        (Some(bid), None) => {
            let before_pos = get_sibling_position(conn, bid, target_parent_id)?;

            // 查找 before_pos 之前紧邻的兄弟
            let prev_pos = repo::get_prev_sibling_position(conn, target_parent_id, &before_pos)
                .map_err(|e| AppError::Internal(format!("查询前驱兄弟失败: {}", e)))?;

            match prev_pos {
                Some(pp) => Ok(fractional::generate_between(&pp, &before_pos)),
                None => Ok(fractional::generate_before(&before_pos)),
            }
        }

        // 情况 3：只指定 after_id → 插在之后
        (None, Some(aid)) => {
            let after_pos = get_sibling_position(conn, aid, target_parent_id)?;

            // 查找 after_pos 之后紧邻的兄弟
            let next_pos = repo::get_next_sibling_position(conn, target_parent_id, &after_pos)
                .map_err(|e| AppError::Internal(format!("查询后继兄弟失败: {}", e)))?;

            match next_pos {
                Some(np) => Ok(fractional::generate_between(&after_pos, &np)),
                None => Ok(fractional::generate_after(&after_pos)),
            }
        }

        // 情况 4：都不指定 → 追加到末尾
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

/// 获取指定兄弟 Block 的 position
///
/// 验证该 Block 是 target_parent 的子块且未删除。
fn get_sibling_position(
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
