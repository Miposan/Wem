//! 内容块编辑操作
//!
//! 提供文档内容 Block 的完整生命周期管理：创建、查询、更新、软删除/恢复、移动。
//! 所有函数接收 `&Db`（即 `Arc<Mutex<Connection>>`），内部自动加锁。
//! 文档级编排见 `service::document`。
//!
//! **架构分层**：
//! - 本文件（service::content）负责 Block 原子操作
//! - `service::document` 负责文档级编排（创建文档、获取文档树等）
//! - `repo::block_repo` 负责所有 SQL 操作
//! - `position` 模块负责 Fractional Index 位置计算

use std::collections::HashMap;

use crate::api::request::{BatchOp, BatchReq, CreateBlockReq, MoveBlockReq, MoveHeadingTreeReq, MoveDocumentTreeReq, PropertiesMode, UpdateBlockReq};
use crate::api::response::{
    BatchOpResult, BatchResult, DeleteResult, RestoreResult,
};
use crate::repo::block_repo as repo;
use crate::repo::block_repo::InsertBlockParams;
use crate::repo::Db;
use crate::error::AppError;
use crate::model::oplog::{Action, BlockSnapshot, ChangeType};
use crate::model::{generate_block_id, Block, BlockType};
use crate::service::{oplog, position};
use crate::util;
use util::now_iso;

// ─── 辅助函数 ──────────────────────────────────────────────────

/// 推断 document_id：给定父块，返回新建子块应使用的 document_id。
///
/// - 如果 parent 是 Document 类型 → document_id = parent.id（文档块自身就是文档根）
/// - 否则继承 parent.document_id（内容块指向所属文档）
fn derive_document_id(parent: &Block) -> String {
    if matches!(parent.block_type, BlockType::Document) {
        parent.id.clone()
    } else {
        parent.document_id.clone()
    }
}

/// 从 parent_id 推断 document_id（不加载完整 Block）。
/// Document 块的 document_id = 自身 id；其他块继承 parent 的 document_id。
fn derive_document_id_from_parent(
    conn: &rusqlite::Connection,
    parent_id: &str,
) -> Result<String, AppError> {
    let parent = repo::find_by_id(conn, parent_id)
        .map_err(|_| AppError::Internal(format!("查询 parent {} 失败", parent_id)))?;
    Ok(derive_document_id(&parent))
}

/// 校验 heading level 是否在合法范围 1..=6 内
fn validate_heading_level(block_type: &BlockType) -> Result<(), AppError> {
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
/// 参考 03-api-rest.md §3 "创建 Block"
pub fn create_block(db: &Db, req: CreateBlockReq) -> Result<Block, AppError> {
    let conn = crate::repo::lock_db(db);

    // 事务保护：插入 Block + 记录历史必须原子化
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| AppError::Internal(format!("开启事务失败: {}", e)))?;

    let result = (|| -> Result<Block, AppError> {
        // 1. 校验 block_type 合法性
        validate_heading_level(&req.block_type)?;

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
            block_type: serde_json::to_string(&req.block_type).unwrap_or_default(),
            content_type: content_type.as_str().to_string(),
            content: req.content.into_bytes(),
            properties: serde_json::to_string(&req.properties).unwrap_or_default(),
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
        let batch = oplog::new_batch(Action::Create, None, &document_id);
        let change = oplog::new_change(
            &batch.id, &id, ChangeType::Created,
            None,
            Some(BlockSnapshot::from_block(&block)),
        );
        oplog::record_batch(&conn, &batch, &[change])?;

        Ok(block)
    })();

    match &result {
        Ok(_) => { let _ = conn.execute_batch("COMMIT"); }
        Err(_) => { let _ = conn.execute_batch("ROLLBACK"); }
    }

    result
}

// ─── 查询 Block ────────────────────────────────────────────────

