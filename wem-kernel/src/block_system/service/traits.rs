//! 类型特化接口定义
//!
//! BlockTypeOps — BlockType 行为钩子（创建校验、移动后处理、类型变更等）
//! TreeMoveOps — 子树移动的类型特化接口
//!
//! 只定义接口，不引用任何具体 impl（HeadingOps / DocumentOps 等）。

use crate::error::AppError;
use crate::block_system::model::{Block, BlockType};

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
/// 各 BlockType 变体可重写特定钩子，通用层通过分派函数调用，默认实现为空操作。
pub trait BlockTypeOps {
    fn use_tree_move() -> bool { false }
    fn use_flat_list_move() -> bool { false }

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
}

// ─── TreeMoveOps trait ───────────────────────────────────────────

/// 子树移动的类型特化钩子
///
/// 各类型各自实现此 trait，提供特有的移动逻辑。
pub(crate) trait TreeMoveOps {
    fn validate_type(current: &Block) -> Result<(), AppError>;

    fn resolve_target_parent(
        conn: &rusqlite::Connection,
        current_parent_id: &str,
        target_parent_id: Option<&str>,
        before_id: &Option<String>,
        after_id: &Option<String>,
    ) -> Result<String, AppError>;

    /// 返回 Ok(Some(block)) 可短路移动
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
        op: &crate::block_system::model::oplog::Operation,
        id: &str,
        current: &Block,
        after: &Block,
    ) -> Result<Vec<crate::block_system::model::oplog::Change>, AppError>;
}

// ─── ExportDepth ─────────────────────────────────────────────────

/// 导出深度控制
#[derive(Debug, Clone, PartialEq)]
pub enum ExportDepth {
    /// 仅直接子块
    Children,
    /// 所有后代（递归）
    Descendants,
}
