//! 通用块操作 + 类型分派（L3 整合层）
//!
//! 对外提供 Block CRUD / Move / Export 的公共 API。
//! 通过分派函数 match BlockType 路由到 L2 的 trait impl。

use std::collections::HashMap;

use crate::api::request::{CreateBlockReq, MoveBlockReq, MoveDocumentTreeReq, UpdateBlockReq};
use crate::api::response::{
    DeleteResult, RestoreResult,
};
use crate::repo::block_repo as repo;
use crate::repo::block_repo::InsertBlockParams;
use crate::repo::Db;
use crate::error::AppError;
use crate::block_system::model::event::BlockEvent;
use crate::block_system::model::oplog::{Action, BlockSnapshot, ChangeType};
use crate::block_system::model::{generate_block_id, Block, BlockType};
use super::traits::{BlockTypeOps, TreeMoveOps};
pub use super::traits::{MoveContext, ExportDepth};
use super::helpers::{run_in_transaction, derive_document_id, validate_no_cycle, reparent_children_to, to_json, merge_properties};
use super::{event, oplog, position};
use crate::util::now_iso;

// ─── 类型分派 ───────────────────────────────────────────────────

use super::heading::HeadingOps;
use super::document::DocumentOps;
use super::paragraph::ParagraphOps;
use super::list::ListOps;

pub(crate) fn use_tree_move(block_type: &BlockType) -> bool {
    match block_type {
        BlockType::Document => DocumentOps::use_tree_move(),
        _ => ParagraphOps::use_tree_move(),
    }
}

pub(crate) fn use_flat_list_move(block_type: &BlockType) -> bool {
    match block_type {
        BlockType::Heading { .. } => HeadingOps::use_flat_list_move(),
        _ => ParagraphOps::use_flat_list_move(),
    }
}

/// 子树移动分派：将 move_block 请求路由到对应的子树移动实现
pub(crate) fn dispatch_tree_move(
    db: &Db,
    block_type: &BlockType,
    req: MoveBlockReq,
) -> Result<Block, AppError> {
    match block_type {
        BlockType::Document => super::document::move_document_tree(db, MoveDocumentTreeReq {
            editor_id: req.editor_id,
            id: req.id.clone(),
            target_parent_id: req.target_parent_id,
            before_id: req.before_id,
            after_id: req.after_id,
        }),
        _ => Err(AppError::BadRequest("不支持子树移动的类型".to_string())),
    }
}

/// Flat-list 移动分派：同文档内基于 before_id/after_id 的 heading 移动
pub(crate) fn dispatch_flat_list_move(
    conn: &rusqlite::Connection,
    block: &Block,
    before_id: Option<&str>,
    after_id: Option<&str>,
) -> Result<Block, AppError> {
    match &block.block_type {
        BlockType::Heading { .. } => super::heading::move_heading_flat(conn, block, before_id, after_id),
        _ => unreachable!(),
    }
}

pub(crate) fn validate_on_create(block_type: &BlockType) -> Result<(), AppError> {
    match block_type {
        BlockType::Heading { .. } => HeadingOps::validate_on_create(block_type),
        _ => Ok(()),
    }
}

pub(crate) fn on_moved(
    conn: &rusqlite::Connection,
    ctx: &MoveContext<'_>,
) -> Result<(), AppError> {
    match &ctx.block.block_type {
        BlockType::Heading { .. } => HeadingOps::on_moved(conn, ctx),
        BlockType::Document => DocumentOps::on_moved(conn, ctx),
        _ => ParagraphOps::on_moved(conn, ctx),
    }
}

pub(crate) fn adjust_content_on_update(
    conn: &rusqlite::Connection,
    block: &Block,
    content: &mut Vec<u8>,
) -> Result<(), AppError> {
    match &block.block_type {
        BlockType::Document => DocumentOps::adjust_content_on_update(conn, block, content),
        _ => ParagraphOps::adjust_content_on_update(conn, block, content),
    }
}

pub(crate) fn on_type_changed(
    conn: &rusqlite::Connection,
    block_id: &str,
    old_block: &Block,
    new_type: &BlockType,
) -> Result<(), AppError> {
    match (&old_block.block_type, new_type) {
        // 涉及 Heading 的转换
        (BlockType::Heading { .. }, _) | (_, BlockType::Heading { .. }) => {
            HeadingOps::on_type_changed(conn, block_id, old_block, new_type)
        }
        // 涉及 List 的转换（X → List 或 List → X）
        (BlockType::List { .. }, _) => {
            ListOps::on_type_changed(conn, block_id, old_block, new_type)
        }
        (_, BlockType::List { .. }) => {
            ListOps::on_type_changed(conn, block_id, old_block, new_type)
        }
        _ => Ok(()),
    }
}

// ─── 创建 Block ────────────────────────────────────────────────

/// 创建 Block
///
/// 流程：
/// 1. 验证 parent_id 存在且未删除
/// 2. 根据 after_id 计算 position（插入指定位置或追加末尾）
/// 3. 生成 20 位 ID + 时间戳
/// 4. INSERT INTO blocks
///
pub fn create_block(db: &Db, req: CreateBlockReq) -> Result<Block, AppError> {
    let editor_id = req.editor_id.clone();
    let conn = crate::repo::lock_db(db);

    let result = run_in_transaction(&conn, || {
        // 1. 校验 block_type 合法性
        validate_on_create(&req.block_type)?;

        // 2. 验证 parent 存在且未删除
        let parent = repo::find_by_id(&conn, &req.parent_id)
            .map_err(|_| AppError::BadRequest(format!("父块 {} 不存在或已删除", req.parent_id)))?;

        // 2.5 父子类型约束校验（ListItem 必须在 List 下、List 子块必须是 ListItem）
        super::list::validate_parent_child_constraint(&parent, &req.block_type)?;
        super::list::validate_list_child_type(&parent.block_type, &req.block_type)?;

        // 3. 推断 document_id
        let document_id = derive_document_id(&parent);

        // 4. 计算 position
        let position =
            position::calculate_insert_position(&conn, &req.parent_id, req.after_id.as_deref())?;

        // 5. 生成 ID 和时间戳
        let id = generate_block_id();
        let now = now_iso();

        // 6. INSERT（通过 repository）
        repo::insert_block(&conn, &InsertBlockParams {
            id: id.clone(),
            parent_id: req.parent_id,
            document_id: document_id.clone(),
            position,
            block_type: to_json(&req.block_type),
            content: req.content.into_bytes(),
            properties: to_json(&req.properties),
            version: 1,
            status: "normal".to_string(),
            schema_version: 1,
            author: "system".to_string(),
            owner_id: None,
            encrypted: false,
            created: now.clone(),
            modified: now,
        })
        .map_err(|e| AppError::Internal(format!("插入 Block 失败: {}", e)))?;

        // 8. 查询完整 Block
        let block = repo::find_by_id_raw(&conn, &id)
            .map_err(|e| AppError::Internal(format!("查询刚创建的 Block 失败: {}", e)))?;

        // 9. 记录操作历史
        let op = oplog::new_operation(Action::Create, &document_id, req.editor_id.clone());
        let change = oplog::new_change(
            &op.id, &id, ChangeType::Created,
            None,
            Some(BlockSnapshot::from_block(&block)),
        );
        oplog::record_operation(&conn, &op, &[change])?;

        Ok(block)
    })?;

    event::EventBus::global().emit(BlockEvent::BlockCreated {
        document_id: result.document_id.clone(),
        editor_id,
        block: result.clone(),
    });

    Ok(result)
}

