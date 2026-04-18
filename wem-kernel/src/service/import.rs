//! 文本导入服务
//!
//! 将 Markdown 等格式的文本解析为 Block 树并插入数据库。
//!
//! **流程**：
//! 1. 调用 parser 将文本解析为 Block 树（Document 根 + 扁平 children 列表）
//! 2. 确定 target_parent（默认全局根块 "/"）
//! 3. 通过 Fractional Index 计算 root Document 的 position
//! 4. 批量 INSERT 所有 Block
//!
//! 参考：`Note/15-markdown-parser.md`

use crate::api::request::ImportTextReq;
use crate::api::response::ImportResult;
use crate::repo::block_repo as repo;
use crate::repo::block_repo::InsertBlockParams;
use crate::repo::Db;
use crate::error::AppError;
use crate::model::Block;
use crate::parser;
use crate::parser::types::ParseOptions;

// ─── 导入 ─────────────────────────────────────────────────────

/// 导入文本为 Block 树
///
/// 将 Markdown 等格式文本解析后，以新 Document 的形式插入到指定父块下。
/// 所有解析出的 Block（含嵌套的标题、段落、列表等）自动批量写入数据库。
///
/// **参数**：
/// - `req.format`：源格式（`"markdown"` / `"md"`）
/// - `req.content`：文本内容
/// - `req.parent_id`：目标父块（默认全局根块）
/// - `req.after_id`：位置提示（默认追加末尾）
/// - `req.title`：覆盖文档标题（默认从内容推断）
///
/// 参考 03-api-rest.md §3 "导入文本"
pub fn import_text(db: &Db, req: ImportTextReq) -> Result<ImportResult, AppError> {
    // 1. 解析文本 → Block 树
    let p = parser::get_parser(&req.format)?;
    let parse_result = p.parse(&req.content, &ParseOptions::default())?;

    // 2. 确定父块（默认全局根块）
    let parent_id = req
        .parent_id
        .unwrap_or_else(|| crate::model::ROOT_ID.to_string());

    let conn = crate::repo::lock_db(db);

    // 验证父块存在且未删除
    let _parent = repo::find_by_id(&conn, &parent_id)
        .map_err(|_| AppError::BadRequest(format!("父块 {} 不存在或已删除", parent_id)))?;

    // 3. 计算 position（插入目标父块的末尾，或 after_id 之后）
    let position =
        super::position::calculate_insert_position(&conn, &parent_id, req.after_id.as_deref())?;

    // 4. 调整根 Block 的 parent_id / position
    let mut root = parse_result.root;
    root.parent_id = parent_id;
    root.position = position;

    // 覆盖标题（如果请求指定）
    if let Some(title) = &req.title {
        root.properties
            .insert("title".to_string(), title.clone());
        root.content = title.clone().into_bytes();
    }

    // 5. 事务包裹批量插入（失败时整体回滚，不会半导入）
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let insert_result = (|| -> Result<(), AppError> {
        insert_block_from_model(&conn, &root)?;
        for child in &parse_result.children {
            insert_block_from_model(&conn, child)?;
        }
        Ok(())
    })();
    if let Err(e) = insert_result {
        conn.execute_batch("ROLLBACK").ok();
        return Err(e);
    }
    conn.execute_batch("COMMIT")?;

    // 6. 返回
    Ok(ImportResult {
        root,
        blocks_imported: parse_result.blocks_created,
        warnings: parse_result.warnings,
    })
}

// ─── 私有辅助 ─────────────────────────────────────────────────

