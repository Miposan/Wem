//! Document 类型特化实现
//!
//! - BlockTypeOps trait 的 Document 变体（use_tree_move / 标题去重）
//! - 文档级编排操作（创建文档、获取文档内容/子文档列表、移动/导入/导出）

use rusqlite::Connection;

use std::collections::HashMap;

use crate::error::AppError;
use crate::block_system::model::Block;
use crate::block_system::model::BlockType;
use crate::block_system::model::event::BlockEvent;
use crate::block_system::model::oplog::{Action, BlockSnapshot, Change, ChangeType, Operation};
use crate::repo::block_repo as repo;
use crate::repo::block_repo::InsertBlockParams;
use crate::repo::Db;
use crate::block_system::service::position;
use crate::util::now_iso;

use super::traits::{BlockTypeOps, MoveContext, TreeMoveOps, ExportDepth};
use super::helpers::{self, run_in_transaction, resolve_target_parent};
use super::{event, oplog};
use crate::api::request::MoveDocumentTreeReq;
use crate::api::response::{BlockNode, DocumentChildrenResult, DocumentContentResult};

/// Document 类型行为实现
pub struct DocumentOps;

impl BlockTypeOps for DocumentOps {
    fn use_tree_move() -> bool {
        true
    }

    fn adjust_content_on_update(
        conn: &rusqlite::Connection,
        block: &Block,
        content: &mut Vec<u8>,
    ) -> Result<(), AppError> {
        if *content == block.content {
            return Ok(());
        }
        let new_title = String::from_utf8_lossy(content);
        let deduped = deduplicate_doc_name(conn, &block.parent_id, &new_title)?;
        if deduped != new_title {
            *content = deduped.into_bytes();
        }
        Ok(())
    }

    fn on_moved(
        _conn: &rusqlite::Connection,
        _ctx: &MoveContext<'_>,
    ) -> Result<(), AppError> {
        Ok(())
    }
}

// ─── 文档级编排操作 ──────────────────────────────────────────────