/// 获取单个 Block（不包含已删除的）
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
    let conn = crate::repo::lock_db(db);

    // 校验 heading level 合法性
    if let Some(ref bt) = req.block_type {
        validate_heading_level(bt)?;
    }

    // 开启事务：update + heading 重组必须原子化
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| AppError::Internal(format!("开启事务失败: {}", e)))?;

    let result = update_block_inner(&conn, id, req);

    match &result {
        Ok(_) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| AppError::Internal(format!("提交事务失败: {}", e)))?;
        }
        Err(_) => {
            // ROLLBACK 失败不影响原始错误，但记录日志
            if let Err(e) = conn.execute_batch("ROLLBACK") {
                tracing::error!("ROLLBACK 失败: {}", e);
            }
        }
    }

    result
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
    let new_content: Vec<u8> = req
        .content
        .map(|c| c.into_bytes())
        .unwrap_or_else(|| current.content.clone());

    // 3. 计算新 block_type 和 content_type
    let new_block_type = req.block_type.clone().unwrap_or(current.block_type.clone());

    // 4. 计算新 properties（merge 或 replace）
    let new_properties = match req.properties {
        Some(ref new_props) if req.properties_mode == PropertiesMode::Replace => new_props.clone(),
        Some(ref new_props) => {
            let mut merged = current.properties.clone();
            merged.extend(new_props.clone());
            merged
        }
        None => current.properties.clone(),
    };
    let properties_json = serde_json::to_string(&new_properties).unwrap_or_default();

    // 5. 写入数据库
    let new_content_type = req.block_type
        .as_ref()
        .map(|bt| bt.default_content_type())
        .unwrap_or(current.content_type.clone());

    write_block_updates(conn, id, &req.block_type, &new_content, &properties_json, &new_block_type, new_content_type.as_str())?;

    // 6. Heading 层级自动重组（仅 block_type 变化时）
    if req.block_type.is_some() {
        reorganize_heading(conn, id, &current, &new_block_type)?;
    }

    // 7. 查询并返回更新后的 Block
    let new_block = repo::find_by_id_raw(conn, id)
        .map_err(|e| AppError::Internal(format!("查询更新后的 Block 失败: {}", e)))?;

    // 8. 记录操作历史
    let batch = oplog::new_batch(Action::Update, None, &current.document_id);
    let change = oplog::block_change_pair(
        &batch.id, id, ChangeType::Updated, &current, &new_block,
    );
    oplog::record_batch(conn, &batch, &[change])?;

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
        let bt_str = serde_json::to_string(new_block_type).unwrap_or_default();
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

/// 步骤 6：Heading 层级自动重组
///
/// 不变量：heading(N) 只能嵌套在 heading(M) 内，且 M < N。
/// 当 block_type 变化时，通过三步操作维护此不变量：
///   a. 提升子块（如果曾经是 heading）
///   b. 逃逸校验（如果变为 heading，确保不被 ≥ 同级的 heading 包裹）
///   c. 吸收兄弟（以新 heading level 吸收后续低级别块）
fn reorganize_heading(
    conn: &rusqlite::Connection,
    id: &str,
    current: &Block,
    new_type: &BlockType,
) -> Result<(), AppError> {
    let was_heading = matches!(&current.block_type, BlockType::Heading { .. });
    let now_heading = matches!(new_type, BlockType::Heading { .. });

    // a. 如果曾经是 heading，先将所有子块提升到当前 parent
    if was_heading {
        promote_children(&conn, id, &current.parent_id, &current.position)?;
    }

    if now_heading {
        let new_level = match new_type {
            BlockType::Heading { level } => *level,
            _ => unreachable!(),
        };

        // b. 逃逸：如果父链中存在 heading(level >= new_level)，reparent 到正确祖先
        let (effective_parent_id, effective_position) =
            escape_heading_if_needed(&conn, id, current, new_level)?;

        // c. 吸收：在正确层级下，将后续低级别块变为子块
        absorb_siblings_after(&conn, id, &effective_parent_id, &effective_position, new_level)?;
    }

    Ok(())
}

// ─── Heading 层级重组辅助 ──────────────────────────────────────

/// 6a. 提升子块：将 heading 的所有直系子块 reparent 到 heading 的 parent
///
/// 子块按原顺序插入到 heading 之后、heading 原后续兄弟之前。
fn promote_children(
    conn: &rusqlite::Connection,
    heading_id: &str,
    heading_parent_id: &str,
    heading_position: &str,
) -> Result<(), AppError> {
    let children = repo::find_children(conn, heading_id)
        .map_err(|e| AppError::Internal(format!("查询子块失败: {}", e)))?;

    if children.is_empty() {
        return Ok(());
    }

    // 提升后子块与 heading 同级，document_id 应继承 heading 的 parent 的 document_id
    let new_document_id = derive_document_id_from_parent(conn, heading_parent_id)?;

    // 计算提升后的起始 position：紧接在 heading 之后
    let siblings_after_heading = repo::find_siblings_after(
        conn, heading_parent_id, heading_position,
    )
    .map_err(|e| AppError::Internal(format!("查询后续兄弟失败: {}", e)))?;

    let mut pos = if let Some(first_after) = siblings_after_heading.first() {
        position::generate_between(heading_position, &first_after.position)
    } else {
        position::generate_after(heading_position)
    };

    for child in &children {
        let now = now_iso();
        repo::update_parent_position_document_id(
            conn, &child.id, heading_parent_id, &pos, &new_document_id, &now,
        )
        .map_err(|e| AppError::Internal(format!("提升子块失败: {}", e)))?;
        pos = position::generate_after(&pos);
    }

    Ok(())
}

