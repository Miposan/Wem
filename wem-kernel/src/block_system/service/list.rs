//! List / ListItem 类型特化实现
//!
//! 列表的结构约束：
//! - List 是纯容器块，content 始终为空
//! - List 的直接子块必须是 ListItem
//! - ListItem 必须挂在 List 下面
//! - List 变空后自动删除
//!
//! 类型转换语义：
//! - Paragraph → List：创建 List 容器 + ListItem 子块（content 迁移到 ListItem）
//! - List → Paragraph：将第一个 ListItem 的 content 赋给 List（已变 Paragraph），删除所有子块
//! - CodeBlock ↔ List：类似 Paragraph 语义

use crate::error::AppError;
use crate::block_system::model::{Block, BlockType};
use crate::repo::block_repo as repo;

use super::traits::{BlockTypeOps, MoveContext};
use super::helpers;

/// List 类型行为实现
pub struct ListOps;

impl BlockTypeOps for ListOps {
    /// List 块被移动后的后置处理
    ///
    /// List 的移动通常是整棵子树一起移动（List + 所有 ListItem），
    /// 不需要像 Heading 那样重建树结构。
    fn on_moved(
        conn: &rusqlite::Connection,
        ctx: &MoveContext<'_>,
    ) -> Result<(), AppError> {
        let _ = (conn, ctx);
        Ok(())
    }

