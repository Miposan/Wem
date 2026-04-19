//! Heading 类型特化实现
//!
//! 实现 BlockTypeOps trait 的 Heading 变体：
//! - on_moved: 移动后 detach 子块 + 吸收后续同级节点
//! - on_type_changed: 类型变更时的层级重组（提升 + 逃逸 + 吸收）
//! - on_self_or_descendant_move: 拖入自身子树时 detach 子块
//!
//! Heading 特有操作：
//! - move_heading_tree: 折叠拖拽场景的子树整体移动

use crate::api::request::MoveHeadingTreeReq;
use crate::error::AppError;
use crate::model::oplog::{BlockSnapshot, Change, ChangeType, Operation};
use crate::model::{Block, BlockType};
use crate::repo::block_repo as repo;
use crate::repo::Db;
use crate::util::now_iso;

use super::block::{self, BlockTypeOps, MoveContext, TreeMoveOps};
use super::{oplog, position};

/// Heading 类型行为实现
pub struct HeadingOps;

impl BlockTypeOps for HeadingOps {
    fn validate_on_create(block_type: &BlockType) -> Result<(), AppError> {
        if let BlockType::Heading { level } = block_type {
            if !(*level >= 1 && *level <= 6) {
                return Err(AppError::BadRequest(format!(
                    "Heading level 必须在 1-6 之间，实际为 {}",
                    level
                )));
            }
        }
        Ok(())
    }

    fn on_self_or_descendant_move(
        conn: &rusqlite::Connection,
        block: &Block,
        _is_self: bool,
        _is_descendant: bool,
    ) -> Result<Option<Block>, AppError> {
        detach_heading_children(conn, block)?;
        Ok(Some(block.clone()))
    }

    fn on_moved(
        conn: &rusqlite::Connection,
        ctx: &MoveContext<'_>,
    ) -> Result<(), AppError> {
        let level = match ctx.block.block_type {
            BlockType::Heading { level } => level,
            _ => return Ok(()),
        };

        // 父块变化时：detach 子块（留在原父块下）
        if ctx.parent_changed {
            detach_heading_children(conn, ctx.block)?;
        }

        // 吸收后续同级节点
        absorb_siblings_after(
            conn,
            &ctx.block.id,
            ctx.target_parent_id,
            ctx.new_position,
            level,
            false,
        )?;

        Ok(())
    }

    fn on_type_changed(
        conn: &rusqlite::Connection,
        block_id: &str,
        old_block: &Block,
        new_type: &BlockType,
    ) -> Result<(), AppError> {
        let was_heading = matches!(old_block.block_type, BlockType::Heading { .. });
        let now_heading = matches!(new_type, BlockType::Heading { .. });

        // 曾经是 heading → 提升子块到父级
        if was_heading {
            promote_children(conn, block_id, &old_block.parent_id, &old_block.position)?;
        }

        // 变为 heading → 逃逸校验 + 吸收
        if now_heading {
            let new_level = match new_type {
                BlockType::Heading { level } => *level,
                _ => unreachable!(),
            };

            let (effective_parent_id, effective_position) =
                escape_heading_if_needed(conn, block_id, old_block, new_level)?;

            absorb_siblings_after(
                conn,
                block_id,
                &effective_parent_id,
                &effective_position,
                new_level,
                true,
            )?;
        }

        Ok(())
    }
}

// ─── Heading 辅助函数 ───────────────────────────────────────────

/// 将 heading 的直接子块 reparent 到 heading 的当前父块下
fn detach_heading_children(
    conn: &rusqlite::Connection,
    heading: &Block,
) -> Result<(), AppError> {
    let child_ids: Vec<String> = repo::find_children(conn, &heading.id)
        .map_err(|e| AppError::Internal(format!("查询 heading 子块失败: {}", e)))?
        .iter()
        .map(|c| c.id.clone())
        .collect();

    block::reparent_children_to(
        conn,
        &heading.parent_id,
        &heading.position,
        &child_ids,
        &heading.parent_id,
        false,
    )
}