/// 6b. Heading 逃逸校验
///
/// 检查 heading(N) 的父链是否存在 heading(M >= N)。
/// 如果存在，将当前块 reparent 到最近的合法祖先（heading(level < N) 或非 heading 根），
/// 定位在"逃逸链"中最外层 heading 之后。
///
/// 返回逃逸后的有效 (parent_id, position)，供后续吸收逻辑使用。
fn escape_heading_if_needed(
    conn: &rusqlite::Connection,
    block_id: &str,
    current: &Block,
    new_level: u8,
) -> Result<(String, String), AppError> {
    let mut check_id = current.parent_id.clone();
    let mut escape_from_id = None; // 最外层需要逃逸的 heading ID

    // 沿父链向上走，找到第一个 level < N 的 heading 或非 heading 节点
    loop {
        let parent = repo::find_by_id(conn, &check_id)
            .map_err(|e| AppError::Internal(format!("查询祖先 {} 失败: {}", check_id, e)))?;

        match &parent.block_type {
            BlockType::Heading { level } if *level >= new_level => {
                // 此 heading 的 level >= 新 level，需要继续逃逸
                escape_from_id = Some(parent.id.clone());
                check_id = parent.parent_id.clone();
            }
            _ => {
                // 找到合法祖先：level < N 的 heading 或非 heading（文档根等）
                break;
            }
        }
    }

    // 无需逃逸
    let Some(escape_id) = escape_from_id else {
        return Ok((current.parent_id.clone(), current.position.clone()));
    };

    // 需要逃逸：target_parent 是 check_id（第一个合法祖先）
    let target_parent_id = check_id;

    // 读取逃逸点的 position（用于计算插入位置）
    let escape_block = repo::find_by_id(conn, &escape_id)
        .map_err(|e| AppError::Internal(format!("查询逃逸点 {} 失败: {}", escape_id, e)))?;

    // 在 target_parent 下，定位在 escape_block 之后
    let siblings_after_escape = repo::find_siblings_after(
        conn, &target_parent_id, &escape_block.position,
    )
    .map_err(|e| AppError::Internal(format!("查询逃逸点后续兄弟失败: {}", e)))?;

    let new_position = if let Some(first_after) = siblings_after_escape.first() {
        position::generate_between(&escape_block.position, &first_after.position)
    } else {
        position::generate_after(&escape_block.position)
    };

    // Reparent 当前块到 target_parent
    let now = now_iso();
    let new_document_id = derive_document_id_from_parent(conn, &target_parent_id)?;
    repo::update_parent_position_document_id(
        conn, block_id, &target_parent_id, &new_position, &new_document_id, &now,
    )
    .map_err(|e| AppError::Internal(format!("逃逸 reparent 失败: {}", e)))?;

    Ok((target_parent_id, new_position))
}

/// 6c. 吸收后续兄弟
///
/// 在 (parent_id, position) 对应的层级下，将 heading 之后的所有低级别块
/// reparent 为 heading 的子块，直到遇到 heading(level <= new_level) 为止。
fn absorb_siblings_after(
    conn: &rusqlite::Connection,
    heading_id: &str,
    parent_id: &str,
    position: &str,
    heading_level: u8,
) -> Result<(), AppError> {
    let siblings_after = repo::find_siblings_after(conn, parent_id, position)
        .map_err(|e| AppError::Internal(format!("查询后续兄弟失败: {}", e)))?;

    // 被吸收的块成为 heading 的子块，document_id 应继承 heading 的 document_id
    let new_document_id = derive_document_id_from_parent(conn, parent_id)?;

    let mut pos = position::calculate_insert_position(conn, heading_id, None)?;
    for sibling in &siblings_after {
        match &sibling.block_type {
            BlockType::Heading { level: sib_level } if *sib_level <= heading_level => {
                // 同级或更高级 heading → 停止吸收
                break;
            }
            _ => {
                let now = now_iso();
                repo::update_parent_position_document_id(
                    conn, &sibling.id, heading_id, &pos, &new_document_id, &now,
                )
                .map_err(|e| AppError::Internal(format!("reparent 失败: {}", e)))?;
                pos = position::generate_after(&pos);
            }
        }
    }

    Ok(())
}

// ─── 删除 Block（软删除）───────────────────────────────────────

/// 软删除 Block 及其所有后代
///
/// 流程：
/// 1. 用递归 CTE 查询所有后代（含自身）
/// 2. 批量 UPDATE status='deleted'
/// 3. 返回 `{ id, version, cascade_count }`
///
/// 软删除的 Block 不参与排序查询（`WHERE status != 'deleted'` 过滤），
/// 但可以通过 restore_block 恢复。
///
/// 参考 03-api-rest.md §3 "删除 Block"
pub fn delete_block(db: &Db, id: &str) -> Result<DeleteResult, AppError> {
    // 全局根块不可删除
    if id == crate::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可删除".to_string()));
    }

    let conn = crate::repo::lock_db(db);

    // 事务保护：删除 Block + 记录历史必须原子化
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| AppError::Internal(format!("开启事务失败: {}", e)))?;

    let result = (|| -> Result<DeleteResult, AppError> {
        // 1. 确认 Block 存在
        let _current = repo::find_by_id(&conn, id)
            .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

        // 2. 递归 CTE 查所有后代（含自身）
        let descendant_ids = repo::find_descendant_ids_include_self(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询后代失败: {}", e)))?;

        // cascade_count 不含自身
        let cascade_count = descendant_ids.len().saturating_sub(1) as u32;

        // 3. 删除前捕获所有受影响块的快照
        let before_blocks: Vec<Block> = descendant_ids.iter()
            .filter_map(|did| repo::find_by_id_raw(&conn, did).ok())
            .collect();

        // 4. 批量软删除
        let now = now_iso();
        repo::batch_update_status_if_not(&conn, &descendant_ids, "deleted", &now, "deleted")
            .map_err(|e| AppError::Internal(format!("批量软删除失败: {}", e)))?;

        // 5. 获取更新后的 version
        let new_version = repo::get_version(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))?;

        // 6. 记录操作历史（每个受影响的块一条 Change）
        let document_id = before_blocks.first().map(|b| b.document_id.clone()).unwrap_or_default();
        let batch = oplog::new_batch(Action::Delete, None, &document_id);
        let changes: Vec<_> = before_blocks.iter().map(|b| {
            oplog::new_change(
                &batch.id, &b.id, ChangeType::Deleted,
                Some(BlockSnapshot::from_block(b)),
                None,
            )
        }).collect();
        oplog::record_batch(&conn, &batch, &changes)?;

        Ok(DeleteResult {
            id: id.to_string(),
            version: new_version,
            cascade_count,
        })
    })();

    match &result {
        Ok(_) => { let _ = conn.execute_batch("COMMIT"); }
        Err(_) => { let _ = conn.execute_batch("ROLLBACK"); }
    }

    result
}