    /// 类型转换后处理
    ///
    /// 任何类型 → List：
    ///   后端自动将原 content 迁移到一个新建的 ListItem 子块中。
    /// List → 其他类型：
    ///   后端自动将第一个 ListItem 的内容赋给转换后的块，并删除所有子块。
    fn on_type_changed(
        conn: &rusqlite::Connection,
        block_id: &str,
        old_block: &Block,
        new_type: &BlockType,
    ) -> Result<(), AppError> {
        match (&old_block.block_type, new_type) {
            // X → List：已更新后的 content 迁移到 ListItem 子块
            // 注意：on_type_changed 在 write_block_updates 之后调用，
            // 数据库中的 content 已是前端传来的新值（如空字符串）。
            // 使用 old_block.content 会拿到过时的旧值（如含 Markdown 标记 "* "），
            // 因此从数据库重新读取已写入的 content。
            (_, BlockType::List { ordered: _ }) => {
                let updated_block = repo::find_by_id_raw(conn, block_id)
                    .map_err(|e| AppError::Internal(format!("查询已更新的 List 块失败: {}", e)))?;
                let content = updated_block.content.clone();

                // 清空 List 自身的 content（List 是纯容器）
                let now = crate::util::now_iso();
                let properties_json = helpers::to_json(&updated_block.properties);
                let bt_json = helpers::to_json(new_type);
                repo::update_block_fields(
                    conn,
                    block_id,
                    b"",
                    &properties_json,
                    Some(&bt_json),
                    &now,
                    Some(updated_block.version),
                )
                .map_err(|e| AppError::Internal(format!("清空 List content 失败: {}", e)))?;

                // 始终创建 ListItem — List 至少需要一个 ListItem 子块
                // （Markdown 快捷键 * + Space 时 content 为空，但用户期望看到一个可编辑的列表项）
                {
                    let document_id = updated_block.document_id.clone();
                    let list_item_id = crate::block_system::model::generate_block_id();
                    let position = super::position::generate_first();

                    repo::insert_block(&conn, &repo::InsertBlockParams {
                        id: list_item_id,
                        parent_id: block_id.to_string(),
                        document_id,
                        position,
                        block_type: helpers::to_json(&BlockType::ListItem),
                        content,
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
                    .map_err(|e| AppError::Internal(format!("创建 ListItem 失败: {}", e)))?;
                }
                Ok(())
            }

            // List → X：提取第一个 ListItem 的 content，删除所有子块
            (BlockType::List { .. }, _) => {
                let children = repo::find_children(conn, block_id)
                    .map_err(|e| AppError::Internal(format!("查询 List 子块失败: {}", e)))?;

                // 取第一个 ListItem 的 content 作为转换后块的内容
                let extracted_content = children
                    .iter()
                    .find(|c| matches!(c.block_type, BlockType::ListItem))
                    .map(|c| c.content.clone())
                    .unwrap_or_default();

                // 更新块：写入提取的 content + 新类型（版本已在 update_block_inner 中 +1）
                // 注意：on_type_changed 在 write_block_updates 之后调用，版本已经更新
                let updated = repo::find_by_id_raw(conn, block_id)
                    .map_err(|e| AppError::Internal(format!("查询 List 块失败: {}", e)))?;

                if !extracted_content.is_empty() {
                    let now = crate::util::now_iso();
                    let properties_json = helpers::to_json(&updated.properties);
                    let bt_json = helpers::to_json(new_type);
                    repo::update_block_fields(
                        conn,
                        block_id,
                        &extracted_content,
                        &properties_json,
                        Some(&bt_json),
                        &now,
                        Some(updated.version),
                    )
                    .map_err(|e| AppError::Internal(format!("更新转换后块失败: {}", e)))?;
                }

                // 软删除所有子块
                let now = crate::util::now_iso();
                for child in &children {
                    repo::update_status(conn, &child.id, "deleted", &now)
                        .map_err(|e| AppError::Internal(format!("删除子块 {} 失败: {}", child.id, e)))?;
                }

                Ok(())
            }

            _ => Ok(()),
        }
    }
}

/// ListItem 类型行为实现
pub struct ListItemOps;

impl BlockTypeOps for ListItemOps {}

// ─── 父子类型约束校验 ─────────────────────────────────────────

/// 校验 block_type 与 parent_id 的兼容性
///
/// 规则：
/// - ListItem 的 parent 必须是 List 类型
/// - List 的直接子块类型必须是 ListItem（通过 create_block 路径保证）
///
/// 在 `create_block` 中调用，parent 已加载到内存。
pub(crate) fn validate_parent_child_constraint(
    parent: &Block,
    block_type: &BlockType,
) -> Result<(), AppError> {
    match block_type {
        BlockType::ListItem => {
            if !matches!(parent.block_type, BlockType::List { .. }) {
                return Err(AppError::BadRequest(format!(
                    "ListItem 必须挂在 List 类型块下，实际父块类型为 {:?}",
                    parent.block_type
                )));
            }
        }
        // 其他块类型暂无父子约束
        _ => {}
    }
    Ok(())
}

/// 校验 List 块下添加子块的合法性
///
/// List 的直接子块只允许是 ListItem。
pub(crate) fn validate_list_child_type(
    parent_type: &BlockType,
    child_type: &BlockType,
) -> Result<(), AppError> {
    if matches!(parent_type, BlockType::List { .. }) {
        if !matches!(child_type, BlockType::ListItem) {
            return Err(AppError::BadRequest(format!(
                "List 块的直接子块必须是 ListItem，实际为 {:?}",
                child_type
            )));
        }
    }
    Ok(())
}

// ─── List 自动清理 ─────────────────────────────────────────────

/// 检查 List 是否为空（无正常状态的子块），如果是则软删除
///
/// 在 ListItem 被删除后调用。
pub(crate) fn cleanup_empty_list(
    conn: &rusqlite::Connection,
    list_id: &str,
) -> Result<bool, AppError> {
    let children = repo::find_children(conn, list_id)
        .map_err(|e| AppError::Internal(format!("查询 List 子块失败: {}", e)))?;

    if children.is_empty() {
        let now = crate::util::now_iso();
        repo::update_status(conn, list_id, "deleted", &now)
            .map_err(|e| AppError::Internal(format!("删除空 List 失败: {}", e)))?;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_system::model::BlockType;

    #[test]
    fn list_ops_validate_on_create_ordered() {
        assert!(ListOps::validate_on_create(&BlockType::List { ordered: true }).is_ok());
    }

    #[test]
    fn list_ops_validate_on_create_unordered() {
        assert!(ListOps::validate_on_create(&BlockType::List { ordered: false }).is_ok());
    }

    #[test]
    fn list_item_ops_validate_on_create() {
        assert!(ListItemOps::validate_on_create(&BlockType::ListItem).is_ok());
    }

    #[test]
    fn validate_parent_child_list_item_under_list() {
        let parent = Block {
            id: "test-list".into(),
            parent_id: "root".into(),
            document_id: "doc1".into(),
            position: "a0".into(),
            block_type: BlockType::List { ordered: false },
            content: Vec::new(),
            properties: std::collections::HashMap::new(),
            version: 1,
            status: crate::block_system::model::BlockStatus::Normal,
            schema_version: 1,
            author: "system".into(),
            owner_id: None,
            encrypted: false,
            created: "2025-01-01T00:00:00Z".into(),
            modified: "2025-01-01T00:00:00Z".into(),
        };

        assert!(validate_parent_child_constraint(&parent, &BlockType::ListItem).is_ok());
    }

    #[test]
    fn validate_parent_child_list_item_under_paragraph_fails() {
        let parent = Block {
            id: "test-para".into(),
            parent_id: "root".into(),
            document_id: "doc1".into(),
            position: "a0".into(),
            block_type: BlockType::Paragraph,
            content: Vec::new(),
            properties: std::collections::HashMap::new(),
            version: 1,
            status: crate::block_system::model::BlockStatus::Normal,
            schema_version: 1,
            author: "system".into(),
            owner_id: None,
            encrypted: false,
            created: "2025-01-01T00:00:00Z".into(),
            modified: "2025-01-01T00:00:00Z".into(),
        };

        let result = validate_parent_child_constraint(&parent, &BlockType::ListItem);
        assert!(result.is_err());
    }

    #[test]
    fn validate_list_child_type_list_item_ok() {
        assert!(validate_list_child_type(
            &BlockType::List { ordered: true },
            &BlockType::ListItem,
        ).is_ok());
    }

    #[test]
    fn validate_list_child_type_paragraph_fails() {
        assert!(validate_list_child_type(
            &BlockType::List { ordered: true },
            &BlockType::Paragraph,
        ).is_err());
    }
}
