//! 文档级业务逻辑
//!
//! 文档 = Document Block + 内容 Block 的编排。
//! 本模块负责文档粒度的操作（创建文档、获取文档内容、列出子文档等），
//! 底层 Block 原子操作由 `service::block` 提供。

use std::collections::HashMap;

use crate::repo::block_repo as repo;
use crate::repo::block_repo::InsertBlockParams;
use crate::repo::Db;
use crate::error::AppError;
use crate::model::{generate_block_id, Block, BlockType};
use crate::service::block::now_iso;
use crate::service::position;
use crate::util::fractional;

use crate::api::response::{BlockNode, DocumentChildrenResult, DocumentContentResult};

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
    let conn = crate::repo::lock_db(db);

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
            crate::model::ROOT_ID.to_string()
        }
    };

    // 2. 计算 position（在同级兄弟文档中的位置）
    let position =
        position::calculate_insert_position(&conn, &parent_id_actual, after_id.as_deref())?;

    // 3. 创建 Document Block
    let mut properties = HashMap::new();
    properties.insert("title".to_string(), title.clone());
    let properties_json = serde_json::to_string(&properties).unwrap_or_default();
    let block_type_json = serde_json::to_string(&BlockType::Document).unwrap();

    repo::insert_block(&conn, &InsertBlockParams {
        id: doc_id.clone(),
        parent_id: parent_id_actual,
        document_id: doc_id.clone(), // 文档块的 document_id 指向自身
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
        document_id: doc_id.clone(), // 内容块指向所属文档
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

// ─── 查询文档内容 ──────────────────────────────────────────────

/// 获取文档内容（编辑器用）
///
/// 返回文档块 + 嵌套的内容块树（paragraph、heading、codeBlock 等），
/// 用于编辑器直接递归渲染。子 document 类型的块会被排除。
///
pub fn get_document_content(db: &Db, doc_id: &str) -> Result<DocumentContentResult, AppError> {
    let conn = crate::repo::lock_db(db);

    // 查询文档块本身
    let document = repo::find_by_id(&conn, doc_id)
        .map_err(|_| AppError::NotFound(format!("文档 {} 不存在", doc_id)))?;

    // 递归 CTE 查询所有未删除的后代（不含文档块自身）
    let all_blocks = repo::find_descendants(&conn, doc_id)
        .map_err(|e| AppError::Internal(format!("查询文档内容失败: {}", e)))?;

    // 过滤：只保留非 document 类型的内容块
    let content_blocks: Vec<Block> = all_blocks
        .into_iter()
        .filter(|b| !matches!(b.block_type, BlockType::Document))
        .collect();

    // 构建嵌套树
    let blocks = build_block_tree(doc_id, &content_blocks);

    Ok(DocumentContentResult {
        document,
        blocks,
        has_more: false, // MVP 阶段不截断
    })
}

/// 获取文档的直系子文档（侧边栏导航用）
///
/// 只返回该文档下的直接子 document 块（一层），不含内容块。
/// 用户展开某个子文档时，再请求该子文档的 /children 获取下一层。
pub fn get_document_children(db: &Db, doc_id: &str) -> Result<DocumentChildrenResult, AppError> {
    let conn = crate::repo::lock_db(db);

    // 验证文档存在
    let _doc = repo::find_by_id(&conn, doc_id)
        .map_err(|_| AppError::NotFound(format!("文档 {} 不存在", doc_id)))?;

    // 查询直系子块，过滤出 document 类型
    let all_children = repo::find_children_paginated(&conn, doc_id, None, 10000)
        .map_err(|e| AppError::Internal(format!("查询子文档失败: {}", e)))?;

    let children: Vec<Block> = all_children
        .into_iter()
        .filter(|b| matches!(b.block_type, BlockType::Document))
        .collect();

    Ok(DocumentChildrenResult { children })
}

/// 列出所有根文档
///
/// 根文档 = 全局根块 "/" 的直接子 document 块。
/// 按 position 排序，直接返回全部（不分页）。
pub fn list_root_documents(db: &Db) -> Result<Vec<Block>, AppError> {
    let conn = crate::repo::lock_db(db);

    let blocks = repo::find_root_documents(&conn)
        .map_err(|e| AppError::Internal(format!("查询根文档失败: {}", e)))?;

    // 只保留 document 类型
    let docs: Vec<Block> = blocks
        .into_iter()
        .filter(|b| matches!(b.block_type, BlockType::Document))
        .collect();

    Ok(docs)
}

// ─── 私有辅助函数 ──────────────────────────────────────────────

/// 将扁平 Block 列表构建为嵌套树
///
/// `parent_id` 为根节点的 ID（通常是文档 ID），
/// 所有 `block.parent_id == parent_id` 的块成为顶层节点，然后递归构建子节点。
fn build_block_tree(parent_id: &str, blocks: &[Block]) -> Vec<BlockNode> {
    // 找出所有直接子块（已按 position 排序）
    let mut children: Vec<BlockNode> = blocks
        .iter()
        .filter(|b| b.parent_id == parent_id)
        .map(|b| BlockNode {
            block: b.clone(),
            children: build_block_tree(&b.id, blocks),
        })
        .collect();

    // 按 position 排序（确保顺序正确）
    children.sort_by(|a, b| a.block.position.cmp(&b.block.position));
    children
}