// ─── 恢复 Block ────────────────────────────────────────────────

/// 恢复已软删除的 Block 及其所有后代
///
/// 前置条件：
/// - 目标 Block 当前状态为 `deleted`
/// - 父块不能是 `deleted`（否则需要先恢复父块）
///
/// 参考 03-api-rest.md §3 "恢复 Block"
pub fn restore_block(db: &Db, id: &str) -> Result<RestoreResult, AppError> {
    let conn = crate::repo::lock_db(db);

    // 事务保护：恢复 Block + 记录历史必须原子化
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| AppError::Internal(format!("开启事务失败: {}", e)))?;

    let result = (|| -> Result<RestoreResult, AppError> {
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
        let batch = oplog::new_batch(Action::Restore, None, &document_id);
        let changes: Vec<_> = before_blocks.iter()
            .filter_map(|before| {
                let after = after_blocks.iter().find(|b| b.id == before.id)?;
                Some(oplog::block_change_pair(
                    &batch.id, &before.id, ChangeType::Restored, before, after,
                ))
            })
            .collect();
        oplog::record_batch(&conn, &batch, &changes)?;

        Ok(RestoreResult {
            id: id.to_string(),
            version: new_version,
            cascade_count,
        })
    })();

    match &result {
        Ok(_) => { let _ = conn.execute_batch("COMMIT"); }
        Err(_) => { let _ = conn.execute_batch("ROLLBACK"); }
    }

    result
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
/// 参考 02-block-tree.md §3.3、03-api-rest.md §3 "移动 Block"
pub fn move_block(db: &Db, id: &str, req: MoveBlockReq) -> Result<Block, AppError> {
    // 全局根块不可移动
    if id == crate::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可移动".to_string()));
    }

    // Document 类型自动转发到 document::move_document_tree
    // （文档移动 = 子树嫁接，属于文档级操作，见 service::document）
    {
        let conn = crate::repo::lock_db(db);
        let is_document = repo::find_by_id(&conn, id)
            .map(|b| matches!(b.block_type, BlockType::Document))
            .unwrap_or(false);
        drop(conn);

        if is_document {
            return crate::service::document::move_document_tree(db, MoveDocumentTreeReq {
                operation_id: req.operation_id,
                id: req.id.clone(),
                target_parent_id: req.target_parent_id,
                before_id: req.before_id,
                after_id: req.after_id,
            });
        }
    }

    let conn = crate::repo::lock_db(db);

    // 事务保护：移动涉及读 position + 写 parent/position，必须原子化
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| AppError::Internal(format!("开启事务失败: {}", e)))?;

    let result = (|| -> Result<Block, AppError> {
        // 1. 查询当前 Block
        let current = repo::find_by_id(&conn, id)
            .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

        // 2. 确定目标父块
        //
        //    优先级：
        //    a) 显式传入 target_parent_id → 使用它
        //    b) 传了 before_id/after_id → 从 sibling 的 parent_id 推导
        //    c) 都没传 → 保持当前父块
        let target_parent_id = match req.target_parent_id.as_deref() {
            Some(pid) => pid.to_string(),
            None => {
                let sibling_id = req
                    .before_id
                    .as_deref()
                    .or(req.after_id.as_deref());

                match sibling_id {
                    Some(sid) => {
                        let sibling = repo::find_by_id(&conn, sid)
                            .map_err(|_| AppError::NotFound(format!(
                                "定位块 {} 不存在或已删除", sid
                            )))?;
                        sibling.parent_id.clone()
                    }
                    None => current.parent_id.clone(),
                }
            }
        };

        // 3. 循环引用检测 + 父块验证（仅当父块改变时）
        let parent_changed = target_parent_id != current.parent_id;
        if parent_changed {
            if target_parent_id == id {
                return Err(AppError::BadRequest("不能将 Block 移动到自身下".to_string()));
            }

            let is_descendant = repo::check_is_descendant(&conn, id, &target_parent_id)
                .unwrap_or(false);

            if is_descendant {
                return Err(AppError::CycleReference);
            }

            let parent_exists = repo::exists_normal(&conn, &target_parent_id)
                .unwrap_or(false);

            if !parent_exists {
                return Err(AppError::BadRequest(format!(
                    "目标父块 {} 不存在或已删除",
                    target_parent_id
                )));
            }
        }

        // 4. 计算新 position
        let new_position = position::calculate_move_position(
            &conn,
            &target_parent_id,
            req.before_id.as_deref(),
            req.after_id.as_deref(),
        )?;

        // 5. UPDATE block
        let now = now_iso();
        let rows = repo::update_parent_position(
            &conn,
            id,
            &target_parent_id,
            &new_position,
            &now,
        )
        .map_err(|e| AppError::Internal(format!("移动 Block 失败: {}", e)))?;

        if rows == 0 {
            return Err(AppError::NotFound(format!("Block {} 不存在", id)));
        }

        // 6. 记录历史（before = 原位置, after = 新位置）
        let after = repo::find_by_id_raw(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询移动后的 Block 失败: {}", e)))?;

        let batch = oplog::new_batch(Action::Move, req.operation_id.clone(), &current.document_id);
        let change = oplog::block_change_pair(
            &batch.id, id, ChangeType::Moved, &current, &after,
        );
        oplog::record_batch(&conn, &batch, &[change])?;

        Ok(after)
    })();

    // 提交或回滚事务
    match &result {
        Ok(_) => conn.execute_batch("COMMIT")
            .map_err(|e| AppError::Internal(format!("提交事务失败: {}", e)))?,
        Err(_) => { let _ = conn.execute_batch("ROLLBACK"); }
    }

    result
}