// ─── 查询 Block ────────────────────────────────────────────────

/// 获取单个 Block
///
/// `include_deleted` 为 true 时也返回已软删除的块，默认只返回正常块。
pub fn get_block(db: &Db, id: &str, include_deleted: bool) -> Result<Block, AppError> {
    let conn = crate::repo::lock_db(db);
    let result = if include_deleted {
        repo::find_by_id_raw(&conn, id)
    } else {
        repo::find_by_id(&conn, id)
    };
    result.map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))
}

// ─── 更新 Block ────────────────────────────────────────────────

/// 更新 Block 内容和/或属性
///
/// 流程：
/// 1. 查询当前 Block
/// 2. 计算新的 content / properties
/// 3. `UPDATE ... SET version=version+1 WHERE id=?`
/// 4. 返回更新后的 Block
///
/// 参考 03-api-rest.md §3 "更新 Block"
pub fn update_block(db: &Db, id: &str, req: UpdateBlockReq) -> Result<Block, AppError> {
    let editor_id = req.editor_id.clone();
    let conn = crate::repo::lock_db(db);

    // 校验 block_type 合法性
    if let Some(ref bt) = req.block_type {
        validate_on_create(bt)?;
    }

    let result = run_in_transaction(&conn, || update_block_inner(&conn, id, req))?;

    event::EventBus::global().emit(BlockEvent::BlockUpdated {
        document_id: result.document_id.clone(),
        editor_id,
        block: result.clone(),
    });

    Ok(result)
}

/// update_block 的核心逻辑（在事务内执行）
fn update_block_inner(
    conn: &rusqlite::Connection,
    id: &str,
    req: UpdateBlockReq,
) -> Result<Block, AppError> {

    // 1. 查询当前 Block
    let current = repo::find_by_id(&conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;
    // 2. 计算新 content
    let mut new_content: Vec<u8> = req
        .content
        .map(|c| c.into_bytes())
        .unwrap_or_else(|| current.content.clone());

    // 2.5 类型特化内容调整
    adjust_content_on_update(&conn, &current, &mut new_content)?;

    // 3. 计算新 block_type
    let new_block_type = req.block_type.clone().unwrap_or(current.block_type.clone());

    // 4. 计算新 properties（merge 或 replace）
    let new_properties = merge_properties(&current.properties, req.properties.as_ref(), &req.properties_mode);
    let properties_json = to_json(&new_properties);

    // 5. 写入数据库
    write_block_updates(conn, id, &req.block_type, &new_content, &properties_json, &new_block_type, current.version)?;

    // 6. 类型变更后处理（仅 block_type 变化时）
    if req.block_type.is_some() {
        on_type_changed(conn, id, &current, &new_block_type)?;
    }

    // 7. 查询并返回更新后的 Block
    let new_block = repo::find_by_id_raw(conn, id)
        .map_err(|e| AppError::Internal(format!("查询更新后的 Block 失败: {}", e)))?;

    // 8. 记录操作历史
    let op = oplog::new_operation(Action::Update, &current.document_id, req.editor_id.clone());
    let change = oplog::block_change_pair(
        &op.id, id, ChangeType::Updated, &current, &new_block,
    );
    oplog::record_operation(conn, &op, &[change])?;

    Ok(new_block)
}

// ─── update_block_inner 子步骤 ─────────────────────────────────

/// 步骤 5：将字段更新写入数据库
fn write_block_updates(
    conn: &rusqlite::Connection,
    id: &str,
    block_type_req: &Option<BlockType>,
    new_content: &[u8],
    properties_json: &str,
    new_block_type: &BlockType,
    expected_version: u64,
) -> Result<(), AppError> {
    let now = now_iso();
    let block_type_changed = block_type_req.is_some();

    let rows = if block_type_changed {
        let bt_str = to_json(new_block_type);
        repo::update_block_fields(
            conn, id, new_content, properties_json,
            Some(&bt_str), &now, Some(expected_version),
        )
    } else {
        repo::update_block_fields(
            conn, id, new_content, properties_json,
            None, &now, Some(expected_version),
        )
    }
    .map_err(|e| AppError::Internal(format!("更新 Block 失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::VersionConflict(format!(
            "Block {} 版本冲突（期望版本 {}）", id, expected_version
        )));
    }

    Ok(())
}

// ─── 共享子块 reparent 辅助 ──────────────────────────────────

// ─── 删除 Block ──────────────────────────────────────────────────

/// 删除单个 Block（子块提升到父级）
///
/// 只软删除目标块本身，其子块 reparent 到被删块的父级，保持原有顺序。
/// 用于编辑器 Backspace 删除空块等场景。
pub fn delete_block(db: &Db, id: &str, editor_id: Option<String>) -> Result<DeleteResult, AppError> {
    if id == crate::block_system::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可删除".to_string()));
    }

    let editor_id_for_event = editor_id.clone();
    let conn = crate::repo::lock_db(db);

    let result = run_in_transaction(&conn, || -> Result<DeleteResult, AppError> {
        let current = repo::find_by_id(&conn, id)
            .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

        let document_id = current.document_id.clone();
        let parent_id_before_delete = current.parent_id.clone();
        let deleted_list_item = matches!(current.block_type, BlockType::ListItem);

        // 将子块 reparent 到被删块的父级，保持原有顺序
        let child_ids: Vec<String> = repo::find_children(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询子块失败: {}", e)))?
            .iter().map(|c| c.id.clone()).collect();

        if !child_ids.is_empty() {
            reparent_children_to(
                &conn, &current.parent_id, &current.position, &child_ids, &current.parent_id, true,
            )?;
        }

        // 软删除目标块
        let now = now_iso();
        repo::update_status(&conn, id, "deleted", &now)
            .map_err(|e| AppError::Internal(format!("软删除失败: {}", e)))?;

        if deleted_list_item {
            let _ = super::list::cleanup_empty_list(&conn, &parent_id_before_delete)?;
        }

        let new_version = repo::get_version(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))?;

        // 记录操作历史
        let op = oplog::new_operation(Action::Delete, &document_id, editor_id);
        let change = oplog::new_change(
            &op.id, id, ChangeType::Deleted,
            Some(BlockSnapshot::from_block(&current)),
            None,
        );
        oplog::record_operation(&conn, &op, &[change])?;

        Ok(DeleteResult {
            id: id.to_string(),
            document_id,
            version: new_version,
            cascade_count: 0,
        })
    })?;

    event::EventBus::global().emit(BlockEvent::BlockDeleted {
        document_id: result.document_id.clone(),
        editor_id: editor_id_for_event,
        block_id: result.id.clone(),
        cascade_count: result.cascade_count,
    });

    Ok(result)
}

