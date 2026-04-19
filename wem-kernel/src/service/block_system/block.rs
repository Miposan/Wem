//! 通用块操作 + BlockTypeOps trait + 类型分派
//!
//! 定义 BlockTypeOps trait 作为类型特化行为的统一接口，
//! heading.rs / document.rs 提供各自的实现。
//! 分派层通过 trait 调用，永不硬编码 `if heading { ... }`。

use std::collections::HashMap;

use crate::api::request::{BatchOp, BatchReq, CreateBlockReq, MoveBlockReq, MoveDocumentTreeReq, PropertiesMode, UpdateBlockReq};
use crate::api::response::{
    BatchOpResult, BatchResult, DeleteResult, RestoreResult,
};
use crate::repo::block_repo as repo;
use crate::repo::block_repo::InsertBlockParams;
use crate::repo::Db;
use crate::error::AppError;
use crate::model::event::BlockEvent;
use crate::model::oplog::{Action, BlockSnapshot, Change, ChangeType, Operation};
use crate::model::{generate_block_id, Block, BlockType};
use super::{event, oplog, position};
use crate::util;
use util::now_iso;

/// 将可序列化值转为 JSON 字符串。
///
/// `BlockType`、`HashMap<String, String>` 等类型的序列化不会失败，
/// 用此函数替代 `.unwrap_or_default()` 以避免静默吞错。
pub(crate) fn to_json<T: serde::Serialize>(val: &T) -> String {
    serde_json::to_string(val).unwrap_or_else(|e| {
        tracing::error!("序列化失败（不应发生）: {}", e);
        "{}".to_string()
    })
}

// ─── 操作上下文 ─────────────────────────────────────────────────

/// 移动操作的上下文，传递给类型钩子
pub struct MoveContext<'a> {
    pub block: &'a Block,
    pub target_parent_id: &'a str,
    pub new_position: &'a str,
    pub parent_changed: bool,
}

// ─── BlockTypeOps trait ──────────────────────────────────────────

/// BlockType 行为钩子
///
/// 定义类型特化的操作接口：各 BlockType 变体可重写特定钩子，
/// 通用层通过分派函数调用，默认实现为空操作。
pub trait BlockTypeOps {
    fn use_tree_move() -> bool { false }

    fn validate_on_create(block_type: &BlockType) -> Result<(), AppError> {
        let _ = block_type;
        Ok(())
    }

    fn on_moved(
        conn: &rusqlite::Connection,
        ctx: &MoveContext<'_>,
    ) -> Result<(), AppError> {
        let _ = (conn, ctx);
        Ok(())
    }

    fn adjust_content_on_update(
        conn: &rusqlite::Connection,
        block: &Block,
        content: &mut Vec<u8>,
    ) -> Result<(), AppError> {
        let _ = (conn, block, content);
        Ok(())
    }

    fn on_type_changed(
        conn: &rusqlite::Connection,
        block_id: &str,
        old_block: &Block,
        new_type: &BlockType,
    ) -> Result<(), AppError> {
        let _ = (conn, block_id, old_block, new_type);
        Ok(())
    }

    fn on_self_or_descendant_move(
        conn: &rusqlite::Connection,
        block: &Block,
        is_self: bool,
        is_descendant: bool,
    ) -> Result<Option<Block>, AppError> {
        let _ = (conn, block, is_descendant);
        if is_self {
            Err(AppError::BadRequest("不能将 Block 移动到自身下".to_string()))
        } else {
            Ok(None)
        }
    }
}

// ─── 类型分派 ───────────────────────────────────────────────────

use super::heading::HeadingOps;
use super::document::DocumentOps;
use super::paragraph::ParagraphOps;

pub(crate) fn use_tree_move(block_type: &BlockType) -> bool {
    match block_type {
        BlockType::Document => DocumentOps::use_tree_move(),
        _ => ParagraphOps::use_tree_move(),
    }
}

