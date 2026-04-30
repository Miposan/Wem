//! Paragraph 类型特化实现
//!
//! - BlockTypeOps trait 的 Paragraph 变体（目前使用默认实现）
//! - Paragraph 特有操作：split（拆分）、merge（合并）

use crate::dto::request::{MergeReq, SplitReq};
use crate::dto::response::{MergeResult, SplitResult};
use crate::error::AppError;
use crate::block_system::model::event::BlockEvent;
use crate::block_system::model::oplog::{Action, BlockSnapshot, ChangeType};
use crate::block_system::model::{generate_block_id, BlockType};
use crate::repo::block_repo as repo;
use crate::repo::block_repo::InsertBlockParams;
use crate::repo::Db;
use crate::util::now_iso;

use super::traits::BlockTypeOps;
use super::helpers::{self};
use super::{event, oplog, position};

/// Paragraph 类型行为实现
pub struct ParagraphOps;

impl BlockTypeOps for ParagraphOps {}

// ─── Split ─────────────────────────────────────────────────────

/// 拆分 Block（原子操作）
///
/// 在单个锁范围内完成「更新当前块内容 + 创建新块」两步操作，
/// 保证数据一致性：要么全部成功，要么全部失败。
///
/// 流程：
/// 1. 查询当前 Block
/// 2. 更新当前块 content = content_before
/// 3. 创建新块（content = content_after，位于当前块之后，同父块）
/// 4. 返回更新后的原块和新块
pub fn split_block(db: &Db, id: &str, req: SplitReq) -> Result<SplitResult, AppError> {
    let editor_id = req.editor_id.clone();
    let conn = crate::repo::lock_db(db);

    let result = helpers::run_in_transaction(&conn, || split_block_inner(&conn, id, req))?;

    let doc_id = result.updated_block.document_id.clone();
    event::EventBus::global().emit(BlockEvent::BlockUpdated {
        document_id: doc_id.clone(),
        editor_id: editor_id.clone(),
        block: result.updated_block.clone(),
    });
    event::EventBus::global().emit(BlockEvent::BlockCreated {
        document_id: doc_id,
        editor_id,
        block: result.new_block.clone(),
    });

    Ok(result)
}

