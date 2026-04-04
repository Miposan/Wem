//! Block CRUD 业务逻辑
//!
//! 提供 Block 的完整生命周期管理：创建、查询、更新、软删除/恢复、移动。
//! 所有函数接收 `&Db`（即 `Arc<Mutex<Connection>>`），内部自动加锁。
//!
//! **架构分层**：
//! - 本文件（service）负责业务逻辑：校验、计算、编排
//! - `db::repository` 负责所有 SQL 操作：查询、插入、更新、删除
//! - `service::fractional` 负责 Fractional Index 位置计算（纯计算，无 SQL）
//!
//! 参考：
//! - 01-block-model.md §1~§6（Block 结构、类型系统）
//! - 02-block-tree.md §2~§3（Fractional Index、树操作）
//! - 03-api-rest.md §2~§3（API 语义）

use std::collections::HashMap;

use crate::api::request::{CreateBlockReq, MoveBlockReq, UpdateBlockReq};
use crate::api::response::{
    ChildrenResult, DeleteResult, DocumentTreeResult, RestoreResult,
};
use crate::db::repository as repo;
use crate::db::repository::InsertBlockParams;
use crate::db::Db;
use crate::error::AppError;
use crate::model::{generate_block_id, Block, BlockType};
use crate::service::fractional;

// ─── 辅助函数 ──────────────────────────────────────────────────