/// 返回需要子树移动的类型名称（用于错误提示）
pub(crate) fn tree_move_type_name(block_type: &BlockType) -> Option<&'static str> {
    match block_type {
        BlockType::Document if use_tree_move(block_type) => Some("Document"),
        _ => None,
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

pub(crate) fn on_self_or_descendant_move(
    conn: &rusqlite::Connection,
    block: &Block,
    is_self: bool,
    is_descendant: bool,
) -> Result<Option<Block>, AppError> {
    match &block.block_type {
        BlockType::Heading { .. } => HeadingOps::on_self_or_descendant_move(conn, block, is_self, is_descendant),
        _ => ParagraphOps::on_self_or_descendant_move(conn, block, is_self, is_descendant),
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
        (BlockType::Heading { .. }, _) | (_, BlockType::Heading { .. }) => {
            HeadingOps::on_type_changed(conn, block_id, old_block, new_type)
        }
        _ => Ok(()),
    }
}

// ─── 辅助函数 ──────────────────────────────────────────────────

/// 推断 document_id：给定父块，返回新建子块应使用的 document_id。
///
/// - 如果 parent 是 Document 类型 → document_id = parent.id（文档块自身就是文档根）
/// - 否则继承 parent.document_id（内容块指向所属文档）
pub(crate) fn derive_document_id(parent: &Block) -> String {
    if matches!(parent.block_type, BlockType::Document) {
        parent.id.clone()
    } else {
        parent.document_id.clone()
    }
}

/// 从 parent_id 推断 document_id（不加载完整 Block）。
/// Document 块的 document_id = 自身 id；其他块继承 parent 的 document_id。
pub(crate) fn derive_document_id_from_parent(
    conn: &rusqlite::Connection,
    parent_id: &str,
) -> Result<String, AppError> {
    let parent = repo::find_by_id(conn, parent_id)
        .map_err(|_| AppError::Internal(format!("查询 parent {} 失败", parent_id)))?;
    Ok(derive_document_id(&parent))
}

/// 合并或替换属性
fn merge_properties(
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

// ─── 创建 Block ────────────────────────────────────────────────

/// 创建 Block
///
/// 流程：
/// 1. 验证 parent_id 存在且未删除
/// 2. 根据 after_id 计算 position（插入指定位置或追加末尾）
/// 3. 推断 content_type（如果请求未指定）
/// 4. 生成 20 位 ID + 时间戳
/// 5. INSERT INTO blocks
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

        // 3. 推断 document_id
        let document_id = derive_document_id(&parent);

        // 4. 计算 position
        let position =
            position::calculate_insert_position(&conn, &req.parent_id, req.after_id.as_deref())?;

        // 5. 推断 content_type
        let content_type = req
            .content_type
            .clone()
            .unwrap_or_else(|| req.block_type.default_content_type());

        // 6. 生成 ID 和时间戳
        let id = generate_block_id();
        let now = now_iso();

        // 7. INSERT（通过 repository）
        repo::insert_block(&conn, &InsertBlockParams {
            id: id.clone(),
            parent_id: req.parent_id,
            document_id: document_id.clone(),
            position,
            block_type: to_json(&req.block_type),
            content_type: content_type.as_str().to_string(),
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
    result.map_err(|_| AppError::NotFound(format!("Block {} 不存在", id)))
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
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在", id)))?;

    // 2. 计算新 content
    let mut new_content: Vec<u8> = req
        .content
        .map(|c| c.into_bytes())
        .unwrap_or_else(|| current.content.clone());

    // 2.5 类型特化内容调整
    adjust_content_on_update(&conn, &current, &mut new_content)?;

    // 3. 计算新 block_type 和 content_type
    let new_block_type = req.block_type.clone().unwrap_or(current.block_type.clone());

    // 4. 计算新 properties（merge 或 replace）
    let new_properties = merge_properties(&current.properties, req.properties.as_ref(), &req.properties_mode);
    let properties_json = to_json(&new_properties);

    // 5. 写入数据库
    let new_content_type = req.block_type
        .as_ref()
        .map(|bt| bt.default_content_type())
        .unwrap_or(current.content_type.clone());

    write_block_updates(conn, id, &req.block_type, &new_content, &properties_json, &new_block_type, new_content_type.as_str())?;

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
    new_content_type: &str,
) -> Result<(), AppError> {
    let now = now_iso();
    let block_type_changed = block_type_req.is_some();

    let rows = if block_type_changed {
        let bt_str = to_json(new_block_type);
        repo::update_block_fields(
            conn, id, new_content, properties_json,
            Some(&bt_str), Some(new_content_type), &now,
        )
    } else {
        repo::update_content_and_props(
            conn, id, new_content, properties_json, &now,
        )
    }
    .map_err(|e| AppError::Internal(format!("更新 Block 失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::NotFound(format!("Block {} 不存在", id)));
    }

    Ok(())
}

// ─── 共享子块 reparent 辅助 ──────────────────────────────────

/// 将 anchor 块的所有直系子块 reparent 到 new_parent_id。
///
/// 子块按原顺序排列在 anchor 之后。如果 new_parent 后有兄弟块，
/// 子块插入到 anchor 和第一个后续兄弟之间；否则追加到末尾。
///
/// `update_document_id`：是否同时更新子块的 document_id（跨文档场景需要）。
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

    let mut pos = match siblings_after.first() {
        Some(first_after) => position::generate_between(anchor_position, &first_after.position)?,
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
        pos = position::generate_after(&pos);
    }

    Ok(())
}

// ─── 删除 Block ──────────────────────────────────────────────────

/// 删除单个 Block（子块提升到父级）
///
/// 只软删除目标块本身，其子块 reparent 到被删块的父级，保持原有顺序。
/// 用于编辑器 Backspace 删除空块等场景。
pub fn delete_block(db: &Db, id: &str, editor_id: Option<String>) -> Result<DeleteResult, AppError> {
    if id == crate::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可删除".to_string()));
    }

    let editor_id_for_event = editor_id.clone();
    let conn = crate::repo::lock_db(db);

    let result = run_in_transaction(&conn, || -> Result<DeleteResult, AppError> {
        let current = repo::find_by_id(&conn, id)
            .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

        let document_id = current.document_id.clone();

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
    if id == crate::model::ROOT_ID {
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
    if id == crate::model::ROOT_ID {
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
        if let Some(type_name) = tree_move_type_name(&current.block_type) {
            return Err(AppError::BadRequest(
                format!("{} 类型请使用 move-{}-tree 接口", type_name, type_name.to_lowercase()),
            ));
        }

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

        // 3. 拖入自身子树（类型特化处理）
        let is_self = target_parent_id == id;
        let is_descendant = repo::check_is_descendant(&conn, id, &target_parent_id).unwrap_or(false);
        if let Some(block) = on_self_or_descendant_move(&conn, &current, is_self, is_descendant)? {
            return Ok(block);
        }

        // 4. 循环引用检测（仅当父块改变时）
        let parent_changed = target_parent_id != current.parent_id;
        if parent_changed {
            if repo::check_is_descendant(&conn, id, &target_parent_id).unwrap_or(false) {
                return Err(AppError::CycleReference);
            }
            if !repo::exists_normal(&conn, &target_parent_id).unwrap_or(false) {
                return Err(AppError::BadRequest(format!(
                    "目标父块 {} 不存在或已删除", target_parent_id
                )));
            }
        }

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
            &conn, id, &target_parent_id, &new_position, &now,
        )
        .map_err(|e| AppError::Internal(format!("移动 Block 失败: {}", e)))?;

        if rows == 0 {
            return Err(AppError::NotFound(format!("Block {} 不存在", id)));
        }

        // 7. 类型特化后置处理
        on_moved(&conn, &MoveContext {
            block: &current,
            target_parent_id: &target_parent_id,
            new_position: &new_position,
            parent_changed,
        })?;

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

/// 导出深度控制
#[derive(Debug, Clone, PartialEq)]
pub enum ExportDepth {
    /// 仅直接子块
    Children,
    /// 所有后代（递归）
    Descendants,
}

/// 通用导出：将任意 Block 及其子树序列化为文本。
///
/// `depth` 控制子树范围：`Children` = 仅直接子块，`Descendants` = 所有后代。
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

    let serializer = crate::parser::get_serializer(format)?;
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

// ─── 子树移动辅助函数 ─────────────────────────────────────────

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

/// 事务提交或回滚（泛型版本）
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

// ─── 通用子树移动 ──────────────────────────────────────────────

/// 子树移动的类型特化钩子
///
/// 各类型各自实现此 trait，提供特有的移动逻辑。
/// 通用骨架 `move_tree` 负责公共流程。
pub(crate) trait TreeMoveOps {
    fn validate_type(current: &Block) -> Result<(), AppError>;

    fn resolve_target_parent(
        conn: &rusqlite::Connection,
        current_parent_id: &str,
        target_parent_id: Option<&str>,
        before_id: &Option<String>,
        after_id: &Option<String>,
    ) -> Result<String, AppError>;

    /// 返回 Ok(Some(block)) 可短路移动（不执行实际移动）
    fn pre_move(
        conn: &rusqlite::Connection,
        current: &Block,
        target_parent_id: &str,
    ) -> Result<Option<Block>, AppError>;

    fn execute_move(
        conn: &rusqlite::Connection,
        id: &str,
        target_parent_id: &str,
        new_position: &str,
        current: &Block,
    ) -> Result<u64, AppError>;

    fn post_move(
        conn: &rusqlite::Connection,
        current: &Block,
        target_parent_id: &str,
        new_position: &str,
    ) -> Result<(), AppError>;

    fn build_changes(
        conn: &rusqlite::Connection,
        op: &Operation,
        id: &str,
        current: &Block,
        after: &Block,
    ) -> Result<Vec<Change>, AppError>;
}

/// 通用子树移动骨架
pub(crate) fn move_tree<H: TreeMoveOps>(
    db: &Db,
    id: &str,
    editor_id: Option<String>,
    target_parent_id: Option<String>,
    before_id: Option<String>,
    after_id: Option<String>,
) -> Result<Block, AppError> {
    if id == crate::model::ROOT_ID {
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
            return Err(AppError::NotFound(format!("Block {} 不存在", id)));
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

// ─── 批量操作 ──────────────────────────────────────────────────

/// 批量执行多个 Block 操作
///
/// 单次最多 50 条操作，按数组顺序在同一事务内执行。
/// `create` 操作可指定 `temp_id`，后续操作可用 `temp_id` 引用该块。
/// 任何操作失败不影响其他操作，每条操作独立返回结果。
///
/// 参考 03-api-rest.md §3 "批量操作"
pub fn batch_operations(db: &Db, req: BatchReq) -> Result<BatchResult, AppError> {
    // 限制操作数量
    if req.operations.len() > 50 {
        return Err(AppError::BadRequest(
            "单次批量操作上限 50 条".to_string(),
        ));
    }

    let editor_id_for_event = req.editor_id.clone();
    let conn = crate::repo::lock_db(db);

    let (result, doc_id) = run_in_transaction(&conn, || {

    let mut id_map: HashMap<String, String> = HashMap::new();
    let mut results: Vec<BatchOpResult> = Vec::with_capacity(req.operations.len());
    let mut pending_changes: Vec<crate::model::oplog::Change> = Vec::new();

    // 预创建 Operation（doc_id 后续根据实际变更确定）
    let operation = oplog::new_operation(Action::BatchOps, "", req.editor_id.clone());
    let op_id = operation.id.clone();

    /// 解析 block_id：如果是 temp_id 映射中存在的，替换为真实 ID
    fn resolve_id(id: &str, id_map: &HashMap<String, String>) -> String {
        id_map.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    for batch_op in req.operations {
        match batch_op {
            BatchOp::Create {
                temp_id,
                parent_id,
                block_type,
                content_type,
                content,
                properties,
                after_id,
            } => {
                let resolved_parent = resolve_id(&parent_id, &id_map);
                let resolved_after = after_id.map(|aid| resolve_id(&aid, &id_map));

                let result = batch_create_block(
                    &conn,
                    &resolved_parent,
                    block_type,
                    content_type.as_ref(),
                    &content,
                    &properties,
                    resolved_after.as_deref(),
                );

                match result {
                    Ok(block) => {
                        let real_id = block.id.clone();
                        // 记录 Change：创建操作 before=None, after=快照
                        pending_changes.push(oplog::new_change(
                            &op_id,
                            &real_id,
                            ChangeType::Created,
                            None,
                            Some(BlockSnapshot::from_block(&block)),
                        ));
                        id_map.insert(temp_id.clone(), real_id.clone());
                        results.push(BatchOpResult {
                            action: "create".to_string(),
                            block_id: real_id,
                            version: Some(block.version),
                            error: None,
                        });
                    }
                    Err(e) => {
                        results.push(BatchOpResult {
                            action: "create".to_string(),
                            block_id: temp_id,
                            version: None,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }

            BatchOp::Update {
                block_id,
                content,
                properties,
                properties_mode,
            } => {
                let resolved_id = resolve_id(&block_id, &id_map);

                // 捕获 before 快照
                let before = repo::find_by_id(&conn, &resolved_id).ok();

                let result = batch_update_block(
                    &conn,
                    &resolved_id,
                    content.as_deref(),
                    &properties,
                    &properties_mode,
                );

                match result {
                    Ok(new_version) => {
                        // 捕获 after 快照
                        let after = repo::find_by_id_raw(&conn, &resolved_id).ok();
                        if let (Some(b), Some(a)) = (&before, &after) {
                            pending_changes.push(oplog::block_change_pair(
                                &op_id, &resolved_id, ChangeType::Updated, b, a,
                            ));
                        }
                        results.push(BatchOpResult {
                            action: "update".to_string(),
                            block_id: resolved_id,
                            version: Some(new_version),
                            error: None,
                        });
                    }
                    Err(e) => {
                        results.push(BatchOpResult {
                            action: "update".to_string(),
                            block_id: block_id,
                            version: None,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }

            BatchOp::Delete { block_id } => {
                let resolved_id = resolve_id(&block_id, &id_map);

                // 捕获 before 快照
                let before = repo::find_by_id(&conn, &resolved_id).ok();

                let result = batch_delete_block(&conn, &resolved_id);

                match result {
                    Ok(new_version) => {
                        if let Some(b) = &before {
                            pending_changes.push(oplog::new_change(
                                &op_id, &resolved_id, ChangeType::Deleted,
                                Some(BlockSnapshot::from_block(b)), None,
                            ));
                        }
                        results.push(BatchOpResult {
                            action: "delete".to_string(),
                            block_id: resolved_id,
                            version: Some(new_version),
                            error: None,
                        });
                    }
                    Err(e) => {
                        results.push(BatchOpResult {
                            action: "delete".to_string(),
                            block_id,
                            version: None,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }

            BatchOp::Move {
                block_id,
                target_parent_id,
                before_id,
                after_id,
            } => {
                let resolved_id = resolve_id(&block_id, &id_map);
                let resolved_target =
                    target_parent_id.map(|pid| resolve_id(&pid, &id_map));
                let resolved_before = before_id.map(|bid| resolve_id(&bid, &id_map));
                let resolved_after = after_id.map(|aid| resolve_id(&aid, &id_map));

                // 捕获 before 快照
                let before = repo::find_by_id(&conn, &resolved_id).ok();

                let result = batch_move_block(
                    &conn,
                    &resolved_id,
                    resolved_target.as_deref(),
                    resolved_before.as_deref(),
                    resolved_after.as_deref(),
                );

                match result {
                    Ok(new_version) => {
                        // 捕获 after 快照
                        let after = repo::find_by_id_raw(&conn, &resolved_id).ok();
                        if let (Some(b), Some(a)) = (&before, &after) {
                            pending_changes.push(oplog::block_change_pair(
                                &op_id, &resolved_id, ChangeType::Moved, b, a,
                            ));
                        }
                        results.push(BatchOpResult {
                            action: "move".to_string(),
                            block_id: resolved_id,
                            version: Some(new_version),
                            error: None,
                        });
                    }
                    Err(e) => {
                        results.push(BatchOpResult {
                            action: "move".to_string(),
                            block_id,
                            version: None,
                            error: Some(e.to_string()),
                        });
                    }
                }
            }
        }
    }

    // 记录 Operation（doc_id 从变更中推断）
    let mut doc_id = String::new();
    if !pending_changes.is_empty() {
        doc_id = pending_changes.first()
            .and_then(|c| c.before.as_ref().map(|s| s.document_id.clone()))
            .or_else(|| pending_changes.first().and_then(|c| c.after.as_ref().map(|s| s.document_id.clone())))
            .unwrap_or_default();
        let mut final_op = operation;
        final_op.document_id = doc_id.clone();
        oplog::record_operation(&conn, &final_op, &pending_changes)?;
    }

    Ok((BatchResult { id_map, results }, doc_id))
    })?;

    if !doc_id.is_empty() {
        event::EventBus::global().emit(BlockEvent::BlocksBatchChanged {
            document_id: doc_id,
            editor_id: editor_id_for_event,
        });
    }

    Ok(result)
}

// ─── 批量操作的内部实现（在已有 conn 上操作，不获取锁）──────────

fn batch_create_block(
    conn: &rusqlite::Connection,
    parent_id: &str,
    block_type: BlockType,
    content_type: Option<&crate::model::ContentType>,
    content: &str,
    properties: &HashMap<String, String>,
    after_id: Option<&str>,
) -> Result<Block, AppError> {
    validate_on_create(&block_type)?;

    // 验证 parent 存在且未删除
    let parent = repo::find_by_id(conn, parent_id)
        .map_err(|_| AppError::BadRequest(format!("父块 {} 不存在或已删除", parent_id)))?;

    // 推断 document_id
    let document_id = derive_document_id(&parent);

    let position = position::calculate_insert_position(conn, parent_id, after_id)?;
    let ct = content_type
        .map(|ct| ct.as_str().to_string())
        .unwrap_or_else(|| block_type.default_content_type().as_str().to_string());

    let id = generate_block_id();
    let now = now_iso();

    repo::insert_block(conn, &InsertBlockParams {
        id: id.clone(),
        parent_id: parent_id.to_string(),
        document_id,
        position,
        block_type: to_json(&block_type),
        content_type: ct,
        content: content.as_bytes().to_vec(),
        properties: to_json(properties),
        version: 1,
        status: "normal".to_string(),
        schema_version: 1,
        author: "system".to_string(),
        owner_id: None,
        encrypted: false,
        created: now.clone(),
        modified: now,
    })
    .map_err(|e| AppError::Internal(format!("批量创建 Block 失败: {}", e)))?;

    repo::find_by_id_raw(conn, &id)
        .map_err(|e| AppError::Internal(format!("查询刚创建的 Block 失败: {}", e)))
}

fn batch_update_block(
    conn: &rusqlite::Connection,
    id: &str,
    content: Option<&str>,
    properties: &Option<HashMap<String, String>>,
    properties_mode: &PropertiesMode,
) -> Result<u64, AppError> {
    let current = repo::find_by_id(conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在", id)))?;

    let new_content: Vec<u8> = content
        .map(|c| c.as_bytes().to_vec())
        .unwrap_or(current.content);

    let new_properties = merge_properties(&current.properties, properties.as_ref(), properties_mode);
    let properties_json = to_json(&new_properties);

    let now = now_iso();
    let rows = repo::update_content_and_props(
        conn, id, &new_content, &properties_json, &now,
    )
    .map_err(|e| AppError::Internal(format!("批量更新 Block 失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::NotFound(format!("Block {} 不存在", id)));
    }

    repo::get_version(conn, id)
        .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))
}

fn batch_delete_block(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<u64, AppError> {
    if id == crate::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可删除".to_string()));
    }

    let _current = repo::find_by_id(conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

    let descendant_ids = repo::find_descendant_ids_include_self(conn, id)
        .map_err(|e| AppError::Internal(format!("查询后代失败: {}", e)))?;

    // 单条 WHERE IN，替代 N 次循环
    let now = now_iso();
    repo::batch_update_status_if_not(conn, &descendant_ids, "deleted", &now, "deleted")
        .map_err(|e| AppError::Internal(format!("批量软删除失败: {}", e)))?;

    repo::get_version(conn, id)
        .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))
}

fn batch_move_block(
    conn: &rusqlite::Connection,
    id: &str,
    target_parent_id: Option<&str>,
    before_id: Option<&str>,
    after_id: Option<&str>,
) -> Result<u64, AppError> {
    if id == crate::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可移动".to_string()));
    }

    let current = repo::find_by_id(conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

    let target_parent = target_parent_id
        .unwrap_or(&current.parent_id)
        .to_string();

    let parent_changed = target_parent != current.parent_id;
    if parent_changed {
        if target_parent == id {
            return Err(AppError::BadRequest("不能将 Block 移动到自身下".to_string()));
        }

        let is_descendant = repo::check_is_descendant(conn, id, &target_parent)
            .unwrap_or(false);
        if is_descendant {
            return Err(AppError::CycleReference);
        }

        let parent_exists = repo::exists_normal(conn, &target_parent).unwrap_or(false);
        if !parent_exists {
            return Err(AppError::BadRequest(format!(
                "目标父块 {} 不存在或已删除",
                target_parent
            )));
        }
    }

    let new_position = position::calculate_move_position(
        conn, &target_parent, before_id, after_id,
    )?;

    let now = now_iso();
    let rows = repo::update_parent_position(
        conn, id, &target_parent, &new_position, &now,
    )
    .map_err(|e| AppError::Internal(format!("批量移动 Block 失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::NotFound(format!("Block {} 不存在", id)));
    }

    // 类型特化后置处理
    on_moved(conn, &MoveContext {
        block: &current,
        target_parent_id: &target_parent,
        new_position: &new_position,
        parent_changed,
    })?;

    repo::get_version(conn, id)
        .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))
}

// ─── Re-export Paragraph 特化操作 ──────────────────────────────

pub use super::paragraph::{split_block, merge_block};

// ─── 单元测试 ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::tests::init_test_db;
    use crate::model::BlockType;
    use super::super::document;

    // ── get_root ─────────────────────────────────────────

    #[test]
    fn get_root_returns_root_block() {
        let db = init_test_db();
        let root = get_block(&db, crate::model::ROOT_ID, false).unwrap();
        assert_eq!(root.id, crate::model::ROOT_ID);
        assert_eq!(root.parent_id, crate::model::ROOT_ID);
    }

    // ── create_block ─────────────────────────────────────

    #[test]
    fn create_block_under_root() {
        let db = init_test_db();

        let req = CreateBlockReq {
            parent_id: crate::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "Hello world".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        };

        let block = create_block(&db, req).unwrap();
        assert_eq!(block.parent_id, crate::model::ROOT_ID);
        assert_eq!(block.block_type, BlockType::Paragraph);
        assert_eq!(block.content, b"Hello world");
        assert_eq!(block.version, 1);
        assert_eq!(block.status, crate::model::BlockStatus::Normal);
    }

    #[test]
    fn create_block_with_after_id() {
        let db = init_test_db();

        // 先创建一个块
        let req1 = CreateBlockReq {
            parent_id: crate::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "first".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        };
        let block1 = create_block(&db, req1).unwrap();

        // 在 block1 之后插入
        let req2 = CreateBlockReq {
            parent_id: crate::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
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
            content_type: None,
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
            parent_id: crate::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
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
        assert_eq!(doc.parent_id, crate::model::ROOT_ID);
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
            parent_id: crate::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
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
            parent_id: crate::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
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
            parent_id: crate::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
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
            parent_id: crate::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
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
        assert_eq!(deleted.status, crate::model::BlockStatus::Deleted);
    }

    #[test]
    fn delete_block_promotes_children() {
        let db = init_test_db();

        let doc = document::create_document(&db, "Promote Doc".to_string(), None, None, None).unwrap();

        // 创建 heading + 子块
        let heading = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Heading { level: 2 },
            content_type: None,
            content: "Section".to_string(),
            properties: HashMap::new(),
            after_id: None,
            editor_id: None,
        }).unwrap();

        let child = create_block(&db, CreateBlockReq {
            parent_id: heading.id.clone(),
            block_type: BlockType::Paragraph,
            content_type: None,
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

        let result = delete_block(&db, crate::model::ROOT_ID, None);
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
            parent_id: crate::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
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
        assert_eq!(restored.status, crate::model::BlockStatus::Normal);
    }

    #[test]
    fn restore_normal_block_fails() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::model::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
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

        let result = move_block(&db, crate::model::ROOT_ID, MoveBlockReq {
            id: crate::model::ROOT_ID.to_string(),
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
            content_type: None,
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
                content_type: None,
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
        assert_eq!(result.root.parent_id, crate::model::ROOT_ID);
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
        let parent = document::create_document(&db, "Parent Doc".to_string(), Some(crate::model::ROOT_ID.to_string()), None, None).unwrap();
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
        let doc = document::create_document(&db, "Created Doc".to_string(), Some(crate::model::ROOT_ID.to_string()), None, None).unwrap();
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
            content_type: None,
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
}
