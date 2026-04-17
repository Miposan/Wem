//! 查询参数 DTO
//!
//! GET 请求的 URL 查询参数类型（`?key=value`）。
//! Axum 通过 `Query<T>` 自动反序列化。

use serde::Deserialize;

/// 获取 Block 时的查询参数
///
/// `?include_deleted=true`
#[derive(Debug, Deserialize)]
pub struct GetBlockQuery {
    /// 是否包含已删除的 Block
    #[serde(default)]
    pub include_deleted: bool,
}

// 文档列表和子块列表不需要分页参数，直接返回全部数据

/// 导出查询参数
///
/// `GET /api/v1/documents/{id}/export?format=markdown`
#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    /// 目标格式（默认 "markdown"）
    #[serde(default = "default_export_format")]
    pub format: String,
}

fn default_export_format() -> String {
    "markdown".to_string()
}

// ─── Oplog 查询参数 ────────────────────────────────────────────

/// 历史/版本查询参数
///
/// `GET /api/v1/blocks/{id}/history?limit=50`
#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    /// 返回条数（默认 50，最大 500）
    #[serde(default = "default_history_limit")]
    pub limit: u32,
}

fn default_history_limit() -> u32 {
    50
}

/// 回滚请求体
///
/// `POST /api/v1/blocks/{id}/rollback`
#[derive(Debug, Deserialize)]
pub struct RollbackReq {
    /// 回滚到的目标版本号
    pub target_version: u64,
}