/// 提升子块：将 heading 的所有直系子块 reparent 到 heading 的 parent
fn promote_children(
    conn: &rusqlite::Connection,
    heading_id: &str,
    heading_parent_id: &str,
    heading_position: &str,
) -> Result<(), AppError> {
    let child_ids: Vec<String> = repo::find_children(conn, heading_id)
        .map_err(|e| AppError::Internal(format!("查询子块失败: {}", e)))?
        .iter()
        .map(|c| c.id.clone())
        .collect();

    block::reparent_children_to(
        conn,
        heading_parent_id,
        heading_position,
        &child_ids,
        heading_parent_id,
        true,
    )
}

/// Heading 逃逸校验
///
/// 检查 heading(N) 的父链是否存在 heading(M >= N)。
/// 如果存在，将当前块 reparent 到最近的合法祖先，
/// 定位在"逃逸链"中最外层 heading 之后。
///
/// 返回逃逸后的有效 (parent_id, position)。
fn escape_heading_if_needed(
    conn: &rusqlite::Connection,
    block_id: &str,
    current: &Block,
    new_level: u8,
) -> Result<(String, String), AppError> {
    let mut check_id = current.parent_id.clone();
    let mut escape_from_id = None;

    loop {
        let parent = repo::find_by_id(conn, &check_id)
            .map_err(|e| AppError::Internal(format!("查询祖先 {} 失败: {}", check_id, e)))?;

        match &parent.block_type {
            BlockType::Heading { level } if *level >= new_level => {
                escape_from_id = Some(parent.id.clone());
                check_id = parent.parent_id.clone();
            }
            _ => break,
        }
    }

    let Some(escape_id) = escape_from_id else {
        return Ok((current.parent_id.clone(), current.position.clone()));
    };

    let target_parent_id = check_id;

    let escape_block = repo::find_by_id(conn, &escape_id)
        .map_err(|e| AppError::Internal(format!("查询逃逸点 {} 失败: {}", escape_id, e)))?;

    let siblings_after_escape = repo::find_siblings_after(
        conn,
        &target_parent_id,
        &escape_block.position,
    )
    .map_err(|e| AppError::Internal(format!("查询逃逸点后续兄弟失败: {}", e)))?;

    let new_position = if let Some(first_after) = siblings_after_escape.first() {
        position::generate_between(&escape_block.position, &first_after.position)
    } else {
        position::generate_after(&escape_block.position)
    };

    let now = now_iso();
    let new_document_id = block::derive_document_id_from_parent(conn, &target_parent_id)?;
    repo::update_parent_position_document_id(
        conn,
        block_id,
        &target_parent_id,
        &new_position,
        &new_document_id,
        &now,
    )
    .map_err(|e| AppError::Internal(format!("逃逸 reparent 失败: {}", e)))?;

    Ok((target_parent_id, new_position))
}

/// 吸收后续兄弟节点
///
/// 在 (parent_id, position) 对应的层级下，将 heading 之后的所有低级别块
/// reparent 为 heading 的子块，直到遇到 heading(level <= heading_level)。
fn absorb_siblings_after(
    conn: &rusqlite::Connection,
    heading_id: &str,
    parent_id: &str,
    position: &str,
    heading_level: u8,
    update_document_id: bool,
) -> Result<(), AppError> {
    let siblings_after = repo::find_siblings_after(conn, parent_id, position)
        .map_err(|e| AppError::Internal(format!("查询后续兄弟失败: {}", e)))?;

    let new_document_id = if update_document_id {
        Some(block::derive_document_id_from_parent(conn, parent_id)?)
    } else {
        None
    };

    let mut pos = position::calculate_insert_position(conn, heading_id, None)?;
    let now = now_iso();

    for sibling in &siblings_after {
        match &sibling.block_type {
            BlockType::Heading { level: sib_level } if *sib_level <= heading_level => break,
            _ => {
                if let Some(ref doc_id) = new_document_id {
                    repo::update_parent_position_document_id(
                        conn, &sibling.id, heading_id, &pos, doc_id, &now,
                    )
                } else {
                    repo::update_parent_position(
                        conn, &sibling.id, heading_id, &pos, &now,
                    )
                }
                .map_err(|e| AppError::Internal(format!("吸收失败: {}", e)))?;
                pos = position::generate_after(&pos);
            }
        }
    }

    Ok(())
}

