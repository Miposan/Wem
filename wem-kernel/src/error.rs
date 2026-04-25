//! 统一错误处理与 API 响应格式
//!
//! 所有 API 返回统一的 JSON 格式：{ code, msg, data }
//! - 成功：code=0, data=业务数据
//! - 失败：code=错误码, msg=错误描述, data=null（冲突类错误 data 含详情）

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

// ─── 统一响应格式 ───────────────────────────────────────────────

/// API 统一响应结构体
///
/// ```json
/// {"code": 0, "msg": "ok", "data": {"id": "xxx", ...}}
/// ```
///
/// - `code`: 0 表示成功，非零表示错误（见错误码常量）
/// - `msg`:  人类可读的描述
/// - `data`: 业务数据，失败时为 null
#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub code: i32,
    pub msg: String,
    /// 序列化时如果为 None 则省略 data 字段（让 JSON 更干净）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T: Serialize> ApiResponse<T> {
    /// 构造成功响应
    ///
    /// ```ignore
    /// ApiResponse::ok(Some(block))  // → {"code":0, "msg":"ok", "data":{...}}
    /// ApiResponse::ok(None::<()>)   // → {"code":0, "msg":"ok"}
    /// ```
    pub fn ok(data: Option<T>) -> Self {
        Self {
            code: 0,
            msg: "ok".to_string(),
            data,
        }
    }
}

// ─── 错误码常量 ─────────────────────────────────────────────────
// 参考 03-api-rest.md §1 错误码表

/// 请求参数错误（缺少必填字段、格式不对等）
pub const CODE_BAD_REQUEST: i32 = 40001;
/// Block 不存在
pub const CODE_NOT_FOUND: i32 = 40401;
/// 版本冲突（乐观锁：客户端版本与当前版本不一致）
pub const CODE_VERSION_CONFLICT: i32 = 40901;
/// 循环引用（移动 Block 时目标父块是被移动块的后代）
pub const CODE_CYCLE_REFERENCE: i32 = 40902;
/// 内部错误（数据库异常等不可预期错误）
pub const CODE_INTERNAL: i32 = 50001;

// ─── 应用错误枚举 ───────────────────────────────────────────────

/// 所有业务错误的统一枚举
///
/// 每个变体对应一种错误场景，通过 IntoResponse 自动转为 HTTP 响应：
/// - BadRequest      → 400 + code:40001
/// - NotFound        → 404 + code:40401
/// - CycleReference  → 409 + code:40902
/// - Internal        → 500 + code:50001
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// 请求参数错误
    #[error("{0}")]
    BadRequest(String),

    /// Block 不存在，携带 Block ID 方便排查
    #[error("Block not found: {0}")]
    NotFound(String),

    /// 版本冲突（乐观锁失败：当前版本与期望版本不一致）
    #[error("Version conflict: {0}")]
    VersionConflict(String),

    /// 移动操作导致循环引用
    #[error("Cycle reference detected")]
    CycleReference,

    /// 内部错误（数据库异常等）
    #[error("{0}")]
    Internal(String),
}

// ─── 错误 → HTTP 响应的转换 ─────────────────────────────────────

/// 让 AppError 能被 Axum 直接作为响应返回
///
/// 核心逻辑：匹配错误类型 → 确定 HTTP 状态码 + 错误码 + 消息 → 组装 JSON
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // 根据错误类型决定 HTTP 状态码和错误码
        // data 统一用 Option<serde_json::Value>，None 表示无额外数据
        let (status, code, msg, data): (_, _, _, Option<serde_json::Value>) = match &self {
            AppError::BadRequest(m) => (
                StatusCode::BAD_REQUEST,
                CODE_BAD_REQUEST,
                m.clone(),
                None,
            ),
            AppError::NotFound(id) => (
                StatusCode::NOT_FOUND,
                CODE_NOT_FOUND,
                format!("Block not found: {}", id),
                None,
            ),
            AppError::VersionConflict(detail) => (
                StatusCode::CONFLICT,
                CODE_VERSION_CONFLICT,
                format!("Version conflict: {}", detail),
                None,
            ),
            AppError::CycleReference => (
                StatusCode::CONFLICT,
                CODE_CYCLE_REFERENCE,
                self.to_string(),
                None,
            ),
            AppError::Internal(m) => {
                tracing::error!("Internal error: {}", m);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    CODE_INTERNAL,
                    "内部错误".to_string(),
                    None,
                )
            }
        };

        // 组装成 { code, msg, data } 的 JSON
        let body = serde_json::json!({
            "code": code,
            "msg": msg,
            "data": data,
        });

        // 返回 (HTTP状态码, JSON响应体)
        (status, axum::Json(body)).into_response()
    }
}

// ─── 第三方错误自动转换 ─────────────────────────────────────────

/// rusqlite 的错误自动转为 Internal 错误
/// 这样在数据库操作时可以用 ? 操作符，不用手动 match
impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        AppError::Internal(e.to_string())
    }
}