/// 生成当前时间的 ISO 8601 字符串（毫秒精度）
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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
    let conn = db.lock().unwrap();

    // 1. 验证 parent 存在且未删除
    let _parent = repo::find_by_id(&conn, &req.parent_id)
        .map_err(|_| AppError::BadRequest(format!("父块 {} 不存在或已删除", req.parent_id)))?;

    // 2. 计算 position
    let position =
        calculate_insert_position(&conn, &req.parent_id, req.after_id.as_deref())?;

    // 3. 推断 content_type
    let content_type = req
        .content_type
        .clone()
        .unwrap_or_else(|| req.block_type.default_content_type());

    // 4. 生成 ID 和时间戳
    let id = generate_block_id();
    let now = now_iso();

    // 5. INSERT（通过 repository）
    repo::insert_block(&conn, &InsertBlockParams {
        id: id.clone(),
        parent_id: req.parent_id,
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

    // 7. 查询并返回完整 Block
    repo::find_by_id_raw(&conn, &id)
        .map_err(|e| AppError::Internal(format!("查询刚创建的 Block 失败: {}", e)))
}

// ─── 查询 Block ────────────────────────────────────────────────

/// 获取单个 Block（不包含已删除的）
pub fn get_block(db: &Db, id: &str) -> Result<Block, AppError> {
    get_block_impl(db, id, false)
}

/// 获取单个 Block（可选择是否包含已删除的）
pub fn get_block_include_deleted(db: &Db, id: &str, include_deleted: bool) -> Result<Block, AppError> {
    get_block_impl(db, id, include_deleted)
}

fn get_block_impl(db: &Db, id: &str, include_deleted: bool) -> Result<Block, AppError> {
    let conn = db.lock().unwrap();
    let result = if include_deleted {
        repo::find_by_id_raw(&conn, id)
    } else {
        repo::find_by_id(&conn, id)
    };
    result.map_err(|_| AppError::NotFound(format!("Block {} 不存在", id)))
}

/// 获取文档树（扁平列表）
///
/// 返回文档根 Block + 所有未删除的后代 Block。
/// 后代通过递归 CTE 按 parent_id 遍历，按 position ASC 排序。
/// 前端根据 parent_id 重建嵌套结构。
///
/// 参考 03-api-rest.md §2 "获取文档 Block Tree"
pub fn get_document_tree(db: &Db, doc_id: &str) -> Result<DocumentTreeResult, AppError> {
    let conn = db.lock().unwrap();

    // 查询文档根 Block
    let root = repo::find_by_id(&conn, doc_id)
        .map_err(|_| AppError::NotFound(format!("文档 {} 不存在", doc_id)))?;

    // 递归 CTE 查询所有未删除的后代（不含根节点自身）
    let blocks = repo::find_descendants(&conn, doc_id)
        .map_err(|e| AppError::Internal(format!("查询文档树失败: {}", e)))?;

    Ok(DocumentTreeResult { root, blocks })
}

/// 列出所有根文档
///
/// 根文档 = 全局根块 "/" 的直接子文档。
/// 按 position 排序。
///
/// 参考 03-api-rest.md §2 "列出根文档"
pub fn list_root_documents(db: &Db) -> Result<Vec<Block>, AppError> {
    let conn = db.lock().unwrap();
    repo::find_root_documents(&conn)
        .map_err(|e| AppError::Internal(format!("查询根文档失败: {}", e)))
}

/// 获取全局根块 "/"
///
/// 根块是所有文档的唯一挂载点，固定 ID = `db::ROOT_ID`。
/// 前端可用于渲染导航树的起点。
pub fn get_root(db: &Db) -> Result<Block, AppError> {
    let conn = db.lock().unwrap();
    repo::find_by_id_raw(&conn, crate::db::ROOT_ID)
        .map_err(|_| AppError::Internal("全局根块不存在（数据库未正确初始化）".to_string()))
}

/// 获取子块列表（分页）
///
/// 返回指定 Block 的直接子块，按 position 排序。
/// 使用 cursor（position 值）实现游标分页。
///
/// 参考 03-api-rest.md §3 "获取子 Block 列表"
pub fn get_children(
    db: &Db,
    parent_id: &str,
    limit: u32,
    cursor: Option<&str>,
) -> Result<ChildrenResult, AppError> {
    let conn = db.lock().unwrap();

    // 验证父块存在
    repo::exists_normal(&conn, parent_id)
        .map_err(|e| AppError::Internal(format!("验证父块失败: {}", e)))?
        .then_some(())
        .ok_or_else(|| AppError::NotFound(format!("Block {} 不存在", parent_id)))?;

    // 限制 limit 范围 [1, 500]
    let limit = limit.clamp(1, 500);
    // 多取一条判断是否有更多数据
    let fetch_limit = limit + 1;

    let blocks = repo::find_children_paginated(&conn, parent_id, cursor, fetch_limit)
        .map_err(|e| AppError::Internal(format!("查询子块失败: {}", e)))?;

    // 判断是否有更多数据
    let has_more = blocks.len() > limit as usize;
    let blocks = if has_more {
        blocks[..limit as usize].to_vec()
    } else {
        blocks
    };
    let next_cursor = if has_more {
        blocks.last().map(|b| b.position.clone())
    } else {
        None
    };

    Ok(ChildrenResult {
        blocks,
        has_more,
        next_cursor,
    })
}

// ─── 更新 Block ────────────────────────────────────────────────

/// 更新 Block 内容和/或属性
///
/// 流程：
/// 1. 查询当前 Block（含 version）
/// 2. 乐观锁校验：`req.version == block.version`，否则返回 40901
/// 3. 计算新的 content / properties
/// 4. `UPDATE ... SET version=version+1 WHERE id=? AND version=?`（双重保护）
/// 5. 返回更新后的 Block
///
/// 参考 03-api-rest.md §3 "更新 Block"
pub fn update_block(db: &Db, id: &str, req: UpdateBlockReq) -> Result<Block, AppError> {
    let conn = db.lock().unwrap();

    // 1. 查询当前 Block
    let current = repo::find_by_id(&conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在", id)))?;

    // 2. 乐观锁校验
    if req.version != current.version {
        return Err(AppError::VersionConflict(current.version));
    }

    // 3. 计算新 content
    let new_content: Vec<u8> = req
        .content
        .map(|c| c.into_bytes())
        .unwrap_or(current.content);

    // 4. 计算新 properties（merge 或 replace）
    let new_properties = match req.properties {
        Some(ref new_props) if req.properties_mode == "replace" => new_props.clone(),
        Some(ref new_props) => {
            // merge 模式：请求中的 key 合并到现有 properties
            let mut merged = current.properties.clone();
            merged.extend(new_props.clone());
            merged
        }
        None => current.properties.clone(),
    };
    let properties_json = serde_json::to_string(&new_properties).unwrap_or_default();

    // 5. UPDATE with optimistic lock（WHERE version = ? 双重保护并发冲突）
    let now = now_iso();
    let rows = repo::update_content_and_props(
        &conn, id, &new_content, &properties_json, &now, req.version,
    )
    .map_err(|e| AppError::Internal(format!("更新 Block 失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::VersionConflict(current.version));
    }

    // 6. 查询并返回更新后的 Block
    repo::find_by_id_raw(&conn, id)
        .map_err(|e| AppError::Internal(format!("查询更新后的 Block 失败: {}", e)))
}

// ─── 删除 Block（软删除）───────────────────────────────────────

/// 软删除 Block 及其所有后代
///
/// 流程：
/// 1. 版本校验
/// 2. 用递归 CTE 查询所有后代（含自身）
/// 3. 批量 UPDATE status='deleted'
/// 4. 返回 `{ id, version, cascade_count }`
///
/// 软删除的 Block 不参与排序查询（`WHERE status != 'deleted'` 过滤），
/// 但可以通过 restore_block 恢复。
///
/// 参考 03-api-rest.md §3 "删除 Block"
pub fn delete_block(db: &Db, id: &str, version: u64) -> Result<DeleteResult, AppError> {
    // 全局根块不可删除
    if id == crate::db::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可删除".to_string()));
    }

    let conn = db.lock().unwrap();

    // 1. 版本校验
    let current = repo::find_by_id(&conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

    if version != current.version {
        return Err(AppError::VersionConflict(current.version));
    }

    // 2. 递归 CTE 查所有后代（含自身）
    let descendant_ids = repo::find_descendant_ids_include_self(&conn, id)
        .map_err(|e| AppError::Internal(format!("查询后代失败: {}", e)))?;

    // cascade_count 不含自身
    let cascade_count = descendant_ids.len().saturating_sub(1) as u32;

    // 3. 批量软删除
    let now = now_iso();
    for did in &descendant_ids {
        repo::update_status_if_not(&conn, did, "deleted", &now, "deleted")
            .map_err(|e| AppError::Internal(format!("软删除 Block 失败: {}", e)))?;
    }

    // 4. 获取更新后的 version
    let new_version = repo::get_version(&conn, id)
        .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))?;

    Ok(DeleteResult {
        id: id.to_string(),
        version: new_version,
        cascade_count,
    })
}

// ─── 恢复 Block ────────────────────────────────────────────────

/// 恢复已软删除的 Block 及其所有后代
///
/// 前置条件：
/// - 目标 Block 当前状态为 `deleted`
/// - 父块不能是 `deleted`（否则需要先恢复父块）
///
/// 参考 03-api-rest.md §3 "恢复 Block"
pub fn restore_block(db: &Db, id: &str, version: u64) -> Result<RestoreResult, AppError> {
    let conn = db.lock().unwrap();

    // 1. 查询当前 Block（必须是 deleted 状态）
    let current = repo::find_deleted(&conn, id)
        .map_err(|_| AppError::BadRequest(format!("Block {} 不是已删除状态", id)))?;

    if version != current.version {
        return Err(AppError::VersionConflict(current.version));
    }

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
    let to_restore = repo::find_deleted_descendant_ids(&conn, id)
        .map_err(|e| AppError::Internal(format!("查询已删除后代失败: {}", e)))?;

    let cascade_count = to_restore.len() as u32;

    // 4. 恢复自身
    let now = now_iso();
    repo::update_status(&conn, id, "normal", &now)
        .map_err(|e| AppError::Internal(format!("恢复 Block 失败: {}", e)))?;

    // 5. 恢复后代（仅恢复 status='deleted' 的）
    repo::batch_update_status_if(&conn, &to_restore, "normal", &now, "deleted")
        .map_err(|e| AppError::Internal(format!("恢复后代 Block 失败: {}", e)))?;

    // 6. 获取更新后的 version
    let new_version = repo::get_version(&conn, id)
        .map_err(|e| AppError::Internal(format!("查询版本失败: {}", e)))?;

    Ok(RestoreResult {
        id: id.to_string(),
        version: new_version,
        cascade_count,
    })
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
    if id == crate::db::ROOT_ID {
        return Err(AppError::BadRequest("全局根块不可移动".to_string()));
    }

    let conn = db.lock().unwrap();

    // 1. 查询当前 Block
    let current = repo::find_by_id(&conn, id)
        .map_err(|_| AppError::NotFound(format!("Block {} 不存在或已删除", id)))?;

    if req.version != current.version {
        return Err(AppError::VersionConflict(current.version));
    }

    // 2. 确定目标父块（未传则保持当前父块）
    let target_parent_id = req
        .target_parent_id
        .as_deref()
        .unwrap_or(&current.parent_id)
        .to_string();

    // 3. 循环引用检测 + 父块验证（仅当父块改变时）
    let parent_changed = target_parent_id != current.parent_id;
    if parent_changed {
        // 不能移动到自身
        if target_parent_id == id {
            return Err(AppError::BadRequest("不能将 Block 移动到自身下".to_string()));
        }

        // 循环引用检测：target_parent 不能是当前 block 的后代
        let is_descendant = repo::check_is_descendant(&conn, id, &target_parent_id)
            .unwrap_or(false);

        if is_descendant {
            return Err(AppError::CycleReference);
        }

        // 验证目标父块存在且未删除
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
    let new_position = calculate_move_position(
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
        req.version,
    )
    .map_err(|e| AppError::Internal(format!("移动 Block 失败: {}", e)))?;

    if rows == 0 {
        return Err(AppError::VersionConflict(current.version));
    }

    // 6. 查询并返回
    repo::find_by_id_raw(&conn, id)
        .map_err(|e| AppError::Internal(format!("查询移动后的 Block 失败: {}", e)))
}

// ─── 创建文档 ──────────────────────────────────────────────────

/// 创建文档
///
/// 创建一个 Document Block + 一个空 Paragraph 子块。
/// 根文档（无 parent_id）挂到全局根块 "/" 下。
///
/// 参考 03-api-rest.md §2 "创建文档"
pub fn create_document(
    db: &Db,
    title: String,
    parent_id: Option<String>,
    after_id: Option<String>,
) -> Result<Block, AppError> {
    let conn = db.lock().unwrap();

    let doc_id = generate_block_id();
    let now = now_iso();

    // 1. 确定 parent_id
    let parent_id_actual = match parent_id {
        Some(ref pid) => {
            // 子文档：验证父文档存在且是 Document 类型
            let parent = repo::find_by_id(&conn, pid)
                .map_err(|_| AppError::BadRequest(format!("父文档 {} 不存在", pid)))?;

            if !matches!(parent.block_type, BlockType::Document) {
                return Err(AppError::BadRequest(
                    "parent_id 必须指向文档类型的 Block".to_string(),
                ));
            }

            pid.clone()
        }
        None => {
            // 根文档：挂到全局根块 "/" 下
            crate::db::ROOT_ID.to_string()
        }
    };

    // 2. 计算 position（在同级兄弟文档中的位置）
    let position = calculate_insert_position(&conn, &parent_id_actual, after_id.as_deref())?;

    // 3. 创建 Document Block
    let mut properties = HashMap::new();
    properties.insert("title".to_string(), title.clone());
    let properties_json = serde_json::to_string(&properties).unwrap_or_default();
    let block_type_json = serde_json::to_string(&BlockType::Document).unwrap();

    repo::insert_block(&conn, &InsertBlockParams {
        id: doc_id.clone(),
        parent_id: parent_id_actual,
        position,
        block_type: block_type_json,
        content_type: "markdown".to_string(),
        content: title.into_bytes(),
        properties: properties_json,
        version: 1,
        status: "normal".to_string(),
        schema_version: 1,
        author: "system".to_string(),
        owner_id: None,
        encrypted: false,
        created: now.clone(),
        modified: now.clone(),
    })
    .map_err(|e| AppError::Internal(format!("创建文档失败: {}", e)))?;

    // 4. 创建空段落子块（段落是文档的子块，与文档不在同一 parent 下）
    let para_id = generate_block_id();
    let para_position = fractional::generate_first();
    let para_block_type = serde_json::to_string(&BlockType::Paragraph).unwrap();

    repo::insert_block(&conn, &InsertBlockParams {
        id: para_id,
        parent_id: doc_id.clone(),
        position: para_position,
        block_type: para_block_type,
        content_type: "markdown".to_string(),
        content: Vec::new(), // 空段落
        properties: "{}".to_string(),
        version: 1,
        status: "normal".to_string(),
        schema_version: 1,
        author: "system".to_string(),
        owner_id: None,
        encrypted: false,
        created: now.clone(),
        modified: now,
    })
    .map_err(|e| AppError::Internal(format!("创建默认段落失败: {}", e)))?;

    // 5. 查询并返回文档 Block
    repo::find_by_id_raw(&conn, &doc_id)
        .map_err(|e| AppError::Internal(format!("查询刚创建的文档失败: {}", e)))
}

// ─── 私有辅助函数 ──────────────────────────────────────────────

/// 计算新 Block 的插入 position
///
/// - 有 after_id → 插在 after_id 之后（如有后继兄弟则插入两者之间）
/// - 无 after_id → 追加到末尾
fn calculate_insert_position(
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
fn calculate_move_position(
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

// ─── 单元测试 ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::init_test_db;
    use crate::model::BlockType;

    // ── get_root ─────────────────────────────────────────

    #[test]
    fn get_root_returns_root_block() {
        let db = init_test_db();
        let root = get_root(&db).unwrap();
        assert_eq!(root.id, crate::db::ROOT_ID);
        assert_eq!(root.parent_id, crate::db::ROOT_ID);
    }

    // ── create_block ─────────────────────────────────────

    #[test]
    fn create_block_under_root() {
        let db = init_test_db();

        let req = CreateBlockReq {
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "Hello world".to_string(),
            properties: HashMap::new(),
            after_id: None,
        };

        let block = create_block(&db, req).unwrap();
        assert_eq!(block.parent_id, crate::db::ROOT_ID);
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
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "first".to_string(),
            properties: HashMap::new(),
            after_id: None,
        };
        let block1 = create_block(&db, req1).unwrap();

        // 在 block1 之后插入
        let req2 = CreateBlockReq {
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "second".to_string(),
            properties: HashMap::new(),
            after_id: Some(block1.id.clone()),
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
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "fetch me".to_string(),
            properties: HashMap::new(),
            after_id: None,
        };
        let created = create_block(&db, req).unwrap();

        let fetched = get_block(&db, &created.id).unwrap();
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.content, b"fetch me");
    }

    #[test]
    fn get_block_nonexistent_fails() {
        let db = init_test_db();

        let result = get_block(&db, "nonexistent0000000");
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

        let doc = create_document(
            &db,
            "My First Doc".to_string(),
            None,   // 根文档
            None,   // 无 after_id
        ).unwrap();

        assert_eq!(doc.block_type, BlockType::Document);
        assert_eq!(doc.parent_id, crate::db::ROOT_ID);
        assert_eq!(doc.content, b"My First Doc");
        assert_eq!(doc.properties.get("title").unwrap(), "My First Doc");

        // 验证同时创建了空段落子块
        let tree = get_document_tree(&db, &doc.id).unwrap();
        assert_eq!(tree.blocks.len(), 1); // 一个空段落
        assert_eq!(tree.blocks[0].block_type, BlockType::Paragraph);
    }

    #[test]
    fn create_sub_document() {
        let db = init_test_db();

        // 先创建根文档
        let parent = create_document(
            &db, "Parent Doc".to_string(), None, None,
        ).unwrap();

        // 创建子文档
        let child = create_document(
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

        let doc1 = create_document(&db, "Doc 1".to_string(), None, None).unwrap();
        let doc2 = create_document(&db, "Doc 2".to_string(), None, Some(doc1.id.clone())).unwrap();

        assert!(doc2.position > doc1.position);
    }

    // ── update_block ─────────────────────────────────────

    #[test]
    fn update_block_content_success() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "original".to_string(),
            properties: HashMap::new(),
            after_id: None,
        }).unwrap();

        let updated = update_block(&db, &created.id, UpdateBlockReq {
            content: Some("updated".to_string()),
            properties: None,
            properties_mode: "merge".to_string(),
            version: 1,
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
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "test".to_string(),
            properties: props,
            after_id: None,
        }).unwrap();

        let mut new_props = HashMap::new();
        new_props.insert("key2".to_string(), "val2".to_string());
        let updated = update_block(&db, &created.id, UpdateBlockReq {
            content: None,
            properties: Some(new_props),
            properties_mode: "merge".to_string(),
            version: 1,
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
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "test".to_string(),
            properties: props,
            after_id: None,
        }).unwrap();

        let mut new_props = HashMap::new();
        new_props.insert("key2".to_string(), "val2".to_string());
        let updated = update_block(&db, &created.id, UpdateBlockReq {
            content: None,
            properties: Some(new_props),
            properties_mode: "replace".to_string(),
            version: 1,
        }).unwrap();

        assert!(updated.properties.get("key1").is_none()); // 被替换掉
        assert_eq!(updated.properties.get("key2").unwrap(), "val2");
    }

    #[test]
    fn update_block_version_conflict() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "test".to_string(),
            properties: HashMap::new(),
            after_id: None,
        }).unwrap();

        let result = update_block(&db, &created.id, UpdateBlockReq {
            content: Some("should fail".to_string()),
            properties: None,
            properties_mode: "merge".to_string(),
            version: 999, // 错误版本号
        });

        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::VersionConflict(v) => assert_eq!(v, 1),
            other => panic!("预期 VersionConflict，实际: {:?}", other),
        }
    }

    // ── delete_block ─────────────────────────────────────

    #[test]
    fn delete_block_soft_deletes() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "delete me".to_string(),
            properties: HashMap::new(),
            after_id: None,
        }).unwrap();

        let result = delete_block(&db, &created.id, 1).unwrap();
        assert_eq!(result.id, created.id);
        assert_eq!(result.cascade_count, 0); // 叶子块无后代

        // get_block 不再能查到
        assert!(get_block(&db, &created.id).is_err());

        // 但 get_block_include_deleted 可以
        let deleted = get_block_include_deleted(&db, &created.id, true).unwrap();
        assert_eq!(deleted.status, crate::model::BlockStatus::Deleted);
    }

    #[test]
    fn delete_block_cascades_to_children() {
        let db = init_test_db();

        let doc = create_document(&db, "Cascade Doc".to_string(), None, None).unwrap();

        let result = delete_block(&db, &doc.id, 1).unwrap();
        assert!(result.cascade_count >= 1); // 至少包含默认段落

        // 文档和段落都不可查
        assert!(get_block(&db, &doc.id).is_err());
    }

    #[test]
    fn delete_root_block_forbidden() {
        let db = init_test_db();

        let result = delete_block(&db, crate::db::ROOT_ID, 1);
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::BadRequest(msg) => assert!(msg.contains("不可删除")),
            other => panic!("预期 BadRequest，实际: {:?}", other),
        }
    }

    #[test]
    fn delete_block_version_conflict() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "test".to_string(),
            properties: HashMap::new(),
            after_id: None,
        }).unwrap();

        let result = delete_block(&db, &created.id, 999);
        assert!(result.is_err());
    }

    // ── restore_block ────────────────────────────────────

    #[test]
    fn restore_deleted_block() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "restore me".to_string(),
            properties: HashMap::new(),
            after_id: None,
        }).unwrap();

        delete_block(&db, &created.id, 1).unwrap();

        // 获取删除后的 version
        let deleted = get_block_include_deleted(&db, &created.id, true).unwrap();
        let new_version = deleted.version;

        let result = restore_block(&db, &created.id, new_version).unwrap();
        assert_eq!(result.id, created.id);

        // 恢复后可以正常查询
        let restored = get_block(&db, &created.id).unwrap();
        assert_eq!(restored.status, crate::model::BlockStatus::Normal);
    }

    #[test]
    fn restore_normal_block_fails() {
        let db = init_test_db();

        let created = create_block(&db, CreateBlockReq {
            parent_id: crate::db::ROOT_ID.to_string(),
            block_type: BlockType::Paragraph,
            content_type: None,
            content: "normal".to_string(),
            properties: HashMap::new(),
            after_id: None,
        }).unwrap();

        let result = restore_block(&db, &created.id, 1);
        assert!(result.is_err());
    }

    // ── move_block ───────────────────────────────────────

    #[test]
    fn move_block_to_new_parent() {
        let db = init_test_db();

        let doc1 = create_document(&db, "Doc 1".to_string(), None, None).unwrap();
        let doc2 = create_document(&db, "Doc 2".to_string(), None, None).unwrap();

        // 将 doc2 移动到 doc1 下
        let moved = move_block(&db, &doc2.id, MoveBlockReq {
            target_parent_id: Some(doc1.id.clone()),
            before_id: None,
            after_id: None,
            version: 1,
        }).unwrap();

        assert_eq!(moved.parent_id, doc1.id);
        assert_eq!(moved.version, 2);
    }

    #[test]
    fn move_root_block_forbidden() {
        let db = init_test_db();

        let result = move_block(&db, crate::db::ROOT_ID, MoveBlockReq {
            target_parent_id: Some("any".to_string()),
            before_id: None,
            after_id: None,
            version: 1,
        });
        assert!(result.is_err());
    }

    #[test]
    fn move_block_cycle_detection() {
        let db = init_test_db();

        let doc = create_document(&db, "Doc".to_string(), None, None).unwrap();

        // 试图将 doc 移动到自身下
        let result = move_block(&db, &doc.id, MoveBlockReq {
            target_parent_id: Some(doc.id.clone()),
            before_id: None,
            after_id: None,
            version: 1,
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

        create_document(&db, "Doc 1".to_string(), None, None).unwrap();
        create_document(&db, "Doc 2".to_string(), None, None).unwrap();

        let docs = list_root_documents(&db).unwrap();
        assert!(docs.len() >= 2); // 可能还有其他非文档块
        let titles: Vec<&str> = docs.iter()
            .filter_map(|d| d.properties.get("title").map(|s| s.as_str()))
            .collect();
        assert!(titles.contains(&"Doc 1"));
        assert!(titles.contains(&"Doc 2"));
    }

    // ── get_document_tree ────────────────────────────────

    #[test]
    fn get_document_tree_nested() {
        let db = init_test_db();

        let doc = create_document(&db, "Tree Doc".to_string(), None, None).unwrap();
        let child = create_block(&db, CreateBlockReq {
            parent_id: doc.id.clone(),
            block_type: BlockType::Heading { level: 2 },
            content_type: None,
            content: "Section".to_string(),
            properties: HashMap::new(),
            after_id: None,
        }).unwrap();

        let tree = get_document_tree(&db, &doc.id).unwrap();
        assert_eq!(tree.root.id, doc.id);
        assert_eq!(tree.blocks.len(), 2); // 默认段落 + heading
        assert!(tree.blocks.iter().any(|b| b.id == child.id));
    }

    #[test]
    fn get_document_tree_nonexistent_fails() {
        let db = init_test_db();

        let result = get_document_tree(&db, "nonexistent0000000");
        assert!(result.is_err());
    }

    // ── get_children ─────────────────────────────────────

    #[test]
    fn get_children_with_pagination() {
        let db = init_test_db();

        let doc = create_document(&db, "Pagination Doc".to_string(), None, None).unwrap();

        // 创建 3 个额外子块（已有 1 个默认段落）
        for i in 0..3 {
            create_block(&db, CreateBlockReq {
                parent_id: doc.id.clone(),
                block_type: BlockType::Paragraph,
                content_type: None,
                content: format!("para {}", i),
                properties: HashMap::new(),
                after_id: None,
            }).unwrap();
        }

        // 限制每页 2 条（总共 4 个子块 = 1 默认段落 + 3 新增）
        let result = get_children(&db, &doc.id, 2, None).unwrap();
        assert_eq!(result.blocks.len(), 2);
        assert!(result.has_more);
        assert!(result.next_cursor.is_some());

        // 翻页：最后 2 条
        let page2 = get_children(&db, &doc.id, 2, result.next_cursor.as_deref()).unwrap();
        assert_eq!(page2.blocks.len(), 2);
        assert!(!page2.has_more); // 4 项 / 2 = 恰好 2 页
        assert!(page2.next_cursor.is_none());
    }

    #[test]
    fn get_children_nonexistent_parent_fails() {
        let db = init_test_db();

        let result = get_children(&db, "nonexistent0000000", 10, None);
        assert!(result.is_err());
    }
}