fn split_block_inner(
    conn: &rusqlite::Connection,
    id: &str,
    req: SplitReq,
) -> Result<SplitResult, AppError> {

    // 1. 查询当前 Block
    let current = repo::find_by_id(&conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

    // 2. 更新当前块 content
    let now = now_iso();
    let new_content = req.content_before.into_bytes();
    let properties_json = helpers::to_json(&current.properties);

    let rows = repo::update_block_fields(
        &conn, id, &new_content, &properties_json, None, &now, Some(current.version),
    )
    .map_err(|e| AppError::Internal(format!("更新 Block 失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::VersionConflict(format!(
            "Block {} 版本冲突（期望版本 {}）", id, current.version
        )));
    }

    // 3. 查询更新后的原块
    let updated_block = repo::find_by_id_raw(&conn, id)
        .map_err(|e| AppError::Internal(format!("查询更新后的 Block 失败: {}", e)))?;

    // 4. 计算新块的 position 和 parent_id
    //    nest_under_parent = true 时，新块作为当前块的第一个子块（heading Enter）
    //    否则作为当前块的兄弟插入到后面
    let (new_parent_id, position) = if req.nest_under_parent.unwrap_or(false) {
        let pos = position::calculate_insert_position(&conn, id, None)?;
        (id.to_string(), pos)
    } else {
        let pos = position::calculate_insert_position(&conn, &current.parent_id, Some(id))?;
        (current.parent_id.clone(), pos)
    };

    // 5. 确定新块的 block_type
    //    前端未指定时，后端根据上下文推断：
    //    - 在 List 下的块（ListItem）→ 新块也是 ListItem
    //    - 其他 → 默认 Paragraph
    let new_block_type = req.new_block_type.unwrap_or_else(|| {
        // 检查 parent 是否为 List 类型，如果是则新块默认为 ListItem
        if let Ok(parent) = repo::find_by_id(&conn, &new_parent_id) {
            if matches!(parent.block_type, BlockType::List { .. }) {
                return BlockType::ListItem;
            }
        }
        BlockType::Paragraph
    });

    // 6. 创建新块
    let new_id = generate_block_id();
    let document_id = helpers::derive_document_id(&current);

    repo::insert_block(&conn, &InsertBlockParams {
        id: new_id.clone(),
        parent_id: new_parent_id,
        document_id,
        position,
        block_type: helpers::to_json(&new_block_type),
        content: req.content_after.into_bytes(),
        properties: "{}".to_string(),
        version: 1,
        status: "normal".to_string(),
        schema_version: 1,
        author: "system".to_string(),
        owner_id: None,
        encrypted: false,
        created: now_iso(),
        modified: now_iso(),
    })
    .map_err(|e| AppError::Internal(format!("插入新 Block 失败: {}", e)))?;

    // 7. 查询新创建的 Block
    let new_block = repo::find_by_id_raw(&conn, &new_id)
        .map_err(|e| AppError::Internal(format!("查询新创建的 Block 失败: {}", e)))?;

    // 8. 记录历史：拆分 = 更新原块 + 创建新块
    let op = oplog::new_operation(Action::Split, &current.document_id, req.editor_id.clone());
    let changes = vec![
        oplog::block_change_pair(
            &op.id, id, ChangeType::Updated, &current, &updated_block,
        ),
        oplog::new_change(
            &op.id, &new_id, ChangeType::Created,
            None,
            Some(BlockSnapshot::from_block(&new_block)),
        ),
    ];
    oplog::record_operation(&conn, &op, &changes)?;

    Ok(SplitResult {
        updated_block,
        new_block,
    })
}

// ─── Merge ─────────────────────────────────────────────────────

/// 合并 Block 到前一个兄弟（原子操作）
///
/// 在单个锁范围内完成「查找前一个兄弟 + 合并内容 + 删除当前块」三步操作。
///
/// 流程：
/// 1. 查询当前 Block
/// 2. 查找前一个兄弟 Block（同 parent_id，position < 当前 position）
/// 3. 合并内容：prev.content + current.content
/// 4. 更新前一个兄弟块
/// 5. 软删除当前块
/// 6. 返回合并后的块和被删除的块 ID
pub fn merge_block(db: &Db, id: &str, req: MergeReq) -> Result<MergeResult, AppError> {
    let editor_id = req.editor_id.clone();
    let conn = crate::repo::lock_db(db);

    let result = helpers::run_in_transaction(&conn, || merge_block_inner(&conn, id, &editor_id))?;

    let doc_id = result.merged_block.document_id.clone();
    event::EventBus::global().emit(BlockEvent::BlockUpdated {
        document_id: doc_id.clone(),
        editor_id: editor_id.clone(),
        block: result.merged_block.clone(),
    });
    event::EventBus::global().emit(BlockEvent::BlockDeleted {
        document_id: doc_id,
        editor_id,
        block_id: result.deleted_block_id.clone(),
        cascade_count: 0,
    });

    Ok(result)
}