/// 级联删除 Block 及其所有后代
///
/// 用递归 CTE 查询所有后代（含自身），批量软删除。
pub fn delete_tree(db: &Db, id: &str, editor_id: Option<String>) -> Result<DeleteResult, AppError> {
    if id == crate::block_system::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可删除".to_string()));
    }

    let editor_id_for_event = editor_id.clone();
    let conn = crate::repo::lock_db(db);

    let result = run_in_transaction(&conn, || -> Result<DeleteResult, AppError> {
        let _current = repo::find_by_id(&conn, id)
            .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

        let descendant_ids = repo::find_descendant_ids_include_self(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询后代失败: {}", e)))?;

        let cascade_count = descendant_ids.len().saturating_sub(1) as u32;

        let before_blocks: Vec<Block> = descendant_ids.iter()
            .filter_map(|did| repo::find_by_id_raw(&conn, did).ok())
            .collect();

        let now = now_iso();
        repo::batch_update_status_if_not(&conn, &descendant_ids, "deleted", &now, "deleted")
            .map_err(|e| AppError::Internal(format!("批量软删除失败: {}", e)))?;

        let new_version = repo::get_version(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))?;

        let document_id = before_blocks.first().map(|b| b.document_id.clone()).unwrap_or_default();
        let op = oplog::new_operation(Action::Delete, &document_id, editor_id);
        let changes: Vec<_> = before_blocks.iter().map(|b| {
            oplog::new_change(
                &op.id, &b.id, ChangeType::Deleted,
                Some(BlockSnapshot::from_block(b)),
                None,
            )
        }).collect();
        oplog::record_operation(&conn, &op, &changes)?;

        Ok(DeleteResult {
            id: id.to_string(),
            document_id,
            version: new_version,
            cascade_count,
        })
    })?;

    event::EventBus::global().emit(BlockEvent::BlockDeleted {
        document_id: result.document_id.clone(),
        editor_id: editor_id_for_event,
        block_id: result.id.clone(),
        cascade_count: result.cascade_count,
    });

    Ok(result)
}

// ─── 恢复 Block ────────────────────────────────────────────────

