//! 文本导出服务
//!
//! 从数据库加载 Block 树，序列化为 Markdown 等文本格式。
//!
//! **流程**：
//! 1. 从数据库加载文档根 Block + 所有后代（递归 CTE）
//! 2. 构建 `children_map`（parent_id → 按 position 排序的子块列表）
//! 3. 调用 serializer 将 Block 树序列化为文本
//!
//! 参考：`Note/15-markdown-parser.md`

use std::collections::HashMap;

use crate::api::response::ExportResult;
use crate::db::repository as repo;
use crate::db::Db;
use crate::error::AppError;
use crate::model::Block;
use crate::parser;

// ─── 导出 ─────────────────────────────────────────────────────

/// 导出文档为文本
///
/// 加载指定文档及其所有后代 Block，序列化为 Markdown 等格式文本。
/// 非原生支持的 BlockType 会做降级处理（如 Embed → 链接），记录在 `lossy_types` 中。
///
/// **参数**：
/// - `doc_id`：文档根 Block ID
/// - `format`：目标格式（`"markdown"` / `"md"`）
///
/// 参考 03-api-rest.md §3 "导出文档"
pub fn export_text(db: &Db, doc_id: &str, format: &str) -> Result<ExportResult, AppError> {
    let conn = db.lock().unwrap();

    // 1. 加载文档根（必须存在且未删除）
    let root = repo::find_by_id(&conn, doc_id)
        .map_err(|_| AppError::NotFound(format!("文档 {} 不存在或已删除", doc_id)))?;

    // 2. 递归 CTE 加载所有后代（不含根节点自身）
    let descendants = repo::find_descendants(&conn, doc_id)
        .map_err(|e| AppError::Internal(format!("查询文档树失败: {}", e)))?;

    // 3. 构建 children_map（parent_id → 子块列表，按 position ASC 排序）
    let mut children_map: HashMap<String, Vec<Block>> = HashMap::new();
    for block in &descendants {
        children_map
            .entry(block.parent_id.clone())
            .or_default()
            .push(block.clone());
    }
    for children in children_map.values_mut() {
        children.sort_by(|a, b| a.position.cmp(&b.position));
    }

    // 4. 序列化
    let serializer = parser::get_serializer(format)?;
    let result = serializer.serialize(&root, &children_map)?;

    // 5. 返回
    Ok(ExportResult {
        content: result.content,
        filename: result.filename,
        blocks_exported: result.blocks_exported,
        lossy_types: result.lossy_types,
    })
}

// ═══════════════════════════════════════════════════════════════
//  测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::request::ImportTextReq;
    use crate::db::tests::init_test_db;
    use crate::db::ROOT_ID;
    use crate::service::block;
    use crate::service::import;

    /// 辅助：导入 Markdown 并返回 doc_id
    fn import_md(db: &Db, content: &str) -> String {
        let result = import::import_text(db, ImportTextReq {
            format: "markdown".to_string(),
            content: content.to_string(),
            parent_id: None,
            after_id: None,
            title: None,
        }).unwrap();
        result.root.id
    }

    // ── 基础导出 ────────────────────────────────────────────

    #[test]
    fn export_simple_document() {
        let db = init_test_db();
        let doc_id = import_md(&db, "# Hello\n\nWorld");

        let result = export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.content.contains("Hello"));
        assert!(result.content.contains("World"));
        assert!(result.blocks_exported >= 3); // Document + H1 + Paragraph
    }

    #[test]
    fn export_empty_document() {
        let db = init_test_db();
        let doc_id = import_md(&db, "");

        let result = export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.blocks_exported >= 1); // at least Document
    }

    #[test]
    fn export_code_block() {
        let db = init_test_db();
        let doc_id = import_md(&db, "```rust\nfn main() {}\n```");

        let result = export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.content.contains("```rust"));
        assert!(result.content.contains("fn main() {}"));
    }

    #[test]
    fn export_list() {
        let db = init_test_db();
        let doc_id = import_md(&db, "- item 1\n- item 2\n- item 3");

        let result = export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.content.contains("item 1"));
        assert!(result.content.contains("item 2"));
        assert!(result.content.contains("item 3"));
    }

    #[test]
    fn export_blockquote() {
        let db = init_test_db();
        let doc_id = import_md(&db, "> quoted text");

        let result = export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.content.contains("quoted text"));
    }

    #[test]
    fn export_filename() {
        let db = init_test_db();
        let doc_id = import_md(&db, "# My Notes\n\nContent");

        let result = export_text(&db, &doc_id, "markdown").unwrap();
        assert!(result.filename.is_some());
        let filename = result.filename.unwrap();
        assert!(filename.contains("My Notes"));
        assert!(filename.ends_with(".md"));
    }

    #[test]
    fn export_md_alias() {
        let db = init_test_db();
        let doc_id = import_md(&db, "# Alias\n\nTest");

        let result = export_text(&db, &doc_id, "md").unwrap();
        assert!(result.content.contains("Alias"));
    }

    // ── 错误场景 ────────────────────────────────────────────

    #[test]
    fn export_nonexistent_document() {
        let db = init_test_db();
        let result = export_text(&db, "nonexistent_id_12345", "markdown");
        assert!(result.is_err());
    }

    #[test]
    fn export_invalid_format() {
        let db = init_test_db();
        let doc_id = import_md(&db, "# Test");

        let result = export_text(&db, &doc_id, "pdf");
        assert!(result.is_err());
    }

    // ── 往返测试 ────────────────────────────────────────────

    #[test]
    fn roundtrip_heading_paragraph() {
        let db = init_test_db();
        let original = "# Title\n\nParagraph text";
        let doc_id = import_md(&db, original);

        let exported = export_text(&db, &doc_id, "markdown").unwrap();
        assert!(exported.content.contains("Title"));
        assert!(exported.content.contains("Paragraph text"));
    }

    #[test]
    fn roundtrip_complex_document() {
        let db = init_test_db();
        let original = "# Main\n\n## Section\n\nText\n\n```js\nconsole.log(1)\n```\n\n- a\n- b\n";
        let doc_id = import_md(&db, original);

        let exported = export_text(&db, &doc_id, "markdown").unwrap();
        assert!(exported.content.contains("Main"));
        assert!(exported.content.contains("Section"));
        assert!(exported.content.contains("console.log(1)"));
        assert!(exported.blocks_exported >= 8);
    }

    #[test]
    fn export_created_document() {
        let db = init_test_db();

        // 使用 block::create_document 创建
        let doc = block::create_document(&db, "Created Doc".to_string(), Some(ROOT_ID.to_string()), None).unwrap();

        let result = export_text(&db, &doc.id, "markdown").unwrap();
        assert!(result.content.contains("Created Doc"));
        assert_eq!(result.blocks_exported, 2); // Document + 空 Paragraph（create_document 自动创建）
    }
}