// ─── 移动子树 ──────────────────────────────────────────────────

/// 移动 Heading 子树（折叠拖拽场景）
///
/// heading + 其下属所有内容块作为一个整体移动到新位置。
/// 同文档内移动为主，跨文档时自动更新后代的 document_id。
///
/// 流程：
/// 1. 验证根块是 Heading 且非全局根
/// 2. 从 before_id / after_id 推导 target_parent_id
/// 3. 循环引用检测
/// 4. 计算新 position → 移动根块
/// 5. 跨文档时批量更新后代 document_id
/// 6. 记录历史
pub fn move_heading_tree(db: &Db, req: MoveHeadingTreeReq) -> Result<Block, AppError> {
    let id = &req.id;

    if id == crate::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可移动".to_string()));
    }

    let conn = crate::repo::lock_db(db);

    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| AppError::Internal(format!("开启事务失败: {}", e)))?;

    let result = (|| -> Result<Block, AppError> {
        // 1. 验证根块是 Heading
        let current = repo::find_by_id(&conn, id)
            .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

        if !matches!(current.block_type, BlockType::Heading { .. }) {
            return Err(AppError::BadRequest(
                "move_heading_tree 只能移动 Heading 类型".to_string(),
            ));
        }

        // 2. 确定目标父块（从 sibling 的 parent_id 推导）
        let target_parent_id = resolve_target_parent(
            &conn, &req.before_id, &req.after_id, &current.parent_id,
        )?;

        // 3. 循环引用检测
        validate_no_cycle(&conn, id, &target_parent_id, &current.parent_id)?;

        // 4. 计算新 position + 推断 document_id
        let new_position = position::calculate_move_position(
            &conn, &target_parent_id,
            req.before_id.as_deref(), req.after_id.as_deref(),
        )?;

        let new_document_id = derive_document_id_from_parent(&conn, &target_parent_id)?;
        let cross_document = new_document_id != current.document_id;

        // 5. 移动根块（parent_id + position + document_id）
        let now = now_iso();
        let rows = repo::update_parent_position_document_id(
            &conn, id, &target_parent_id, &new_position, &new_document_id, &now,
        )
        .map_err(|e| AppError::Internal(format!("移动根块失败: {}", e)))?;

        if rows == 0 {
            return Err(AppError::NotFound(format!("Block {} 不存在", id)));
        }

        // 6. 跨文档时：批量更新后代 document_id
        if cross_document {
            let descendant_ids = repo::find_descendant_ids(&conn, id)
                .map_err(|e| AppError::Internal(format!("查询子树后代失败: {}", e)))?;

            if !descendant_ids.is_empty() {
                repo::batch_update_document_id(
                    &conn, &descendant_ids, &new_document_id, &now_iso(),
                )
                .map_err(|e| AppError::Internal(format!("更新后代 document_id 失败: {}", e)))?;
            }
        }

        // 7. 记录历史
        let after = repo::find_by_id_raw(&conn, id)
            .map_err(|e| AppError::Internal(format!("查询移动后的 Block 失败: {}", e)))?;

        let batch = oplog::new_batch(Action::Move, req.operation_id.clone(), &current.document_id);
        let mut changes = vec![oplog::block_change_pair(
            &batch.id, id, ChangeType::Moved, &current, &after,
        )];

        // 跨文档移动时，后代 document_id 变更也要记录
        if cross_document {
            let descendant_ids = repo::find_descendant_ids(&conn, id)
                .map_err(|e| AppError::Internal(format!("查询后代失败: {}", e)))?;
            for did in &descendant_ids {
                if let Ok(desc_after) = repo::find_by_id_raw(&conn, did) {
                    changes.push(oplog::new_change(
                        &batch.id, did, ChangeType::Moved,
                        None,
                        Some(BlockSnapshot::from_block(&desc_after)),
                    ));
                }
            }
        }

        oplog::record_batch(&conn, &batch, &changes)?;
        Ok(after)
    })();

    finish_tx(&conn, &result)?;
    result
}



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

