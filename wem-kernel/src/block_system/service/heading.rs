//! Heading 类型特化实现
//!
//! 1. BlockTypeOps 生命周期钩子：
//!    - validate_on_create: 校验 heading level 1-6
//!    - on_moved: 跨文档移动后 flat-list 重建
//!    - on_type_changed: heading 变更后 flat-list 重建
//!
//! 2. Flat-list 树操作：
//!    - build_flat_list: 前序遍历构建文档 flat list
//!    - reconstruct_tree: 栈算法重建 heading 层级树
//!    - find_subtree_end: 在 flat list 中定位子树边界
//!    - move_heading_flat: 展开状态下移动单个 heading
//!    - move_heading_tree: 折叠状态下移动整个子树
//!
//! 3. 辅助函数：
//!    - heading_level: 从 Block 提取 heading level

use std::collections::HashMap;

use crate::api::request::MoveHeadingTreeReq;
use crate::error::AppError;
use crate::block_system::model::event::BlockEvent;
use crate::block_system::model::oplog::{BlockSnapshot, ChangeType};
use crate::block_system::model::{Block, BlockType};
use crate::repo::block_repo as repo;
use crate::repo::Db;
use crate::util::now_iso;

use super::traits::{BlockTypeOps, MoveContext};
use super::helpers::{self, run_in_transaction};
use super::{oplog, position};

/// Heading 类型行为实现
pub struct HeadingOps;

impl BlockTypeOps for HeadingOps {
    fn use_flat_list_move() -> bool { true }

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

    fn on_moved(
        conn: &rusqlite::Connection,
        ctx: &MoveContext<'_>,
    ) -> Result<(), AppError> {
        // 同文档移动由 move_heading_flat 的 reconstruct_tree 处理，不会走到这里。
        // 这里只处理跨文档移动：用 flat-list 重建新文档的树。
        if ctx.parent_changed {
            let new_document_id = helpers::derive_document_id_from_parent(conn, ctx.target_parent_id)?;
            if new_document_id != ctx.block.document_id {
                let flat = build_flat_list(conn, &new_document_id)?;
                reconstruct_tree(conn, &new_document_id, &flat)?;
            }
        }

        Ok(())
    }

    fn on_type_changed(
        conn: &rusqlite::Connection,
        _block_id: &str,
        old_block: &Block,
        _new_type: &BlockType,
    ) -> Result<(), AppError> {
        let was_or_now_heading = matches!(old_block.block_type, BlockType::Heading { .. })
            || matches!(_new_type, BlockType::Heading { .. });
        if was_or_now_heading {
            let flat = build_flat_list(conn, &old_block.document_id)?;
            reconstruct_tree(conn, &old_block.document_id, &flat)?;
        }
        Ok(())
    }
}

fn heading_level(block: &Block) -> u8 {
    match &block.block_type {
        BlockType::Heading { level } => *level,
        _ => 0,
    }
}

// ─── Flat-list 移动 ────────────────────────────────────────────

/// 在 flat list 中找到 block 及其完整子树的末尾索引。
///
/// Heading: 子树末尾 = 下一个 level <= 自身 level 的 heading 之前（或 list 末尾）。
/// 非 Heading: 通过 parent_id 链追踪，连续后代都算子树。
fn find_subtree_end(flat: &[Block], start_idx: usize) -> usize {
    if let BlockType::Heading { level } = &flat[start_idx].block_type {
        let level = *level;
        flat[start_idx + 1..].iter()
            .position(|b| matches!(&b.block_type, BlockType::Heading { level: l } if *l <= level))
            .map(|p| start_idx + 1 + p)
            .unwrap_or(flat.len())
    } else {
        let mut subtree_ids = std::collections::HashSet::new();
        subtree_ids.insert(flat[start_idx].id.clone());
        let mut end = start_idx + 1;
        while end < flat.len() {
            if subtree_ids.contains(&flat[end].parent_id) {
                subtree_ids.insert(flat[end].id.clone());
                end += 1;
            } else {
                break;
            }
        }
        end
    }
}