/// 恢复已软删除的 Block 及其所有后代
///
/// 前置条件：
/// - 目标 Block 当前状态为 `deleted`
/// - 父块不能是 `deleted`（否则需要先恢复父块）
///
pub fn restore_block(db: &Db, id: &str, editor_id: Option<String>) -> Result<RestoreResult, AppError> {
    let editor_id_for_event = editor_id.clone();
    let conn = crate::repo::lock_db(db);

    let (result, restored_block): (RestoreResult, Block) = run_in_transaction(&conn, || {
        // 1. 查询当前 Block（必须是 deleted 状态）
        let current = repo::find_deleted(&conn, id)
            .map_err(|_| AppError::BadRequest(format!("Block {} 不是已删除状态", id)))?;

        // 2. 检查父块状态（根文档 parent_id == id，跳过检查）
        if current.parent_id != current.id {
            let parent_status = repo::get_status(&conn, &current.parent_id)
                .unwrap_or_else(|_| "deleted".to_string());

            if parent_status == "deleted" {
                return Err(AppError::BadRequest(format!(
                    "父块 {} 已被删除，请先恢复父块",
                    current.parent_id
                )));
            }
        }

        // 3. 递归 CTE 查所有已删除的后代
        let descendant_ids = repo::find_deleted_descendant_ids(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询已删除后代失败: {}", e)))?;

        let cascade_count = descendant_ids.len() as u32;

        // 4. 收集所有将被恢复的块 ID（自身 + 后代）
        let all_restore_ids = {
            let mut ids = vec![id.to_string()];
            ids.extend(descendant_ids.iter().cloned());
            ids
        };

        // 5. 捕获恢复前的快照（deleted 状态）
        let before_blocks: Vec<Block> = all_restore_ids.iter()
            .filter_map(|did| repo::find_by_id_raw(&conn, did).ok())
            .collect();

        // 6. 恢复自身
        let now = now_iso();
        repo::update_status(&conn, id, "normal", &now)
            .map_err(|e| AppError::Internal(format!("恢复 Block 失败: {}", e)))?;

        // 7. 恢复后代（仅恢复 status='deleted' 的）
        repo::batch_update_status_if(&conn, &descendant_ids, "normal", &now, "deleted")
            .map_err(|e| AppError::Internal(format!("恢复后代 Block 失败: {}", e)))?;

        // 8. 获取更新后的 version
        let new_version = repo::get_version(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))?;

        // 9. 捕获恢复后的快照（normal 状态）
        let after_blocks: Vec<Block> = all_restore_ids.iter()
            .filter_map(|did| repo::find_by_id_raw(&conn, did).ok())
            .collect();

        // 10. 记录操作历史（每个恢复的块一条 Change）
        let document_id = before_blocks.first().map(|b| b.document_id.clone()).unwrap_or_default();
        let op = oplog::new_operation(Action::Restore, &document_id, editor_id);
        let after_map: HashMap<&str, &Block> = after_blocks.iter()
            .map(|b| (b.id.as_str(), b))
            .collect();
        let changes: Vec<_> = before_blocks.iter()
            .filter_map(|before| {
                let after = after_map.get(before.id.as_str())?;
                Some(oplog::block_change_pair(
                    &op.id, &before.id, ChangeType::Restored, before, after,
                ))
            })
            .collect();
        oplog::record_operation(&conn, &op, &changes)?;

        let restored = repo::find_by_id_raw(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询恢复后的 Block 失败: {}", e)))?;

        Ok((RestoreResult {
            id: id.to_string(),
            document_id,
            version: new_version,
            cascade_count,
        }, restored))
    })?;

    event::EventBus::global().emit(BlockEvent::BlockRestored {
        document_id: result.document_id.clone(),
        editor_id: editor_id_for_event,
        block: restored_block,
    });

    Ok(result)
}

// ─── 移动 Block ────────────────────────────────────────────────

/// 移动 Block 到新的父块和/或位置
///
/// 流程：
/// 1. 版本校验
/// 2. 如果切换父块 → 循环引用检测（target_parent 不能是 block 的后代）
/// 3. 根据 before_id / after_id 计算新 position
/// 4. UPDATE → 返回更新后的 Block
///
pub fn move_block(db: &Db, id: &str, req: MoveBlockReq) -> Result<Block, AppError> {
    if id == crate::block_system::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可移动".to_string()));
    }

    let editor_id = req.editor_id.clone();

    // 类型特化：某些块类型需要使用子树移动
    {
        let conn = crate::repo::lock_db(db);
        let block_type = repo::find_by_id(&conn, id)
            .map(|b| b.block_type.clone())
            .unwrap_or(BlockType::Paragraph);
        drop(conn);

        if use_tree_move(&block_type) {
            return dispatch_tree_move(db, &block_type, req);
        }
    }

    let conn = crate::repo::lock_db(db);

    let result = run_in_transaction(&conn, || {
        let current = repo::find_by_id(&conn, id)
            .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

        // 事务内二次检查（防止竞态窗口内类型被改）
        if use_tree_move(&current.block_type) {
            return Err(AppError::BadRequest(
                "该类型请使用对应的子树移动接口".to_string(),
            ));
        }

        // ── Flat-list 移动（同文档 + before_id/after_id） ──
        if use_flat_list_move(&current.block_type)
            && (req.before_id.is_some() || req.after_id.is_some())
        {
            let sid = req.before_id.as_deref().or(req.after_id.as_deref()).unwrap();
            let sibling = repo::find_by_id(&conn, sid)
                .map_err(|_| AppError::NotFound(format!(
                    "定位块 {} 不存在或已删除", sid
                )))?;
            if sibling.document_id == current.document_id {
                let after = dispatch_flat_list_move(
                    &conn, &current, req.before_id.as_deref(), req.after_id.as_deref(),
                )?;
                let op = oplog::new_operation(Action::Move, &current.document_id, req.editor_id.clone());
                let change = oplog::block_change_pair(
                    &op.id, id, ChangeType::Moved, &current, &after,
                );
                oplog::record_operation(&conn, &op, &[change])?;
                return Ok(after);
            }
            // 跨文档：走下方通用逻辑
        }

        // ── 通用移动逻辑 ──────────────────────────────────────────

        // 2. 确定目标父块
        let target_parent_id = match (req.before_id.as_deref(), req.after_id.as_deref()) {
            (Some(_), _) | (_, Some(_)) => {
                let sid = req.before_id.as_deref()
                    .or(req.after_id.as_deref())
                    .unwrap();
                let sibling = repo::find_by_id(&conn, sid)
                    .map_err(|_| AppError::NotFound(format!(
                        "定位块 {} 不存在或已删除", sid
                    )))?;
                sibling.parent_id.clone()
            }
            _ => req.target_parent_id.as_deref()
                .ok_or_else(|| AppError::BadRequest(
                    "必须指定 before_id、after_id 或 target_parent_id".to_string(),
                ))?
                .to_string(),
        };

        // 3. 自引用 / 后代引用 → 平铺模型下的合法位置调整（先平铺再建树）
        if target_parent_id == id || repo::check_is_descendant(&conn, id, &target_parent_id)
            .map_err(|e| AppError::Internal(format!("检查后代关系失败: {}", e)))?
        {
            return Ok(current);
        }

        // 4. 父块存在性检测
        let parent_changed = target_parent_id != current.parent_id;
        if parent_changed && !repo::exists_normal(&conn, &target_parent_id)
            .map_err(|e| AppError::Internal(format!("检查父块存在性失败: {}", e)))?
        {
            return Err(AppError::BadRequest(format!(
                "目标父块 {} 不存在或已删除", target_parent_id
            )));
        }

        let target_parent = repo::find_by_id(&conn, &target_parent_id)
            .map_err(|_| AppError::BadRequest(format!(
                "目标父块 {} 不存在或已删除", target_parent_id
            )))?;
        super::list::validate_parent_child_constraint(&target_parent, &current.block_type)?;
        super::list::validate_list_child_type(&target_parent.block_type, &current.block_type)?;

        // 5. 计算新 position
        let new_position = match (req.before_id.as_deref(), req.after_id.as_deref()) {
            (Some(_), _) | (_, Some(_)) => position::calculate_move_position(
                &conn, &target_parent_id,
                req.before_id.as_deref(), req.after_id.as_deref(),
            )?,
            // target_parent_id 模式 → 父块下首位
            _ => {
                let children = repo::find_children(&conn, &target_parent_id)
                    .map_err(|e| AppError::Internal(format!("查询目标父块子块失败: {}", e)))?;
                match children.first() {
                    Some(first) => position::generate_before(&first.position),
                    None => position::generate_first(),
                }
            }
        };

        // 6. UPDATE block
        let now = now_iso();
        let rows = repo::update_parent_position(
            &conn, id, &target_parent_id, &new_position, &now, Some(current.version),
        )
        .map_err(|e| AppError::Internal(format!("移动 Block 失败: {}", e)))?;

        if rows == 0 {
            return Err(AppError::VersionConflict(format!(
                "Block {} 版本冲突（期望版本 {}）", id, current.version
            )));
        }

        // 7. 类型特化后置处理
        on_moved(&conn, &MoveContext {
            block: &current,
            target_parent_id: &target_parent_id,
            new_position: &new_position,
            parent_changed,
        })?;

        if parent_changed && matches!(current.block_type, BlockType::ListItem) {
            let _ = super::list::cleanup_empty_list(&conn, &current.parent_id)?;
        }

        // 8. 记录历史
        let after = repo::find_by_id_raw(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询移动后的 Block 失败: {}", e)))?;

        let op = oplog::new_operation(Action::Move, &current.document_id, req.editor_id.clone());
        let change = oplog::block_change_pair(
            &op.id, id, ChangeType::Moved, &current, &after,
        );
        oplog::record_operation(&conn, &op, &[change])?;

        Ok(after)
    })?;

    event::EventBus::global().emit(BlockEvent::BlockMoved {
        document_id: result.document_id.clone(),
        editor_id,
        block: result.clone(),
    });

    Ok(result)
}

// ─── 导出 Block 树 ──────────────────────────────────────────────

pub fn export_block(
    db: &Db,
    block_id: &str,
    format: &str,
    depth: ExportDepth,
) -> Result<crate::api::response::ExportResult, AppError> {
    let conn = crate::repo::lock_db(db);

    let root = repo::find_by_id(&conn, block_id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", block_id)))?;

    let blocks = match depth {
        ExportDepth::Children => repo::find_children(&conn, block_id)
            .map_err(|e| AppError::Internal(format!("查询子块失败: {}", e)))?,
        ExportDepth::Descendants => repo::find_descendants(&conn, block_id)
            .map_err(|e| AppError::Internal(format!("查询后代失败: {}", e)))?,
    };

    let mut children_map: HashMap<String, Vec<Block>> = HashMap::new();
    for block in &blocks {
        children_map
            .entry(block.parent_id.clone())
            .or_default()
            .push(block.clone());
    }
    for children in children_map.values_mut() {
        children.sort_by(|a, b| a.position.cmp(&b.position));
    }

    let serializer = crate::block_system::parser::get_serializer(format)?;
    let result = serializer.serialize(&root, &children_map)?;

    Ok(crate::api::response::ExportResult {
        content: result.content,
        filename: result.filename,
        blocks_exported: result.blocks_exported,
        lossy_types: result.lossy_types,
    })
}

// ─── Re-export 类型特化操作 ──────────────────────────────────────

pub use super::heading::move_heading_tree;

// ─── 通用子树移动骨架 ───────────────────────────────────────────

pub(crate) fn move_tree<H: TreeMoveOps>(
    db: &Db,
    id: &str,
    editor_id: Option<String>,
    target_parent_id: Option<String>,
    before_id: Option<String>,
    after_id: Option<String>,
) -> Result<Block, AppError> {
    if id == crate::block_system::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可移动".to_string()));
    }

    let editor_id_for_event = editor_id.clone();
    let conn = crate::repo::lock_db(db);

    let result = run_in_transaction(&conn, || {
        let current = repo::find_by_id(&conn, id)
            .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

        H::validate_type(&current)?;

        let resolved_target = H::resolve_target_parent(
            &conn,
            &current.parent_id,
            target_parent_id.as_deref(),
            &before_id,
            &after_id,
        )?;

        if let Some(early) = H::pre_move(&conn, &current, &resolved_target)? {
            return Ok(early);
        }

        validate_no_cycle(&conn, id, &resolved_target, &current.parent_id)?;

        let new_position = position::calculate_move_position(
            &conn, &resolved_target, before_id.as_deref(), after_id.as_deref(),
        )?;

        let rows = H::execute_move(&conn, id, &resolved_target, &new_position, &current)?;
        if rows == 0 {
            return Err(AppError::VersionConflict(format!(
                "Block {} 版本冲突（期望版本 {}）", id, current.version
            )));
        }

        H::post_move(&conn, &current, &resolved_target, &new_position)?;

        let after = repo::find_by_id_raw(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询移动后的 Block 失败: {}", e)))?;

        let op = oplog::new_operation(Action::Move, &current.document_id, editor_id);
        let changes = H::build_changes(&conn, &op, id, &current, &after)?;
        oplog::record_operation(&conn, &op, &changes)?;

        Ok(after)
    })?;

    event::EventBus::global().emit(BlockEvent::BlockMoved {
        document_id: result.document_id.clone(),
        editor_id: editor_id_for_event,
        block: result.clone(),
    });

    Ok(result)
}

// ─── Re-export ──────────────────────────────────────────────────

pub use super::batch::batch_operations;
pub use super::paragraph::{split_block, merge_block};

// ─── 单元测试 ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::tests::init_test_db;
    use crate::block_system::model::BlockType;
    use crate::api::request::PropertiesMode;
    use super::super::document;

    // ── get_root ─────────────────────────────────────────

    #[test]
    fn get_root_returns_root_block() {
        let db = init_test_db();
        let root = get_block(&db, crate::block_system::model::ROOT_ID, false).unwrap();
        assert_eq!(root.id, crate::block_system::model::ROOT_ID);
        assert_eq!(root.parent_id, crate::block_system::model::ROOT_ID);
    }

    // ── create_block ─────────────────────────────────────

    #[test]
    fn create_block_under_root() {
        let db = init_test_db();

        let req = CreateBlockReq {
            parent_id: crate::block_system::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content: "Hello world".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        };

        let block = create_block(&db, req).unwrap();
        assert_eq!(block.parent_id, crate::block_system::model::ROOT_ID);
        assert_eq!(block.block_type, BlockType::Paragraph);
        assert_eq!(block.content, b"Hello world");
        assert_eq!(block.version, 1);
        assert_eq!(block.status, crate::block_system::model::BlockStatus::Normal);
    }

    #[test]
    fn create_block_with_after_id() {
        let db = init_test_db();

        // 先创建一个块
        let req1 = CreateBlockReq {
            parent_id: crate::block_system::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content: "first".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        };
        let block1 = create_block(&db, req1).unwrap();

        // 在 block1 之后插入
        let req2 = CreateBlockReq {
            parent_id: crate::block_system::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content: "second".to_string(),
            properties: HashMap::new(),
            after_id: Some(block1.id.clone()),
            editor_id: None,
        };
        let block2 = create_block(&db, req2).unwrap();

        assert!(block2.position > block1.position);
    }

    #[test]
    fn create_block_nonexistent_parent_fails() {
        let db = init_test_db();

        let req = CreateBlockReq {
            parent_id: "nonexistent0000000".to_string(),
            block_type: BlockType::Paragraph,
            content: "test".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        };

        let result = create_block(&db, req);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("不存在")),
            other => panic!("预期 BadRequest，实际: {:?}", other),
        }
    }

    // ── get_block ────────────────────────────────────────

    #[test]
    fn get_block_returns_existing() {
        let db = init_test_db();

        let req = CreateBlockReq {
            parent_id: crate::block_system::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content: "fetch me".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        };
        let created = create_block(&db, req).unwrap();

        let fetched = get_block(&db, &created.id, false).unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.content, b"fetch me");
    }

    #[test]
    fn get_block_nonexistent_fails() {
        let db = init_test_db();

        let result = get_block(&db, "nonexistent0000000", false);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::NotFound(_) => {}
            other => panic!("预期 NotFound，实际: {:?}", other),
        }
    }

    // ── create_document ──────────────────────────────────

    #[test]
    fn create_root_document() {
        let db = init_test_db();

        let doc = document::create_document(
            &db,
            "My First Doc".to_string(),
            None,   // 根文档
            None,   // 无 after_id
            None,
        ).unwrap();

        assert_eq!(doc.block_type, BlockType::Document);
        assert_eq!(doc.parent_id, crate::block_system::model::ROOT_ID);
        assert_eq!(doc.content, b"My First Doc");
        assert_eq!(doc.properties.get("title").unwrap(), "My First Doc");

        // 验证同时创建了空段落子块
        let content = document::get_document_content(&db, &doc.id).unwrap();
        assert_eq!(content.blocks.len(), 1); // 一个空段落
        assert_eq!(content.blocks[0].block.block_type, BlockType::Paragraph);
    }

    #[test]
    fn create_sub_document() {
        let db = init_test_db();

        // 先创建根文档
        let parent = document::create_document(
            &db, "Parent Doc".to_string(), None, None, None,
        ).unwrap();

        // 创建子文档
        let child = document::create_document(
            &db,
            "Child Doc".to_string(),
            Some(parent.id.clone()),
            None,
            None,
        ).unwrap();

        assert_eq!(child.parent_id, parent.id);
        assert_eq!(child.content, b"Child Doc");
    }

    #[test]
    fn create_document_with_position() {
        let db = init_test_db();

        let doc1 = document::create_document(&db, "Doc 1".to_string(), None, None, None).unwrap();
        let doc2 = document::create_document(&db, "Doc 2".to_string(), None, Some(doc1.id.clone()), None).unwrap();

        assert!(doc2.position > doc1.position);
    }

    // ── update_block ─────────────────────────────────────

    #[test]
    fn update_block_content_success() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::block_system::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content: "original".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let updated = update_block(&db, &created.id, UpdateBlockReq {
            id: created.id.clone(),
            block_type: None,
            content: Some("updated".to_string()),
            properties: None,
            properties_mode: PropertiesMode::Merge,
            editor_id: None,
        }).unwrap();

        assert_eq!(updated.content, b"updated");
        assert_eq!(updated.version, 2);
    }

    #[test]
    fn update_block_merge_properties() {
        let db = init_test_db();

        let mut props = HashMap::new();
        props.insert("key1".to_string(), "val1".to_string());
        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::block_system::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content: "test".to_string(),
            properties: props,
            after_id: None,
            editor_id: None,
        }).unwrap();

        let mut new_props = HashMap::new();
        new_props.insert("key2".to_string(), "val2".to_string());
        let updated = update_block(&db, &created.id, UpdateBlockReq {
            id: created.id.clone(),
            block_type: None,
            content: None,
            properties: Some(new_props),
            properties_mode: PropertiesMode::Merge,
            editor_id: None,
        }).unwrap();

        assert_eq!(updated.properties.get("key1").unwrap(), "val1");
        assert_eq!(updated.properties.get("key2").unwrap(), "val2");
    }

    #[test]
    fn update_block_replace_properties() {
        let db = init_test_db();

        let mut props = HashMap::new();
        props.insert("key1".to_string(), "val1".to_string());
        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::block_system::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content: "test".to_string(),
            properties: props,
            after_id: None,
            editor_id: None,
        }).unwrap();

        let mut new_props = HashMap::new();
        new_props.insert("key2".to_string(), "val2".to_string());
        let updated = update_block(&db, &created.id, UpdateBlockReq {
            id: created.id.clone(),
            block_type: None,
            content: None,
            properties: Some(new_props),
            properties_mode: PropertiesMode::Replace,
            editor_id: None,
        }).unwrap();

        assert!(updated.properties.get("key1").is_none()); // 被替换掉
        assert_eq!(updated.properties.get("key2").unwrap(), "val2");
    }

    // ── delete_block ─────────────────────────────────────

    #[test]
    fn delete_block_soft_deletes() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::block_system::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content: "delete me".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let result = delete_block(&db, &created.id, None).unwrap();
        assert_eq!(result.id, created.id);
        assert_eq!(result.cascade_count, 0); // 叶子块无后代

        // get_block 不再能查到
        assert!(get_block(&db, &created.id, false).is_err());

        // 但 get_block_include_deleted 可以
        let deleted = get_block(&db, &created.id, true).unwrap();
        assert_eq!(deleted.status, crate::block_system::model::BlockStatus::Deleted);
    }

    #[test]
    fn delete_block_promotes_children() {
        let db = init_test_db();

        let doc = document::create_document(&db, "Promote Doc".to_string(), None, None, None).unwrap();

        // 创建 heading + 子块
        let heading = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Heading { level: 2 },
            content: "Section".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let child = create_block(&db, CreateBlockReq {
            parent_id: heading.id.clone(),
            block_type: BlockType::Paragraph,
            content: "child content".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        // 删除 heading（单块删除，子块提升）
        let result = delete_block(&db, &heading.id, None).unwrap();
        assert_eq!(result.id, heading.id);
        assert_eq!(result.cascade_count, 0);

        // heading 被删
        assert!(get_block(&db, &heading.id, false).is_err());

        // 子块仍在，现在挂在 doc 下
        let promoted = get_block(&db, &child.id, false).unwrap();
        assert_eq!(promoted.parent_id, doc.id);
    }

    #[test]
    fn delete_tree_cascades_to_children() {
        let db = init_test_db();

        let doc = document::create_document(&db, "Cascade Doc".to_string(), None, None, None).unwrap();

        let result = delete_tree(&db, &doc.id, None).unwrap();
        assert!(result.cascade_count >= 1); // 至少包含默认段落

        // 文档和段落都不可查
        assert!(get_block(&db, &doc.id, false).is_err());
    }

    #[test]
    fn delete_root_block_forbidden() {
        let db = init_test_db();

        let result = delete_block(&db, crate::block_system::model::ROOT_ID, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("不可删除")),
            other => panic!("预期 BadRequest，实际: {:?}", other),
        }
    }

    // ── restore_block ────────────────────────────────────

    #[test]
    fn restore_deleted_block() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::block_system::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content: "restore me".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        delete_block(&db, &created.id, None).unwrap();

        let result = restore_block(&db, &created.id, None).unwrap();
        assert_eq!(result.id, created.id);

        // 恢复后可以正常查询
        let restored = get_block(&db, &created.id, false).unwrap();
        assert_eq!(restored.status, crate::block_system::model::BlockStatus::Normal);
    }

    #[test]
    fn restore_normal_block_fails() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::block_system::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content: "normal".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let result = restore_block(&db, &created.id, None);
        assert!(result.is_err());
    }

    // ── move_block ───────────────────────────────────────

    #[test]
    fn move_block_to_new_parent() {
        let db = init_test_db();

        let doc1 = document::create_document(&db, "Doc 1".to_string(), None, None, None).unwrap();
        let doc2 = document::create_document(&db, "Doc 2".to_string(), None, None, None).unwrap();

        // 将 doc2 移动到 doc1 下
        let moved = move_block(&db, &doc2.id, MoveBlockReq {
            id: doc2.id.clone(),
            target_parent_id: Some(doc1.id.clone()),
            before_id: None,
            after_id: None,
            editor_id: None,
        }).unwrap();

        assert_eq!(moved.parent_id, doc1.id);
        assert_eq!(moved.version, 2);
    }

    #[test]
    fn move_root_block_forbidden() {
        let db = init_test_db();

        let result = move_block(&db, crate::block_system::model::ROOT_ID, MoveBlockReq {
            id: crate::block_system::model::ROOT_ID.to_string(),
            target_parent_id: Some("any".to_string()),
            before_id: None,
            after_id: None,
            editor_id: None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn move_block_cycle_detection() {
        let db = init_test_db();

        let doc = document::create_document(&db, "Doc".to_string(), None, None, None).unwrap();

        // 试图将 doc 移动到自身下
        let result = move_block(&db, &doc.id, MoveBlockReq {
            id: doc.id.clone(),
            target_parent_id: Some(doc.id.clone()),
            before_id: None,
            after_id: None,
            editor_id: None,
        });
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(_) => {} // "不能将 Block 移动到自身下"
            AppError::CycleReference => {} // 或循环引用
            other => panic!("预期移动错误，实际: {:?}", other),
        }
    }

    // ── list_root_documents ──────────────────────────────

    #[test]
    fn list_root_documents_after_create() {
        let db = init_test_db();

        document::create_document(&db, "Doc 1".to_string(), None, None, None).unwrap();
        document::create_document(&db, "Doc 2".to_string(), None, None, None).unwrap();

        let docs = document::list_root_documents(&db).unwrap();
        assert!(docs.len() >= 2);
        let titles: Vec<&str> = docs.iter()
            .filter_map(|d| d.properties.get("title").map(|s: &String| s.as_str()))
            .collect();
        assert!(titles.contains(&"Doc 1"));
        assert!(titles.contains(&"Doc 2"));
    }

    // ── get_document_tree ────────────────────────────────

    #[test]
    fn get_document_tree_nested() {
        let db = init_test_db();

        let doc = document::create_document(&db, "Tree Doc".to_string(), None, None, None).unwrap();
        let child = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Heading { level: 2 },
            content: "Section".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let result = document::get_document_content(&db, &doc.id).unwrap();
        assert_eq!(result.document.id, doc.id);
        assert_eq!(result.blocks.len(), 2); // 默认段落 + heading
        assert!(result.blocks.iter().any(|n| n.block.id == child.id));
    }

    #[test]
    fn get_document_tree_nonexistent_fails() {
        let db = init_test_db();

        let result = document::get_document_content(&db, "nonexistent0000000");
        assert!(result.is_err());
    }

    // ── get_children ─────────────────────────────────────

    #[test]
    fn get_children_with_pagination() {
        let db = init_test_db();

        let doc = document::create_document(&db, "Pagination Doc".to_string(), None, None, None).unwrap();

        // 创建 3 个额外子块（已有 1 个默认段落）
        for i in 0..3 {
            create_block(&db, CreateBlockReq {
                parent_id: doc.id.clone(),
                block_type: BlockType::Paragraph,
                content: format!("para {}", i),
                properties: HashMap::new(),
                after_id: None,
                editor_id: None,
            }).unwrap();
        }

        // 通过 repo 层测试分页：限制每页 2 条（总共 4 个子块 = 1 默认段落 + 3 新增）
        let db_conn = crate::repo::lock_db(&db);
        let page1 = crate::repo::block_repo::find_children_paginated(&db_conn, &doc.id, None, 2).unwrap();
        drop(db_conn);
        assert_eq!(page1.len(), 2);

        // 翻页：取剩下的 2 条
        let db_conn = crate::repo::lock_db(&db);
        let page2 = crate::repo::block_repo::find_children_paginated(&db_conn, &doc.id, Some(&page1[1].position), 2).unwrap();
        drop(db_conn);
        assert_eq!(page2.len(), 2);
    }

    #[test]
    fn get_children_nonexistent_parent_fails() {
        let db = init_test_db();

        // get_document_children 对不存在的文档应返回错误
        let result = document::get_document_children(&db, "nonexistent0000000");
        assert!(result.is_err());
    }

    // ── import_text ─────────────────────────────────────

    fn import_md(db: &Db, content: &str) -> Result<crate::api::response::ImportResult, AppError> {
        document::import_text(db, crate::api::request::ImportTextReq {
            editor_id: None,
            format: "markdown".to_string(),
            content: content.to_string(),
            parent_id: None,
            after_id: None,
            title: None,
        })
    }

    #[test]
    fn import_simple_paragraph() {
        let db = init_test_db();
        let result = import_md(&db, "Hello world").unwrap();
        assert_eq!(result.root.block_type, BlockType::Document);
        assert_eq!(result.root.parent_id, crate::block_system::model::ROOT_ID);
        assert!(result.blocks_imported >= 2);
    }

    #[test]
    fn import_heading_and_paragraph() {
        let db = init_test_db();
        let result = import_md(&db, "# My Title\n\nSome content here").unwrap();
        assert_eq!(result.root.properties.get("title").unwrap(), "My Title");
        assert!(result.blocks_imported >= 3);
    }

    #[test]
    fn import_empty_content() {
        let db = init_test_db();
        let result = import_md(&db, "").unwrap();
        assert_eq!(result.root.block_type, BlockType::Document);
        assert!(result.blocks_imported >= 2);
    }

    #[test]
    fn import_with_title_override() {
        let db = init_test_db();
        let req = crate::api::request::ImportTextReq {
            editor_id: None,
            format: "markdown".to_string(),
            content: "# Original\n\nContent".to_string(),
            parent_id: None,
            after_id: None,
            title: Some("Overridden Title".to_string()),
        };
        let result = document::import_text(&db, req).unwrap();
        assert_eq!(result.root.properties.get("title").unwrap(), "Overridden Title");
    }

    #[test]
    fn import_to_specific_parent() {
        let db = init_test_db();
        let parent = document::create_document(&db, "Parent Doc".to_string(), Some(crate::block_system::model::ROOT_ID.to_string()), None, None).unwrap();
        let req = crate::api::request::ImportTextReq {
            editor_id: None,
            format: "markdown".to_string(),
            content: "# Child\n\nChild content".to_string(),
            parent_id: Some(parent.id.clone()),
            after_id: None,
            title: None,
        };
        let result = document::import_text(&db, req).unwrap();
        assert_eq!(result.root.parent_id, parent.id);
    }

    #[test]
    fn import_invalid_format() {
        let db = init_test_db();
        let req = crate::api::request::ImportTextReq {
            editor_id: None,
            format: "pdf".to_string(),
            content: "some text".to_string(),
            parent_id: None,
            after_id: None,
            title: None,
        };
        assert!(document::import_text(&db, req).is_err());
    }

    #[test]
    fn import_nonexistent_parent() {
        let db = init_test_db();
        let req = crate::api::request::ImportTextReq {
            editor_id: None,
            format: "markdown".to_string(),
            content: "Hello".to_string(),
            parent_id: Some("nonexistent_id_12345".to_string()),
            after_id: None,
            title: None,
        };
        assert!(document::import_text(&db, req).is_err());
    }

    #[test]
    fn import_code_block() {
        let db = init_test_db();
        let result = import_md(&db, "```rust\nfn main() {}\n```").unwrap();
        assert!(result.blocks_imported >= 2);
    }

    #[test]
    fn import_list() {
        let db = init_test_db();
        let result = import_md(&db, "- item 1\n- item 2\n- item 3").unwrap();
        assert!(result.blocks_imported >= 7);
    }

    #[test]
    fn import_blockquote() {
        let db = init_test_db();
        let result = import_md(&db, "> This is a quote\n> Second line").unwrap();
        assert!(result.blocks_imported >= 3);
    }

    #[test]
    fn import_multiple_documents() {
        let db = init_test_db();
        let r1 = import_md(&db, "# Doc 1").unwrap();
        let r2 = import_md(&db, "# Doc 2").unwrap();
        let r3 = import_md(&db, "# Doc 3").unwrap();
        assert_ne!(r1.root.id, r2.root.id);
        assert_ne!(r2.root.id, r3.root.id);
        assert!(r2.root.position > r1.root.position);
        assert!(r3.root.position > r2.root.position);
    }

    #[test]
    fn import_md_alias() {
        let db = init_test_db();
        let req = crate::api::request::ImportTextReq {
            editor_id: None,
            format: "md".to_string(),
            content: "# Alias Test".to_string(),
            parent_id: None,
            after_id: None,
            title: None,
        };
        assert!(document::import_text(&db, req).is_ok());
    }

    // ── export_block / export_text ──────────────────────

    #[test]
    fn export_simple_document() {
        let db = init_test_db();
        let doc_id = import_md(&db, "# Hello\n\nWorld").unwrap().root.id;
        let result = document::export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.content.contains("Hello"));
        assert!(result.content.contains("World"));
        assert!(result.blocks_exported >= 3);
    }

    #[test]
    fn export_empty_document() {
        let db = init_test_db();
        let doc_id = import_md(&db, "").unwrap().root.id;
        let result = document::export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.blocks_exported >= 1);
    }

    #[test]
    fn export_code_block() {
        let db = init_test_db();
        let doc_id = import_md(&db, "```rust\nfn main() {}\n```").unwrap().root.id;
        let result = document::export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.content.contains("```rust"));
    }

    #[test]
    fn export_list() {
        let db = init_test_db();
        let doc_id = import_md(&db, "- item 1\n- item 2\n- item 3").unwrap().root.id;
        let result = document::export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.content.contains("item 1"));
    }

    #[test]
    fn export_blockquote() {
        let db = init_test_db();
        let doc_id = import_md(&db, "> quoted text").unwrap().root.id;
        let result = document::export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.content.contains("quoted text"));
    }

    #[test]
    fn export_filename() {
        let db = init_test_db();
        let doc_id = import_md(&db, "# My Notes\n\nContent").unwrap().root.id;
        let result = document::export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.filename.is_some());
        let filename = result.filename.unwrap();
        assert!(filename.contains("My Notes"));
        assert!(filename.ends_with(".md"));
    }

    #[test]
    fn export_md_alias() {
        let db = init_test_db();
        let doc_id = import_md(&db, "# Alias\n\nTest").unwrap().root.id;
        let result = document::export_text(&db, &doc_id, "md").unwrap();
        assert!(result.content.contains("Alias"));
    }

    #[test]
    fn export_nonexistent_document() {
        let db = init_test_db();
        assert!(document::export_text(&db, "nonexistent_id_12345", "markdown").is_err());
    }

    #[test]
    fn export_invalid_format() {
        let db = init_test_db();
        let doc_id = import_md(&db, "# Test").unwrap().root.id;
        assert!(document::export_text(&db, &doc_id, "pdf").is_err());
    }

    #[test]
    fn export_created_document() {
        let db = init_test_db();
        let doc = document::create_document(&db, "Created Doc".to_string(), Some(crate::block_system::model::ROOT_ID.to_string()), None, None).unwrap();
        let result = document::export_text(&db, &doc.id, "markdown").unwrap();
        assert!(result.content.contains("Created Doc"));
        assert_eq!(result.blocks_exported, 2);
    }

    #[test]
    fn export_block_children_depth() {
        let db = init_test_db();
        let doc_id = import_md(&db, "# Title\n\n## Section\n\nText\n\n```js\nconsole.log(1)\n```\n\n- a\n- b\n").unwrap().root.id;

        // Export with Descendants
        let full = export_block(&db, &doc_id, "markdown", ExportDepth::Descendants).unwrap();
        assert!(full.content.contains("Title"));
        assert!(full.content.contains("Section"));
        assert!(full.content.contains("console.log(1)"));
        assert!(full.blocks_exported >= 8);
    }

    #[test]
    fn export_heading_as_block() {
        let db = init_test_db();
        let doc = document::create_document(&db, "Doc".to_string(), None, None, None).unwrap();

        let mut props = HashMap::new();
        props.insert("title".to_string(), "My Section".to_string());
        let heading = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Heading { level: 2 },
            content: "My Section".to_string(),
            properties: props,
            after_id: None,
            editor_id: None,
        }).unwrap();

        // Export heading as a block root (Children depth)
        let result = export_block(&db, &heading.id, "markdown", ExportDepth::Children).unwrap();
        assert!(result.content.contains("My Section"));
        assert_eq!(result.blocks_exported, 1); // heading itself
    }

    #[test]
    fn move_heading_within_document() {
        let db = init_test_db();

        let doc = document::create_document(&db, "Test Doc".to_string(), None, None, None).unwrap();

        let p1 = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Paragraph,
            content: "p1".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let h2 = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Heading { level: 2 },
            content: "Section".to_string(),
            properties: HashMap::new(),
            after_id: Some(p1.id.clone()),
            editor_id: None,
        }).unwrap();

        let p2 = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Paragraph,
            content: "p2".to_string(),
            properties: HashMap::new(),
            after_id: Some(h2.id.clone()),
            editor_id: None,
        }).unwrap();

        // Move h2 before p1
        let moved = move_block(&db, &h2.id, MoveBlockReq {
            id: h2.id.clone(),
            target_parent_id: None,
            before_id: Some(p1.id.clone()),
            after_id: None,
            editor_id: Some("test".to_string()),
        }).unwrap();

        eprintln!("moved h2: pos={}", moved.position);
        eprintln!("p1 pos={}, p2 pos={}", p1.position, p2.position);

        let conn = crate::repo::lock_db(&db);
        let doc_children = crate::repo::block_repo::find_children(&conn, &doc.id).unwrap();
        let h2_children = crate::repo::block_repo::find_children(&conn, &h2.id).unwrap();
        eprintln!("doc children: {:?}", doc_children.iter().map(|c| (&c.id, &c.position)).collect::<Vec<_>>());
        eprintln!("h2 children: {:?}", h2_children.iter().map(|c| (&c.id, &c.position)).collect::<Vec<_>>());
        drop(conn);

        assert_eq!(moved.id, h2.id);
        assert!(h2_children.len() >= 1, "h2 should absorb siblings after move");
    }

    #[test]
    fn move_heading_after_own_child_promotes_and_moves() {
        let db = init_test_db();
        let doc = document::create_document(&db, "Doc".to_string(), None, None, None).unwrap();

        // doc → h2 → [p1, p2, p3]
        let h2 = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Heading { level: 2 },
            content: "Section".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let p1 = create_block(&db, CreateBlockReq {
            parent_id: h2.id.clone(),
            block_type: BlockType::Paragraph,
            content: "p1".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let p2 = create_block(&db, CreateBlockReq {
            parent_id: h2.id.clone(),
            block_type: BlockType::Paragraph,
            content: "p2".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let p3 = create_block(&db, CreateBlockReq {
            parent_id: h2.id.clone(),
            block_type: BlockType::Paragraph,
            content: "p3".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        // 拖 h2 到 p2 之后 → 提升子块到 doc，h2 移到 p2 后面
        let moved = move_block(&db, &h2.id, MoveBlockReq {
            id: h2.id.clone(),
            target_parent_id: None,
            before_id: None,
            after_id: Some(p2.id.clone()),
            editor_id: Some("test".to_string()),
        }).unwrap();

        assert_eq!(moved.parent_id, doc.id);

        // doc 应该有: p1, p2, h2(with p3)
        let conn = crate::repo::lock_db(&db);
        let doc_children = crate::repo::block_repo::find_children(&conn, &doc.id).unwrap();
        let h2_children = crate::repo::block_repo::find_children(&conn, &h2.id).unwrap();

        // p1 和 p2 被提升到 doc 级别
        assert!(doc_children.iter().any(|c| c.id == p1.id), "p1 should be at doc level");
        assert!(doc_children.iter().any(|c| c.id == p2.id), "p2 should be at doc level");
        // h2 仍在 doc 级别
        assert!(doc_children.iter().any(|c| c.id == h2.id), "h2 should be at doc level");
        // p3 被 h2 重新吸收
        assert!(h2_children.iter().any(|c| c.id == p3.id), "p3 should be re-absorbed by h2");

        // 顺序: p1 < p2 < h2 (at doc level)
        let pos_p1 = doc_children.iter().find(|c| c.id == p1.id).unwrap().position.as_str();
        let pos_p2 = doc_children.iter().find(|c| c.id == p2.id).unwrap().position.as_str();
        let pos_h2 = doc_children.iter().find(|c| c.id == h2.id).unwrap().position.as_str();
        assert!(pos_p1 < pos_p2, "p1 should be before p2");
        assert!(pos_p2 < pos_h2, "p2 should be before h2");
    }

    #[test]
    fn move_heading_same_doc_children_follow() {
        let db = init_test_db();
        let doc = document::create_document(&db, "Doc".to_string(), None, None, None).unwrap();

        // doc → p1, h2(with p2), p3
        let p1 = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Paragraph,
            content: "p1".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let h2 = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Heading { level: 2 },
            content: "Section".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        // p2 is placed after h2, so heading will absorb it
        let _p2 = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Paragraph,
            content: "p2".to_string(),
            properties: HashMap::new(),
            after_id: Some(h2.id.clone()),
            editor_id: None,
        }).unwrap();

        // Move h2 before p1 (same document)
        let moved = move_block(&db, &h2.id, MoveBlockReq {
            id: h2.id.clone(),
            target_parent_id: None,
            before_id: Some(p1.id.clone()),
            after_id: None,
            editor_id: Some("test".to_string()),
        }).unwrap();

        // h2 should have absorbed p2 (and p1) after move
        let conn = crate::repo::lock_db(&db);
        let h2_children = crate::repo::block_repo::find_children(&conn, &h2.id).unwrap();
        drop(conn);

        // doc should only have h2 now, h2 should have the others
        assert!(h2_children.len() >= 1, "h2 should have children after same-doc move");
        assert_eq!(moved.parent_id, doc.id);
    }
}