// ─── Heading 子树移动 ──────────────────────────────────────────

struct HeadingTreeMove;

impl TreeMoveOps for HeadingTreeMove {
    fn validate_type(current: &Block) -> Result<(), AppError> {
        if !matches!(current.block_type, BlockType::Heading { .. }) {
            return Err(AppError::BadRequest(
                "move_heading_tree 只能移动 Heading 类型".to_string(),
            ));
        }
        Ok(())
    }

    fn resolve_target_parent(
        conn: &rusqlite::Connection,
        current_parent_id: &str,
        _target_parent_id: Option<&str>,
        before_id: &Option<String>,
        after_id: &Option<String>,
    ) -> Result<String, AppError> {
        block::resolve_target_parent(conn, before_id, after_id, current_parent_id)
    }

    fn pre_move(
        conn: &rusqlite::Connection,
        current: &Block,
        target_parent_id: &str,
    ) -> Result<Option<Block>, AppError> {
        if target_parent_id == current.id
            || repo::check_is_descendant(conn, &current.id, target_parent_id).unwrap_or(false)
        {
            detach_heading_children(conn, current)?;
            return Ok(Some(current.clone()));
        }
        Ok(None)
    }

    fn execute_move(
        conn: &rusqlite::Connection,
        id: &str,
        target_parent_id: &str,
        new_position: &str,
        _current: &Block,
    ) -> Result<u64, AppError> {
        let new_document_id = block::derive_document_id_from_parent(conn, target_parent_id)?;
        let now = now_iso();
        repo::update_parent_position_document_id(
            conn, id, target_parent_id, new_position, &new_document_id, &now,
        )
        .map_err(|e| AppError::Internal(format!("移动根块失败: {}", e)))
    }

    fn post_move(
        conn: &rusqlite::Connection,
        current: &Block,
        target_parent_id: &str,
        new_position: &str,
    ) -> Result<(), AppError> {
        let new_document_id = block::derive_document_id_from_parent(conn, target_parent_id)?;
        let cross_document = new_document_id != current.document_id;

        if cross_document {
            let descendant_ids = repo::find_descendant_ids(conn, &current.id)
                .map_err(|e| AppError::Internal(format!("查询子树后代失败: {}", e)))?;
            if !descendant_ids.is_empty() {
                repo::batch_update_document_id(
                    conn, &descendant_ids, &new_document_id, &now_iso(),
                )
                .map_err(|e| AppError::Internal(format!("更新后代 document_id 失败: {}", e)))?;
            }
        }

        let heading_level = match current.block_type {
            BlockType::Heading { level } => level,
            _ => unreachable!("validate_type 已验证是 Heading"),
        };
        absorb_siblings_after(
            conn, &current.id, target_parent_id, new_position, heading_level, false,
        )?;

        Ok(())
    }

    fn build_changes(
        conn: &rusqlite::Connection,
        op: &Operation,
        id: &str,
        current: &Block,
        after: &Block,
    ) -> Result<Vec<Change>, AppError> {
        let mut changes = vec![oplog::block_change_pair(
            &op.id, id, ChangeType::Moved, current, after,
        )];

        let new_document_id = block::derive_document_id_from_parent(conn, &after.parent_id)?;
        if new_document_id != current.document_id {
            let descendant_ids = repo::find_descendant_ids(conn, id)
                .map_err(|e| AppError::Internal(format!("查询后代失败: {}", e)))?;
            for did in &descendant_ids {
                if let Ok(desc_after) = repo::find_by_id_raw(conn, did) {
                    changes.push(oplog::new_change(
                        &op.id, did, ChangeType::Moved,
                        None,
                        Some(BlockSnapshot::from_block(&desc_after)),
                    ));
                }
            }
        }

        Ok(changes)
    }
}

/// 移动 Heading 子树（折叠拖拽场景）
pub fn move_heading_tree(db: &Db, req: MoveHeadingTreeReq) -> Result<Block, AppError> {
    block::move_tree::<HeadingTreeMove>(
        db, &req.id, req.editor_id, None, req.before_id, req.after_id,
    )
}
