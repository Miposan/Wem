//! 批量操作
//!
//! 单次最多 50 条 Block 操作，在同一事务内执行。
//! create 操作支持 temp_id，后续操作可用 temp_id 引用该块。

use std::collections::{HashMap, HashSet};

use crate::api::request::{BatchOp, BatchReq, PropertiesMode};
use crate::api::response::{BatchOpResult, BatchResult};
use crate::error::AppError;
use crate::block_system::model::event::BlockEvent;
use crate::block_system::model::oplog::{Action, BlockSnapshot, ChangeType};
use crate::block_system::model::{generate_block_id, Block, BlockType};
use crate::repo::block_repo as repo;
use crate::repo::block_repo::InsertBlockParams;
use crate::repo::Db;
use crate::util::now_iso;

use super::helpers::{run_in_transaction, derive_document_id, to_json, merge_properties};
use super::traits::MoveContext;
use super::{event, oplog, position};

/// 批量执行多个 Block 操作
///
/// 单次最多 50 条操作，按数组顺序在同一事务内执行。
/// `create` 操作可指定 `temp_id`，后续操作可用 `temp_id` 引用该块。
/// 任何操作失败不影响其他操作，每条操作独立返回结果。
///
/// 参考 03-api-rest.md §3 "批量操作"
pub fn batch_operations(db: &Db, req: BatchReq) -> Result<BatchResult, AppError> {
    if req.operations.len() > 50 {
        return Err(AppError::BadRequest(
            "单次批量操作上限 50 条".to_string(),
        ));
    }

    let editor_id_for_event = req.editor_id.clone();
    let conn = crate::repo::lock_db(db);

    let result = run_in_transaction(&conn, || {

    let mut id_map: HashMap<String, String> = HashMap::new();
    let mut results: Vec<BatchOpResult> = Vec::with_capacity(req.operations.len());
    let mut pending_changes: Vec<crate::block_system::model::oplog::Change> = Vec::new();

    let operation = oplog::new_operation(Action::BatchOps, "", req.editor_id.clone());
    let op_id = operation.id.clone();

    fn resolve_id(id: &str, id_map: &HashMap<String, String>) -> String {
        id_map.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    for batch_op in req.operations {
        match batch_op {
            BatchOp::Create {
                temp_id,
                parent_id,
                block_type,
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
                    &content,
                    &properties,
                    resolved_after.as_deref(),
                );

                match result {
                    Ok(block) => {
                        let real_id = block.id.clone();
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

    let mut affected_doc_ids: HashSet<String> = HashSet::new();
    if !pending_changes.is_empty() {
        // 从所有变更中收集受影响的文档 ID
        for change in &pending_changes {
            if let Some(ref snap) = change.before {
                affected_doc_ids.insert(snap.document_id.clone());
            }
            if let Some(ref snap) = change.after {
                affected_doc_ids.insert(snap.document_id.clone());
            }
        }

        // 按文档分组变更，为每个文档创建独立的 Operation
        let mut changes_by_doc: HashMap<String, Vec<crate::block_system::model::oplog::Change>> = HashMap::new();
        for change in pending_changes {
            let doc_id = change.before.as_ref()
                .map(|s| s.document_id.clone())
                .or_else(|| change.after.as_ref().map(|s| s.document_id.clone()))
                .unwrap_or_default();
            changes_by_doc.entry(doc_id).or_default().push(change);
        }

        for (doc_id, doc_changes) in changes_by_doc {
            let per_doc_op = oplog::new_operation(
                Action::BatchOps, &doc_id, req.editor_id.clone(),
            );
            oplog::record_operation(&conn, &per_doc_op, &doc_changes)?;
        }
    }

    let doc_ids_vec: Vec<String> = affected_doc_ids.into_iter().collect();
    Ok((BatchResult { id_map, results }, doc_ids_vec))
    })?;

    // result 是 (BatchResult, Vec<String>) 元组
    let (batch_result, affected_doc_ids) = result;

    for doc_id in &affected_doc_ids {
        event::EventBus::global().emit(BlockEvent::BlocksBatchChanged {
            document_id: doc_id.clone(),
            editor_id: editor_id_for_event.clone(),
        });
    }

    Ok(batch_result)
}

// ─── 批量操作的内部实现（在已有 conn 上操作，不获取锁）──────────

fn batch_create_block(
    conn: &rusqlite::Connection,
    parent_id: &str,
    block_type: BlockType,
    content: &str,
    properties: &HashMap<String, String>,
    after_id: Option<&str>,
) -> Result<Block, AppError> {
    super::block::validate_on_create(&block_type)?;

    let parent = repo::find_by_id(conn, parent_id)
        .map_err(|_| AppError::BadRequest(format!("父块 {} 不存在或已删除", parent_id)))?;

    let document_id = derive_document_id(&parent);

    let position = position::calculate_insert_position(conn, parent_id, after_id)?;

    let id = generate_block_id();
    let now = now_iso();

    repo::insert_block(conn, &InsertBlockParams {
        id: id.clone(),
        parent_id: parent_id.to_string(),
        document_id,
        position,
        block_type: to_json(&block_type),
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
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;
    let new_content: Vec<u8> = content
        .map(|c| c.as_bytes().to_vec())
        .unwrap_or(current.content);

    let new_properties = merge_properties(&current.properties, properties.as_ref(), properties_mode);
    let properties_json = to_json(&new_properties);

    let now = now_iso();
    let rows = repo::update_block_fields(
        conn, id, &new_content, &properties_json, None, &now, Some(current.version),
    )
    .map_err(|e| AppError::Internal(format!("批量更新 Block 失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::VersionConflict(format!(
            "Block {} 版本冲突（期望版本 {}）", id, current.version
        )));
    }

    repo::get_version(conn, id)
        .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))
}

fn batch_delete_block(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<u64, AppError> {
    if id == crate::block_system::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可删除".to_string()));
    }

    let _current = repo::find_by_id(conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

    let descendant_ids = repo::find_descendant_ids_include_self(conn, id)
        .map_err(|e| AppError::Internal(format!("查询后代失败: {}", e)))?;

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
    if id == crate::block_system::model::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可移动".to_string()));
    }

    let current = repo::find_by_id(conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

    let target_parent = target_parent_id
        .unwrap_or(&current.parent_id)
        .to_string();

    let parent_changed = target_parent != current.parent_id;
    if parent_changed {
        if target_parent == id || repo::check_is_descendant(conn, id, &target_parent)
            .map_err(|e| AppError::Internal(format!("检查后代关系失败: {}", e)))?
        {
            return repo::get_version(conn, id)
                .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)));
        }

        let parent_exists = repo::exists_normal(conn, &target_parent)
            .map_err(|e| AppError::Internal(format!("检查父块存在性失败: {}", e)))?;
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
        conn, id, &target_parent, &new_position, &now, Some(current.version),
    )
    .map_err(|e| AppError::Internal(format!("批量移动 Block 失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::VersionConflict(format!(
            "Block {} 版本冲突（期望版本 {}）", id, current.version
        )));
    }

    super::block::on_moved(conn, &MoveContext {
        block: &current,
        target_parent_id: &target_parent,
        new_position: &new_position,
        parent_changed,
    })?;

    repo::get_version(conn, id)
        .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))
}