/// 事务提交或回滚
pub(crate) fn finish_tx(
    conn: &rusqlite::Connection,
    result: &Result<Block, AppError>,
) -> Result<(), AppError> {
    match result {
        Ok(_) => conn.execute_batch("COMMIT")
            .map_err(|e| AppError::Internal(format!("提交事务失败: {}", e))),
        Err(_) => { let _ = conn.execute_batch("ROLLBACK"); Ok(()) }
    }
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

    let conn = crate::repo::lock_db(db);

    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| AppError::Internal(format!("开启事务失败: {}", e)))?;

    let mut id_map: HashMap<String, String> = HashMap::new();
    let mut results: Vec<BatchOpResult> = Vec::with_capacity(req.operations.len());
    let mut pending_changes: Vec<crate::model::oplog::Change> = Vec::new();

    /// 解析 block_id：如果是 temp_id 映射中存在的，替换为真实 ID
    fn resolve_id(id: &str, id_map: &HashMap<String, String>) -> String {
        id_map.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    for op in req.operations {
        match op {
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
                            "", // batch_id 后面统一设置
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
                                "", &resolved_id, ChangeType::Updated, b, a,
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
                                "", &resolved_id, ChangeType::Deleted,
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
                                "", &resolved_id, ChangeType::Moved, b, a,
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

    // 将所有变更记录为一个 Batch（统一设置 batch_id）
    if !pending_changes.is_empty() {
        // batch_operations 可能跨文档，使用第一个变更的 document_id
        let doc_id = pending_changes.first()
            .and_then(|c| c.before.as_ref().map(|s| s.document_id.clone()))
            .or_else(|| pending_changes.first().and_then(|c| c.after.as_ref().map(|s| s.document_id.clone())))
            .unwrap_or_default();
        let batch = oplog::new_batch(Action::BatchOps, req.operation_id, &doc_id);
        for change in &mut pending_changes {
            change.batch_id = batch.id.clone();
        }
        oplog::record_batch(&conn, &batch, &pending_changes)?;
    }

    // 单条操作失败不影响其他操作，但整个批次在同一个事务内
    match conn.execute_batch("COMMIT") {
        Ok(()) => Ok(BatchResult { id_map, results }),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(AppError::Internal(format!("提交批量事务失败: {}", e)))
        }
    }
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
        block_type: serde_json::to_string(&block_type).unwrap_or_default(),
        content_type: ct,
        content: content.as_bytes().to_vec(),
        properties: serde_json::to_string(properties).unwrap_or_default(),
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

    let new_properties = match properties {
        Some(new_props) if properties_mode == &PropertiesMode::Replace => new_props.clone(),
        Some(new_props) => {
            let mut merged = current.properties.clone();
            merged.extend(new_props.clone());
            merged
        }
        None => current.properties.clone(),
    };
    let properties_json = serde_json::to_string(&new_properties).unwrap_or_default();

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

    repo::get_version(conn, id)
        .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))
}

// ─── Split / Merge 意图操作 ──────────────────────────────────

use crate::api::request::{MergeReq, SplitReq};
use crate::api::response::{MergeResult, SplitResult};

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
    let conn = crate::repo::lock_db(db);

    // 开启事务：更新当前块 + 插入新块必须原子化
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| AppError::Internal(format!("开启事务失败: {}", e)))?;

    let result = split_block_inner(&conn, id, req);

    match &result {
        Ok(_) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| AppError::Internal(format!("提交事务失败: {}", e)))?;
        }
        Err(_) => {
            let _ = conn.execute_batch("ROLLBACK");
        }
    }

    result
}