fn merge_block_inner(
    conn: &rusqlite::Connection,
    id: &str,
    editor_id: &Option<String>,
) -> Result<MergeResult, AppError> {

    // 1. 查询当前 Block
    let current = repo::find_by_id(&conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

    // 全局根块不可合并
    if id == crate::block_system::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可合并".to_string()));
    }

    // 2. 确定合并目标：优先前驱兄弟，回退到父块
    let prev_sibling = repo::find_prev_sibling(&conn, &current.parent_id, &current.position)
        .map_err(|e| AppError::Internal(format!("查询前驱兄弟失败: {}", e)))?;

    let (target, merge_into_parent) = match prev_sibling {
        Some(s) => (s, false),
        None => {
            if current.parent_id == crate::block_system::model::ROOT_ID {
                return Err(AppError::BadRequest(format!(
                    "Block {} 是根块的第一个子块，无法合并",
                    id
                )));
            }
            let parent = repo::find_by_id(&conn, &current.parent_id).map_err(|_| {
                AppError::NotFound(format!("父块 {} 不存在", current.parent_id))
            })?;
            (parent, true)
        }
    };

    // 2.5 类型约束：不能合并到 List 容器块（List 不应有 content）
    //    List 容器块的 content 始终为空，合并文本进去违反数据模型。
    //    正确的语义是“退出列表”，由前端通过 delete + create 组合实现。
    if matches!(target.block_type, BlockType::List { .. }) {
        return Err(AppError::BadRequest(
            "不能合并到 List 容器块。ListItem 在列表首位时应使用“退出列表”操作".to_string()
        ));
    }

    // 3. 合并内容
    let target_text = String::from_utf8_lossy(&target.content);
    let current_text = String::from_utf8_lossy(&current.content);
    let merged_content = format!("{}{}", target_text, current_text);

    // 4. 更新合并目标块
    let now = now_iso();
    let properties_json = helpers::to_json(&target.properties);

    let rows = repo::update_block_fields(
        &conn,
        &target.id,
        merged_content.as_bytes(),
        &properties_json,
        None,
        &now,
        Some(target.version),
    )
    .map_err(|e| AppError::Internal(format!("更新合并目标块失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::VersionConflict(format!(
            "合并目标 Block {} 版本冲突（期望版本 {}）", target.id, target.version
        )));
    }

    // 5. 将当前块的子块 reparent 到合并目标
    let children = repo::find_children(&conn, id)
        .map_err(|e| AppError::Internal(format!("查询子块失败: {}", e)))?;

    if !children.is_empty() {
        let reparent_target_id = &target.id;
        let new_document_id = helpers::derive_document_id_from_parent(conn, reparent_target_id)?;

        let siblings_after =
            repo::find_siblings_after(&conn, &current.parent_id, &current.position)
                .map_err(|e| AppError::Internal(format!("查询后续兄弟失败: {}", e)))?;

        let mut pos = if merge_into_parent {
            if let Some(first_after) = siblings_after.first() {
                position::generate_between(&current.position, &first_after.position)?
            } else {
                position::generate_after(&current.position)
            }
        } else {
            let max_pos = repo::get_max_position(&conn, reparent_target_id)
                .map_err(|e| AppError::Internal(format!("查询最大 position 失败: {}", e)))?;
            match max_pos {
                Some(mp) => position::generate_after(&mp),
                None => position::generate_first(),
            }
        };

        for child in &children {
            let t = now_iso();
            repo::update_parent_position_document_id(
                &conn, &child.id, reparent_target_id, &pos, &new_document_id, &t, None,
            )
            .map_err(|e| AppError::Internal(format!("Reparent 子块失败: {}", e)))?;
            pos = position::generate_after(&pos);
        }
    }

    // 6. 软删除当前块
    repo::update_status(&conn, id, "deleted", &now)
        .map_err(|e| AppError::Internal(format!("软删除当前块失败: {}", e)))?;

    // 7. 查询合并后的块
    let merged_block = repo::find_by_id_raw(&conn, &target.id)
        .map_err(|e| AppError::Internal(format!("查询合并后的 Block 失败: {}", e)))?;

    // 8. 记录历史：合并 = 更新目标块 + 删除当前块
    let op = oplog::new_operation(Action::Merge, &current.document_id, editor_id.clone());
    let changes = vec![
        oplog::block_change_pair(
            &op.id, &target.id, ChangeType::Updated, &target, &merged_block,
        ),
        oplog::new_change(
            &op.id, id, ChangeType::Deleted,
            Some(BlockSnapshot::from_block(&current)),
            None,
        ),
    ];
    oplog::record_operation(&conn, &op, &changes)?;

    Ok(MergeResult {
        merged_block,
        deleted_block_id: id.to_string(),
    })
}
