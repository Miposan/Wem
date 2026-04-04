//! 查询参数 DTO
//!
//! GET 请求的 URL 查询参数类型（`?key=value`）。
//! Axum 通过 `Query<T>` 自动反序列化。

use serde::Deserialize;

/// 删除/恢复请求中的 version 参数
///
/// `?version=N`
#[derive(Debug, Deserialize)]
pub struct VersionQuery {
    pub version: u64,
}

/// 获取 Block 时的查询参数
///
/// `?include_deleted=true`
#[derive(Debug, Deserialize)]
pub struct GetBlockQuery {
    /// 是否包含已删除的 Block
    #[serde(default)]
    pub include_deleted: bool,
}

/// 获取子块列表的查询参数
///
/// `?limit=50&cursor=xxx`
#[derive(Debug, Deserialize)]
pub struct ChildrenQuery {
    /// 每页数量（默认 50，最大 500）
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// 游标（上一页最后一条的 position）
    pub cursor: Option<String>,
}

fn default_limit() -> u32 {
    50
}