/// split_block 的核心逻辑（在事务内执行）
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
    let properties_json = serde_json::to_string(&current.properties).unwrap_or_default();

    let rows = repo::update_content_and_props(
        &conn, id, &new_content, &properties_json, &now,
    )
    .map_err(|e| AppError::Internal(format!("更新 Block 失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::NotFound(format!("Block {} 不存在", id)));
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
    let new_block_type = req.new_block_type.unwrap_or(BlockType::Paragraph);
    let content_type = new_block_type.default_content_type();

    // 6. 创建新块
    let new_id = generate_block_id();
    let document_id = derive_document_id(&current);

    repo::insert_block(&conn, &InsertBlockParams {
        id: new_id.clone(),
        parent_id: new_parent_id,
        document_id,
        position,
        block_type: serde_json::to_string(&new_block_type).unwrap_or_default(),
        content_type: content_type.as_str().to_string(),
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
    let batch = oplog::new_batch(Action::Split, req.operation_id, &current.document_id);
    let changes = vec![
        // Change 1: 原块被更新（内容从完整变为前半）
        oplog::block_change_pair(
            &batch.id, id, ChangeType::Updated, &current, &updated_block,
        ),
        // Change 2: 新块被创建
        oplog::new_change(
            &batch.id, &new_id, ChangeType::Created,
            None,
            Some(BlockSnapshot::from_block(&new_block)),
        ),
    ];
    oplog::record_batch(&conn, &batch, &changes)?;

    Ok(SplitResult {
        updated_block,
        new_block,
    })
}

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
pub fn merge_block(db: &Db, id: &str, _req: MergeReq) -> Result<MergeResult, AppError> {
    let conn = crate::repo::lock_db(db);

    // 开启事务：合并内容 + reparent 子块 + 软删除必须原子化
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| AppError::Internal(format!("开启事务失败: {}", e)))?;

    let result = merge_block_inner(&conn, id);

    match &result {
        Ok(_) => {
            conn.execute_batch("COMMIT")
                .map_err(|e| AppError::Internal(format!("提交事务失败: {}", e)))?;
        }
        Err(_) => {
            let _ = conn.execute_batch("ROLLBACK");
        }
    }

    result
}

/// merge_block 的核心逻辑（在事务内执行）
fn merge_block_inner(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<MergeResult, AppError> {

    // 1. 查询当前 Block
    let current = repo::find_by_id(&conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

    // 全局根块不可合并
    if id == crate::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可合并".to_string()));
    }

    // 2. 确定合并目标：优先前驱兄弟，回退到父块
    //
    // 在 DFS 顺序中，一个块的"前一个"有两种情况：
    //   a) 有前驱兄弟 → 合并到前驱兄弟（同父同级）
    //   b) 无前驱兄弟 → 合并到父块（DFS 中父块就是前一个）
    let prev_sibling = repo::find_prev_sibling(&conn, &current.parent_id, &current.position)
        .map_err(|e| AppError::Internal(format!("查询前驱兄弟失败: {}", e)))?;

    let (target, merge_into_parent) = match prev_sibling {
        Some(s) => (s, false),
        None => {
            // 无前驱兄弟 → 回退到父块
            if current.parent_id == crate::model::ROOT_ID {
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

    // 3. 合并内容
    let target_text = String::from_utf8_lossy(&target.content);
    let current_text = String::from_utf8_lossy(&current.content);
    let merged_content = format!("{}{}", target_text, current_text);

    // 4. 更新合并目标块
    let now = now_iso();
    let properties_json = serde_json::to_string(&target.properties).unwrap_or_default();

    let rows = repo::update_content_and_props(
        &conn,
        &target.id,
        merged_content.as_bytes(),
        &properties_json,
        &now,
    )
    .map_err(|e| AppError::Internal(format!("更新合并目标块失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::NotFound(format!(
            "合并目标 Block {} 不存在",
            target.id
        )));
    }

    // 5. 将当前块的子块 reparent 到合并目标
    let children = repo::find_children(&conn, id)
        .map_err(|e| AppError::Internal(format!("查询子块失败: {}", e)))?;

    if !children.is_empty() {
        // 子块按原顺序插入到 target（或 current 之后的位置）
        let reparent_target_id = &target.id;
        let new_document_id = derive_document_id_from_parent(conn, reparent_target_id)?;

        // 计算插入起始位置：在 current 之后
        let siblings_after =
            repo::find_siblings_after(&conn, &current.parent_id, &current.position)
                .map_err(|e| AppError::Internal(format!("查询后续兄弟失败: {}", e)))?;

        let mut pos = if merge_into_parent {
            // 合并到父块时，子块插入到 current 之后、current 的后续兄弟之前
            if let Some(first_after) = siblings_after.first() {
                position::generate_between(&current.position, &first_after.position)
            } else {
                position::generate_after(&current.position)
            }
        } else {
            // 合并到前驱兄弟时，子块追加到 target 的子块末尾
            // 使用 target 的 position 作为参考点，在其子块之后追加
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
                &conn, &child.id, reparent_target_id, &pos, &new_document_id, &t,
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
    let batch = oplog::new_batch(Action::Merge, None, &current.document_id);
    let changes = vec![
        // Change 1: 目标块被更新（追加了内容）
        oplog::block_change_pair(
            &batch.id, &target.id, ChangeType::Updated, &target, &merged_block,
        ),
        // Change 2: 当前块被删除
        oplog::new_change(
            &batch.id, id, ChangeType::Deleted,
            Some(BlockSnapshot::from_block(&current)),
            None,
        ),
    ];
    oplog::record_batch(&conn, &batch, &changes)?;

    Ok(MergeResult {
        merged_block,
        deleted_block_id: id.to_string(),
    })
}

// ─── 单元测试 ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::tests::init_test_db;
    use crate::model::BlockType;
    use crate::service::document;

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
            operation_id: None,
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
            operation_id: None,
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
            operation_id: None,
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
            operation_id: None,
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
            operation_id: None,
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
            &db, "Parent Doc".to_string(), None, None,
        ).unwrap();

        // 创建子文档
        let child = document::create_document(
            &db,
            "Child Doc".to_string(),
            Some(parent.id.clone()),
            None,
        ).unwrap();

        assert_eq!(child.parent_id, parent.id);
        assert_eq!(child.content, b"Child Doc");
    }

    #[test]
    fn create_document_with_position() {
        let db = init_test_db();

        let doc1 = document::create_document(&db, "Doc 1".to_string(), None, None).unwrap();
        let doc2 = document::create_document(&db, "Doc 2".to_string(), None, Some(doc1.id.clone())).unwrap();

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
            operation_id: None,
        }).unwrap();

        let updated = update_block(&db, &created.id, UpdateBlockReq {
            id: created.id.clone(),
            block_type: None,
            content: Some("updated".to_string()),
            properties: None,
            properties_mode: PropertiesMode::Merge,
            operation_id: None,
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
            operation_id: None,
        }).unwrap();

        let mut new_props = HashMap::new();
        new_props.insert("key2".to_string(), "val2".to_string());
        let updated = update_block(&db, &created.id, UpdateBlockReq {
            id: created.id.clone(),
            block_type: None,
            content: None,
            properties: Some(new_props),
            properties_mode: PropertiesMode::Merge,
            operation_id: None,
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
            operation_id: None,
        }).unwrap();

        let mut new_props = HashMap::new();
        new_props.insert("key2".to_string(), "val2".to_string());
        let updated = update_block(&db, &created.id, UpdateBlockReq {
            id: created.id.clone(),
            block_type: None,
            content: None,
            properties: Some(new_props),
            properties_mode: PropertiesMode::Replace,
            operation_id: None,
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
            operation_id: None,
        }).unwrap();

        let result = delete_block(&db, &created.id).unwrap();
        assert_eq!(result.id, created.id);
        assert_eq!(result.cascade_count, 0); // 叶子块无后代

        // get_block 不再能查到
        assert!(get_block(&db, &created.id, false).is_err());

        // 但 get_block_include_deleted 可以
        let deleted = get_block(&db, &created.id, true).unwrap();
        assert_eq!(deleted.status, crate::model::BlockStatus::Deleted);
    }

    #[test]
    fn delete_block_cascades_to_children() {
        let db = init_test_db();

        let doc = document::create_document(&db, "Cascade Doc".to_string(), None, None).unwrap();

        let result = delete_block(&db, &doc.id).unwrap();
        assert!(result.cascade_count >= 1); // 至少包含默认段落

        // 文档和段落都不可查
        assert!(get_block(&db, &doc.id, false).is_err());
    }

    #[test]
    fn delete_root_block_forbidden() {
        let db = init_test_db();

        let result = delete_block(&db, crate::model::ROOT_ID);
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
            operation_id: None,
        }).unwrap();

        delete_block(&db, &created.id).unwrap();

        let result = restore_block(&db, &created.id).unwrap();
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
            operation_id: None,
        }).unwrap();

        let result = restore_block(&db, &created.id);
        assert!(result.is_err());
    }

    // ── move_block ───────────────────────────────────────

    #[test]
    fn move_block_to_new_parent() {
        let db = init_test_db();

        let doc1 = document::create_document(&db, "Doc 1".to_string(), None, None).unwrap();
        let doc2 = document::create_document(&db, "Doc 2".to_string(), None, None).unwrap();

        // 将 doc2 移动到 doc1 下
        let moved = move_block(&db, &doc2.id, MoveBlockReq {
            id: doc2.id.clone(),
            target_parent_id: Some(doc1.id.clone()),
            before_id: None,
            after_id: None,
            operation_id: None,
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
            operation_id: None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn move_block_cycle_detection() {
        let db = init_test_db();

        let doc = document::create_document(&db, "Doc".to_string(), None, None).unwrap();

        // 试图将 doc 移动到自身下
        let result = move_block(&db, &doc.id, MoveBlockReq {
            id: doc.id.clone(),
            target_parent_id: Some(doc.id.clone()),
            before_id: None,
            after_id: None,
            operation_id: None,
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

        document::create_document(&db, "Doc 1".to_string(), None, None).unwrap();
        document::create_document(&db, "Doc 2".to_string(), None, None).unwrap();

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

        let doc = document::create_document(&db, "Tree Doc".to_string(), None, None).unwrap();
        let child = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Heading { level: 2 },
            content_type: None,
            content: "Section".to_string(),
            properties: HashMap::new(),
            after_id: None,
            operation_id: None,
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

        let doc = document::create_document(&db, "Pagination Doc".to_string(), None, None).unwrap();

        // 创建 3 个额外子块（已有 1 个默认段落）
        for i in 0..3 {
            create_block(&db, CreateBlockReq {
                parent_id: doc.id.clone(),
                block_type: BlockType::Paragraph,
                content_type: None,
                content: format!("para {}", i),
                properties: HashMap::new(),
                after_id: None,
                operation_id: None,
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
}