/// 将内存中的 Block 模型转为 [`InsertBlockParams`] 并插入数据库
fn insert_block_from_model(conn: &rusqlite::Connection, block: &Block) -> Result<(), AppError> {
    repo::insert_block(
        conn,
        &InsertBlockParams {
            id: block.id.clone(),
            parent_id: block.parent_id.clone(),
            document_id: block.document_id.clone(),
            position: block.position.clone(),
            block_type: serde_json::to_string(&block.block_type).unwrap_or_default(),
            content_type: block.content_type.as_str().to_string(),
            content: block.content.clone(),
            properties: serde_json::to_string(&block.properties).unwrap_or_default(),
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

// ═══════════════════════════════════════════════════════════════
//  测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::tests::init_test_db;
    use crate::model::ROOT_ID;
    use crate::model::BlockType;
    use crate::service::content;
    use crate::service::document;

    /// 辅助：导入 Markdown 文本
    fn import_md(db: &Db, content: &str) -> Result<ImportResult, AppError> {
        import_text(db, ImportTextReq {
            operation_id: None,
            format: "markdown".to_string(),
            content: content.to_string(),
            parent_id: None,
            after_id: None,
            title: None,
        })
    }

    // ── 基础导入 ────────────────────────────────────────────

    #[test]
    fn import_simple_paragraph() {
        let db = init_test_db();
        let result = import_md(&db, "Hello world").unwrap();

        // root 是 Document，parent_id 是全局根块
        assert_eq!(result.root.block_type, BlockType::Document);
        assert_eq!(result.root.parent_id, ROOT_ID);
        assert!(result.blocks_imported >= 2); // Document + Paragraph

        // 验证 DB 中确实存在
        let loaded = content::get_block(&db, &result.root.id, false).unwrap();
        assert_eq!(loaded.id, result.root.id);
        assert_eq!(loaded.parent_id, ROOT_ID);
    }

    #[test]
    fn import_heading_and_paragraph() {
        let db = init_test_db();
        let result = import_md(&db, "# My Title\n\nSome content here").unwrap();

        assert_eq!(result.root.properties.get("title").unwrap(), "My Title");
        assert!(result.blocks_imported >= 3); // Document + H1 + Paragraph
    }

    #[test]
    fn import_empty_content() {
        let db = init_test_db();
        let result = import_md(&db, "").unwrap();

        assert_eq!(result.root.block_type, BlockType::Document);
        assert!(result.blocks_imported >= 2); // Document + empty Paragraph
    }

    #[test]
    fn import_with_title_override() {
        let db = init_test_db();
        let req = ImportTextReq {
            operation_id: None,
            format: "markdown".to_string(),
            content: "# Original\n\nContent".to_string(),
            parent_id: None,
            after_id: None,
            title: Some("Overridden Title".to_string()),
        };
        let result = import_text(&db, req).unwrap();

        assert_eq!(result.root.properties.get("title").unwrap(), "Overridden Title");
    }

    #[test]
    fn import_to_specific_parent() {
        let db = init_test_db();

        // 先创建一个 Document 作为父块
        let parent = document::create_document(&db, "Parent Doc".to_string(), Some(ROOT_ID.to_string()), None).unwrap();

        let req = ImportTextReq {
            operation_id: None,
            format: "markdown".to_string(),
            content: "# Child\n\nChild content".to_string(),
            parent_id: Some(parent.id.clone()),
            after_id: None,
            title: None,
        };
        let result = import_text(&db, req).unwrap();

        assert_eq!(result.root.parent_id, parent.id);
    }

    #[test]
    fn import_with_after_id() {
        let db = init_test_db();

        // 先创建一个 Document
        let doc1 = document::create_document(&db, "First".to_string(), Some(ROOT_ID.to_string()), None).unwrap();

        // 在 doc1 之后导入
        let req = ImportTextReq {
            operation_id: None,
            format: "markdown".to_string(),
            content: "# Second\n\nSecond content".to_string(),
            parent_id: None,
            after_id: Some(doc1.id.clone()),
            title: None,
        };
        let result = import_text(&db, req).unwrap();

        // 新文档应在 doc1 之后
        assert!(result.root.position > doc1.position);
    }

    // ── 错误场景 ────────────────────────────────────────────

    #[test]
    fn import_invalid_format() {
        let db = init_test_db();
        let req = ImportTextReq {
            operation_id: None,
            format: "pdf".to_string(),
            content: "some text".to_string(),
            parent_id: None,
            after_id: None,
            title: None,
        };
        let result = import_text(&db, req);
        assert!(result.is_err());
    }

    #[test]
    fn import_nonexistent_parent() {
        let db = init_test_db();
        let req = ImportTextReq {
            operation_id: None,
            format: "markdown".to_string(),
            content: "Hello".to_string(),
            parent_id: Some("nonexistent_id_12345".to_string()),
            after_id: None,
            title: None,
        };
        let result = import_text(&db, req);
        assert!(result.is_err());
    }

    // ── 复杂内容导入 ────────────────────────────────────────

    #[test]
    fn import_code_block() {
        let db = init_test_db();
        let md = "```rust\nfn main() {}\n```";
        let result = import_md(&db, md).unwrap();
        assert!(result.blocks_imported >= 2); // Document + CodeBlock
    }

    #[test]
    fn import_list() {
        let db = init_test_db();
        let md = "- item 1\n- item 2\n- item 3";
        let result = import_md(&db, md).unwrap();
        // Document + List + ListItem*3 + Paragraph*3 (inside items)
        assert!(result.blocks_imported >= 7);
    }

    #[test]
    fn import_blockquote() {
        let db = init_test_db();
        let md = "> This is a quote\n> Second line";
        let result = import_md(&db, md).unwrap();
        assert!(result.blocks_imported >= 3); // Document + Blockquote + Paragraph
    }

    #[test]
    fn import_multiple_documents() {
        let db = init_test_db();

        let r1 = import_md(&db, "# Doc 1").unwrap();
        let r2 = import_md(&db, "# Doc 2").unwrap();
        let r3 = import_md(&db, "# Doc 3").unwrap();

        // 每个文档应独立
        assert_ne!(r1.root.id, r2.root.id);
        assert_ne!(r2.root.id, r3.root.id);

        // 位置应递增
        assert!(r2.root.position > r1.root.position);
        assert!(r3.root.position > r2.root.position);
    }

    #[test]
    fn import_md_alias() {
        let db = init_test_db();
        let req = ImportTextReq {
            operation_id: None,
            format: "md".to_string(),
            content: "# Alias Test".to_string(),
            parent_id: None,
            after_id: None,
            title: None,
        };
        let result = import_text(&db, req);
        assert!(result.is_ok());
    }
}
