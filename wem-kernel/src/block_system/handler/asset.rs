//! 资源上传 HTTP 处理层
//!
//! - POST /api/v1/assets/upload — multipart 文件上传
//! - GET  /api/v1/assets/*path   — 静态文件服务

use axum::{
    extract::{DefaultBodyLimit, Multipart, Path as AxumPath, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};

use crate::block_system::service::asset;
use crate::dto::ApiResponse;
use crate::error::AppError;

#[derive(Clone)]
pub struct AssetState {
    pub data_root: String,
}

#[derive(serde::Serialize)]
struct UploadResponse {
    succ_map: std::collections::HashMap<String, String>,
}

/// POST /api/v1/assets/upload
#[allow(private_interfaces)]
pub async fn upload(
    State(state): State<AssetState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, axum::Json<ApiResponse<UploadResponse>>), AppError> {
    let mut succ_map = std::collections::HashMap::new();

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return Err(AppError::BadRequest(format!("读取 multipart 字段失败: {}", e))),
        };

        let filename = field
            .file_name()
            .map(|n: &str| n.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let data = match field.bytes().await {
            Ok(b) => b,
            Err(e) => return Err(AppError::BadRequest(format!("读取文件数据失败: {}", e))),
        };

        let result = asset::upload_file(&state.data_root, &filename, &data)
            .map_err(AppError::BadRequest)?;

        succ_map.insert(result.original_name, result.path);
    }

    Ok((StatusCode::OK, axum::Json(ApiResponse::ok(Some(UploadResponse { succ_map })))))
}

/// GET /api/v1/assets/*path
pub async fn serve_asset(
    State(state): State<AssetState>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    match asset::resolve_asset_path(&state.data_root, &path) {
        Some(file_path) => {
            let content_type = guess_content_type(&file_path);

            match tokio::task::spawn_blocking(move || std::fs::read(&file_path)).await {
                Ok(Ok(bytes)) => (
                    StatusCode::OK,
                    [
                        (header::CONTENT_TYPE, content_type),
                        (header::CACHE_CONTROL, "public, max-age=31536000, immutable".to_string()),
                    ],
                    bytes,
                )
                    .into_response(),
                _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn guess_content_type(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("avif") => "image/avif",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("pdf") => "application/pdf",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
    .to_string()
}

pub fn asset_router(state: AssetState) -> Router<()> {
    Router::new()
        .route("/api/v1/assets/upload", post(upload))
        .route("/api/v1/assets/{*path}", get(serve_asset))
        .with_state(state)
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024))
}