/// 基于 flat-list 模型移动 heading。
///
/// 模型：文档的块按前序遍历展开为 flat list，
/// 移动 heading 就是在 flat list 中改变它的位置，
/// 然后根据 heading 层级重新推导父子关系。
///
/// 父块推导规则（栈算法）：
/// - Heading(level N)：向前找最近的 level < N 的 heading 作为父块
/// - 非 heading 块：父块是前面最近的 heading（或文档根）
pub fn move_heading_flat(
    conn: &rusqlite::Connection,
    heading: &Block,
    before_id: Option<&str>,
    after_id: Option<&str>,
) -> Result<Block, AppError> {
    let doc_id = &heading.document_id;

    // 1. 构建 flat list（前序遍历）
    let mut flat = build_flat_list(conn, doc_id)?;

    // 2. 从 flat list 移除 heading
    let heading_idx = flat.iter().position(|b| b.id == heading.id)
        .ok_or_else(|| AppError::Internal("Heading 不在 flat list 中".into()))?;
    flat.remove(heading_idx);

    // 3. 确定插入位置
    let insert_idx = match (before_id, after_id) {
        (Some(bid), _) => flat.iter().position(|b| b.id == bid).unwrap_or(flat.len()),
        (_, Some(aid)) => {
            let idx = flat.iter().position(|b| b.id == aid).unwrap_or(flat.len());
            if idx < flat.len() { find_subtree_end(&flat, idx) } else { flat.len() }
        }
        _ => flat.len(),
    };

    // 4. 插入 heading 到目标位置
    flat.insert(insert_idx, heading.clone());

    // 5. 重建树
    reconstruct_tree(conn, doc_id, &flat)?;

    // 6. 返回更新后的 heading
    repo::find_by_id_raw(conn, &heading.id)
        .map_err(|e| AppError::Internal(format!("查询更新后的 heading 失败: {}", e)))
}

/// 构建文档的 flat list（前序遍历）
fn build_flat_list(
    conn: &rusqlite::Connection,
    doc_id: &str,
) -> Result<Vec<Block>, AppError> {
    let all = repo::find_descendants(conn, doc_id)
        .map_err(|e| AppError::Internal(format!("查询文档块失败: {}", e)))?;

    let mut cm: HashMap<String, Vec<Block>> = HashMap::new();
    for b in &all {
        cm.entry(b.parent_id.clone()).or_default().push(b.clone());
    }
    for v in cm.values_mut() {
        v.sort_by(|a, b| a.position.cmp(&b.position));
    }

    let mut flat = Vec::with_capacity(all.len());
    fn traverse(pid: &str, cm: &HashMap<String, Vec<Block>>, out: &mut Vec<Block>) {
        if let Some(ch) = cm.get(pid) {
            for c in ch {
                out.push(c.clone());
                traverse(&c.id, cm, out);
            }
        }
    }
    traverse(doc_id, &cm, &mut flat);
    Ok(flat)
}

/// 从 flat list 重建 heading 树。
///
/// 1. 栈算法确定每个块的正确父块
/// 2. 按 correct parent 分组，每组内保持 flat list 顺序
/// 3. 每组从当前 max position 之后统一分配新 position（杜绝冲突）
/// 4. 只写回 parent 或 position 实际变化的块
fn reconstruct_tree(
    conn: &rusqlite::Connection,
    doc_id: &str,
    flat: &[Block],
) -> Result<(), AppError> {
    // 1. 栈算法：确定每个块的正确父块
    let mut stack: Vec<(&str, u8)> = Vec::new();
    let mut correct_parents: Vec<&str> = Vec::with_capacity(flat.len());

    for block in flat {
        match &block.block_type {
            BlockType::Heading { level } => {
                while let Some((_, sl)) = stack.last() {
                    if *sl < *level { break; }
                    stack.pop();
                }
                correct_parents.push(stack.last().map(|(id, _)| *id).unwrap_or(doc_id));
                stack.push((&block.id, *level));
            }
            _ => {
                correct_parents.push(stack.last().map(|(id, _)| *id).unwrap_or(doc_id));
            }
        }
    }

    // 2. 按 correct parent 分组
    let mut groups: HashMap<&str, Vec<&Block>> = HashMap::new();
    for (i, block) in flat.iter().enumerate() {
        groups.entry(correct_parents[i]).or_default().push(block);
    }

    // 3 & 4. 每组：从 max position 之后分配，避免与现有 position 冲突
    let now = now_iso();
    for (parent_id, children) in &groups {
        let new_doc_id = helpers::derive_document_id_from_parent(conn, parent_id)?;

        // 读取该父块下当前的 max position，新 position 从它之后开始
        let max_pos = repo::get_max_position(conn, parent_id)
            .map_err(|e| AppError::Internal(format!("查询最大位置失败: {}", e)))?;
        let mut pos = match &max_pos {
            Some(mp) => position::generate_after(mp),
            None => position::generate_first(),
        };

        for block in children {
            let parent_changed = block.parent_id != *parent_id;
            let pos_changed = block.position != pos;

            if parent_changed || pos_changed {
                repo::update_parent_position_document_id(
                    conn, &block.id, parent_id, &pos, &new_doc_id, &now,
                )
                .map_err(|e| AppError::Internal(format!("重建树失败: {}", e)))?;
            }
            pos = position::generate_after(&pos);
        }
    }

    Ok(())
}

