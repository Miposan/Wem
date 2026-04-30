pub mod request;
pub mod response;

use serde::Serialize;

use crate::error::AppError;

// ─── 统一响应格式 ───────────────────────────────────────────────

#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub code: i32,
    pub msg: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: Option<T>) -> Self {
        Self {
            code: 0,
            msg: "ok".to_string(),
            data,
        }
    }
}

// ─── Handler 辅助函数 ──────────────────────────────────────────────

pub async fn blocking<F, T>(f: F) -> Result<axum::Json<ApiResponse<T>>, AppError>
where
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    T: Send + Serialize + 'static,
{
    let result = tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("任务执行失败: {}", e)))??;
    Ok(axum::Json(ApiResponse::ok(Some(result))))
}