pub fn create_document(
    db: &Db,
    title: String,
    parent_id: Option<String>,
    after_id: Option<String>,
    editor_id: Option<String>,
) -> Result<Block, AppError> {
    let conn = crate::repo::lock_db(db);

    let result = helpers::run_in_transaction(&conn, || {
        let doc_id = crate::block_system::model::generate_block_id();
        let now = now_iso();

        let parent_id_actual = match parent_id {
            Some(ref pid) => {
                let parent = repo::find_by_id(&conn, pid)
                    .map_err(|_| AppError::BadRequest(format!("父文档 {} 不存在", pid)))?;
                if !matches!(parent.block_type, BlockType::Document) {
                    return Err(AppError::BadRequest(
                        "parent_id 必须指向文档类型的 Block".to_string(),
                    ));
                }
                pid.clone()
            }
            None => crate::block_system::model::ROOT_ID.to_string(),
        };

        let final_title = deduplicate_doc_name(&conn, &parent_id_actual, &title)?;

        let position =
            position::calculate_insert_position(&conn, &parent_id_actual, after_id.as_deref())?;

        let mut properties = HashMap::new();
        properties.insert("title".to_string(), final_title.clone());
        let properties_json = helpers::to_json(&properties);
        let block_type_json = helpers::to_json(&BlockType::Document);

        repo::insert_block(&conn, &InsertBlockParams {
            id: doc_id.clone(),
            parent_id: parent_id_actual,
            document_id: doc_id.clone(),
            position,
            block_type: block_type_json,
            content: final_title.into_bytes(),
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

        let para_id = crate::block_system::model::generate_block_id();
        let para_position = position::generate_first();
        let para_block_type = helpers::to_json(&BlockType::Paragraph);

        repo::insert_block(&conn, &InsertBlockParams {
            id: para_id,
            parent_id: doc_id.clone(),
            document_id: doc_id.clone(),
            position: para_position,
            block_type: para_block_type,
            content: Vec::new(),
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

        repo::find_by_id_raw(&conn, &doc_id)
            .map_err(|e| AppError::Internal(format!("查询刚创建的文档失败: {}", e)))
    })?;

    event::EventBus::global().emit(BlockEvent::BlockCreated {
        document_id: result.document_id.clone(),
        editor_id,
        block: result.clone(),
    });

    Ok(result)
}

pub fn get_document_content(db: &Db, doc_id: &str) -> Result<DocumentContentResult, AppError> {
    let conn = crate::repo::lock_db(db);

    let document = repo::find_by_id(&conn, doc_id)
        .map_err(|_| AppError::NotFound(format!("文档 {} 不存在", doc_id)))?;

    let all_blocks = repo::find_descendants(&conn, doc_id)
        .map_err(|e| AppError::Internal(format!("查询文档内容失败: {}", e)))?;

    let blocks = build_block_tree(doc_id, &all_blocks);

    Ok(DocumentContentResult {
        document,
        blocks,
        has_more: false,
    })
}

pub fn get_document_children(db: &Db, doc_id: &str) -> Result<DocumentChildrenResult, AppError> {
    let conn = crate::repo::lock_db(db);

    let _doc = repo::find_by_id(&conn, doc_id)
        .map_err(|_| AppError::NotFound(format!("文档 {} 不存在", doc_id)))?;

    let all_children = repo::find_children_paginated(&conn, doc_id, None, 10000)
        .map_err(|e| AppError::Internal(format!("查询子文档失败: {}", e)))?;

    let children: Vec<Block> = all_children
        .into_iter()
        .filter(|b| matches!(b.block_type, BlockType::Document))
        .collect();

    Ok(DocumentChildrenResult { children })
}

pub fn list_root_documents(db: &Db) -> Result<Vec<Block>, AppError> {
    let conn = crate::repo::lock_db(db);

    repo::find_root_documents(&conn)
        .map_err(|e| AppError::Internal(format!("查询根文档失败: {}", e)))
}

// ─── Document 子树移动 ──────────────────────────────────────────

struct DocumentTreeMove;

impl TreeMoveOps for DocumentTreeMove {
    fn validate_type(current: &Block) -> Result<(), AppError> {
        if !matches!(current.block_type, BlockType::Document) {
            return Err(AppError::BadRequest(
                "move_document_tree 只能移动 Document 类型".to_string(),
            ));
        }
        Ok(())
    }

    fn resolve_target_parent(
        conn: &rusqlite::Connection,
        _current_parent_id: &str,
        target_parent_id: Option<&str>,
        before_id: &Option<String>,
        after_id: &Option<String>,
    ) -> Result<String, AppError> {
        match target_parent_id {
            Some(pid) => Ok(pid.to_string()),
            None => resolve_target_parent(conn, before_id, after_id, _current_parent_id),
        }
    }

    fn pre_move(
        conn: &rusqlite::Connection,
        current: &Block,
        target_parent_id: &str,
    ) -> Result<Option<Block>, AppError> {
        if target_parent_id != crate::block_system::model::ROOT_ID {
            let target_parent = repo::find_by_id(conn, target_parent_id)
                .map_err(|_| AppError::NotFound(format!(
                    "目标父块 {} 不存在或已删除", target_parent_id
                )))?;
            if !matches!(target_parent.block_type, BlockType::Document) {
                return Err(AppError::BadRequest(
                    "Document 只能移动到根目录或另一个 Document 下".to_string(),
                ));
            }
        }

        if target_parent_id != current.parent_id {
            let title = String::from_utf8_lossy(&current.content);
            let deduped = deduplicate_doc_name(conn, target_parent_id, &title)?;
            if deduped != title {
                repo::update_block_fields(
                    conn,
                    &current.id,
                    &deduped.into_bytes(),
                    &helpers::to_json(&current.properties),
                    None,
                    &now_iso(),
                    Some(current.version),
                )?;
            }
        }

        Ok(None)
    }

    fn execute_move(
        conn: &rusqlite::Connection,
        id: &str,
        target_parent_id: &str,
        new_position: &str,
        current: &Block,
    ) -> Result<u64, AppError> {
        let now = now_iso();
        repo::update_parent_position(conn, id, target_parent_id, new_position, &now, Some(current.version))
            .map_err(|e| AppError::Internal(format!("移动 Document 失败: {}", e)))
    }

    fn post_move(
        _conn: &rusqlite::Connection,
        _current: &Block,
        _target_parent_id: &str,
        _new_position: &str,
    ) -> Result<(), AppError> {
        Ok(())
    }

    fn build_changes(
        _conn: &rusqlite::Connection,
        op: &Operation,
        id: &str,
        current: &Block,
        after: &Block,
    ) -> Result<Vec<Change>, AppError> {
        Ok(vec![oplog::block_change_pair(
            &op.id, id, ChangeType::Moved, current, after,
        )])
    }
}

pub fn move_document_tree(db: &Db, req: MoveDocumentTreeReq) -> Result<Block, AppError> {
    super::block::move_tree::<DocumentTreeMove>(
        db, &req.id, req.editor_id, req.target_parent_id, req.before_id, req.after_id,
    )
}

// ─── 导入/导出 ──────────────────────────────────────────────────

pub fn import_text(db: &Db, req: crate::api::request::ImportTextReq) -> Result<crate::api::response::ImportResult, AppError> {
    let p = crate::block_system::parser::get_parser(&req.format)?;
    let parse_result = p.parse(&req.content, &crate::block_system::parser::types::ParseOptions::default())?;

    let parent_id = req
        .parent_id
        .clone()
        .unwrap_or_else(|| crate::block_system::model::ROOT_ID.to_string());

    let conn = crate::repo::lock_db(db);

    let _parent = repo::find_by_id(&conn, &parent_id)
        .map_err(|_| AppError::BadRequest(format!("父块 {} 不存在或已删除", parent_id)))?;

    let position =
        position::calculate_insert_position(&conn, &parent_id, req.after_id.as_deref())?;

    let mut root = parse_result.root;
    root.parent_id = parent_id;
    root.position = position;

    if let Some(title) = &req.title {
        root.properties.insert("title".to_string(), title.clone());
        root.content = title.clone().into_bytes();
    }

    run_in_transaction(&conn, || {
        insert_block_from_model(&conn, &root)?;
        for child in &parse_result.children {
            insert_block_from_model(&conn, child)?;
        }

        // 记录导入操作到 oplog（支持 undo）
        let document_id = root.document_id.clone();
        let op = oplog::new_operation(Action::Import, &document_id, req.editor_id.clone());
        let mut changes = Vec::with_capacity(parse_result.blocks_created);
        changes.push(oplog::new_change(
            &op.id, &root.id, ChangeType::Created,
            None,
            Some(BlockSnapshot::from_block(&root)),
        ));
        for child in &parse_result.children {
            changes.push(oplog::new_change(
                &op.id, &child.id, ChangeType::Created,
                None,
                Some(BlockSnapshot::from_block(child)),
            ));
        }
        oplog::record_operation(&conn, &op, &changes)?;

        Ok(())
    })?;

    let result = crate::api::response::ImportResult {
        root: root.clone(),
        blocks_imported: parse_result.blocks_created,
        warnings: parse_result.warnings,
    };

    event::EventBus::global().emit(BlockEvent::BlockCreated {
        document_id: result.root.id.clone(),
        editor_id: req.editor_id.clone(),
        block: result.root.clone(),
    });

    Ok(result)
}

fn insert_block_from_model(conn: &rusqlite::Connection, block: &Block) -> Result<(), AppError> {
    repo::insert_block(
        conn,
        &InsertBlockParams {
            id: block.id.clone(),
            parent_id: block.parent_id.clone(),
            document_id: block.document_id.clone(),
            position: block.position.clone(),
            block_type: helpers::to_json(&block.block_type),
            content: block.content.clone(),
            properties: helpers::to_json(&block.properties),
            version: block.version,
            status: block.status.as_str().to_string(),
            schema_version: block.schema_version,
            author: block.author.clone(),
            owner_id: block.owner_id.clone(),
            encrypted: block.encrypted,
            created: block.created.clone(),
            modified: block.modified.clone(),
        },
    )
    .map_err(|e| AppError::Internal(format!("插入 Block 失败: {}", e)))
}

pub fn export_text(db: &Db, doc_id: &str, format: &str) -> Result<crate::api::response::ExportResult, AppError> {
    super::block::export_block(db, doc_id, format, ExportDepth::Descendants)
}

// ─── 内部辅助 ──────────────────────────────────────────────────

fn build_block_tree(parent_id: &str, blocks: &[Block]) -> Vec<BlockNode> {
    let mut children: Vec<BlockNode> = blocks
        .iter()
        .filter(|b| b.parent_id == parent_id)
        .map(|b| BlockNode {
            block: b.clone(),
            children: build_block_tree(&b.id, blocks),
        })
        .collect();

    children.sort_by(|a, b| a.block.position.cmp(&b.block.position));
    children
}

// ─── 业务策略（从 repo 层搬来） ──────────────────────────────────

/// 同层文档名自动去重
///
/// 如果同层已有同名文档，自动追加序号 `(1)`、`(2)` ...
fn deduplicate_doc_name(
    conn: &Connection,
    parent_id: &str,
    title: &str,
) -> Result<String, AppError> {
    let existing = repo::find_sibling_doc_names(conn, parent_id)
        .map_err(|e| AppError::Internal(format!("查询同层文档名失败: {}", e)))?;
    if !existing.contains(&title.to_string()) {
        return Ok(title.to_string());
    }
    let prefix = format!("{} (", title);
    let mut max_seq: u32 = 0;
    for name in &existing {
        if name == title {
            continue;
        }
        if name.starts_with(&prefix) && name.ends_with(')') {
            let seq_str = &name[prefix.len()..name.len() - 1];
            if let Ok(n) = seq_str.parse::<u32>() {
                max_seq = max_seq.max(n);
            }
        }
    }
    let mut seq = 1;
    loop {
        let candidate = format!("{} ({})", title, seq);
        if !existing.contains(&candidate) {
            return Ok(candidate);
        }
        seq += 1;
    }
}

// ─── CLI 辅助 ──────────────────────────────────────────────────

/// 按父文档和标题查找文档（供 CLI 按名称引用文档使用）
pub fn find_doc_by_name(
    db: &Db,
    parent_id: &str,
    name: &str,
) -> Result<Option<Block>, AppError> {
    let conn = crate::repo::lock_db(db);
    repo::find_doc_by_parent_and_title(&conn, parent_id, name)
        .map_err(|e| AppError::Internal(format!("按名称查找文档失败: {}", e)))
}