// ─── Heading 子树移动（折叠拖拽） ─────────────────────────────────

/// 移动 Heading 子树（折叠拖拽场景）
///
/// 与 move_heading_flat 的区别：子树整体移动（heading + 所有后代一起），
/// 而非只移动 heading 本身。
///
/// 算法：
/// 1. 构建 flat list
/// 2. 从 flat list 移除 heading 及其所有后代（前序遍历中连续的子串）
/// 3. 在目标位置重新插入这整个子串
/// 4. reconstruct_tree 重建整棵树
pub fn move_heading_tree(db: &Db, req: MoveHeadingTreeReq) -> Result<Block, AppError> {
    let id = &req.id;
    let editor_id = req.editor_id.clone();

    let conn = crate::repo::lock_db(db);

    let result = run_in_transaction(&conn, || {
        let current = repo::find_by_id(&conn, id)
            .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

        if !matches!(current.block_type, BlockType::Heading { .. }) {
            return Err(AppError::BadRequest(
                "move_heading_tree 只能移动 Heading 类型".to_string(),
            ));
        }

        let doc_id = &current.document_id;

        // 1. 构建 flat list
        let mut flat = build_flat_list(&conn, doc_id)?;

        // 2. 找到 heading 及其子树在 flat list 中的范围
        let heading_idx = flat.iter().position(|b| b.id == *id)
            .ok_or_else(|| AppError::Internal("Heading 不在 flat list 中".into()))?;

        // 子树在前序遍历中是连续的，找子树末尾
        let heading_level = heading_level(&current);
        let subtree_end = flat[heading_idx + 1..].iter()
            .position(|b| {
                matches!(&b.block_type, BlockType::Heading { level } if *level <= heading_level)
            })
            .map(|p| heading_idx + 1 + p)
            .unwrap_or(flat.len());

        // 提取子树（heading + 所有后代）
        let subtree: Vec<Block> = flat.drain(heading_idx..subtree_end).collect();

        // 3. 确定插入位置
        let insert_idx = match (req.before_id.as_deref(), req.after_id.as_deref()) {
            (Some(bid), _) => flat.iter().position(|b| b.id == bid).unwrap_or(flat.len()),
            (_, Some(aid)) => {
                let idx = flat.iter().position(|b| b.id == aid).unwrap_or(flat.len());
                if idx < flat.len() { find_subtree_end(&flat, idx) } else { flat.len() }
            }
            _ => flat.len(),
        };

        // 4. 插入子树到目标位置
        for (i, block) in subtree.into_iter().enumerate() {
            flat.insert(insert_idx + i, block);
        }

        // 5. 重建树
        reconstruct_tree(&conn, doc_id, &flat)?;

        // 6. 记录 oplog
        let after = repo::find_by_id_raw(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询移动后的 Block 失败: {}", e)))?;

        let op = oplog::new_operation(
            crate::block_system::model::oplog::Action::Move, doc_id, editor_id,
        );
        let change = oplog::new_change(
            &op.id, id, ChangeType::Moved,
            Some(BlockSnapshot::from_block(&current)),
            Some(BlockSnapshot::from_block(&after)),
        );
        oplog::record_operation(&conn, &op, &[change])?;

        Ok(after)
    })?;

    crate::block_system::service::event::EventBus::global().emit(BlockEvent::BlockMoved {
        document_id: result.document_id.clone(),
        editor_id: req.editor_id,
        block: result.clone(),
    });

    Ok(result)
}
