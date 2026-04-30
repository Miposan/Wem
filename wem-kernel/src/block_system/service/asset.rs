//! 资源文件存储服务
//!
//! 负责文件的物理存储和路径管理。
//! 文件存放在 `{data_dir}/assets/` 目录下，以 `{hash}.{ext}` 命名。

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

const ASSETS_DIR_NAME: &str = "assets";

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "avif", "ico",
];

const ALLOWED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "avif", "ico",
    "pdf", "mp3", "wav", "ogg", "m4a", "flac",
    "mp4", "webm", "mov",
];

const MAX_FILE_SIZE: usize = 20 * 1024 * 1024;

#[derive(Debug, serde::Serialize)]
pub struct UploadResult {
    pub path: String,
    pub original_name: String,
    pub size: usize,
    pub hash: String,
}

fn assets_root(data_root: &str) -> PathBuf {
    Path::new(data_root).join(ASSETS_DIR_NAME)
}

fn extract_ext(filename: &str) -> Option<String> {
    Path::new(filename)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
}

fn is_allowed_ext(ext: &str) -> bool {
    ALLOWED_EXTENSIONS.contains(&ext)
}

#[allow(dead_code)]
pub fn is_image_ext(ext: &str) -> bool {
    IMAGE_EXTENSIONS.contains(&ext)
}

fn hash_data(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

pub fn upload_file(
    data_root: &str,
    filename: &str,
    data: &[u8],
) -> Result<UploadResult, String> {
    if data.is_empty() {
        return Err("文件为空".to_string());
    }
    if data.len() > MAX_FILE_SIZE {
        return Err(format!("文件大小超出限制 ({}MB)", MAX_FILE_SIZE / 1024 / 1024));
    }

    let ext = extract_ext(filename)
        .ok_or_else(|| "无法识别文件扩展名".to_string())?;

    if !is_allowed_ext(&ext) {
        return Err(format!("不支持的文件类型: .{}", ext));
    }

    let root = assets_root(data_root);
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("创建资源目录失败: {}", e))?;

    let hash = hash_data(data);
    let stored_name = format!("{}.{}", hash, ext);
    let full_path = root.join(&stored_name);
    let relative_path = format!("{}/{}", ASSETS_DIR_NAME, stored_name);

    if full_path.exists() {
        return Ok(UploadResult {
            path: relative_path,
            original_name: filename.to_string(),
            size: data.len(),
            hash,
        });
    }

    std::fs::write(&full_path, data)
        .map_err(|e| format!("写入文件失败: {}", e))?;

    Ok(UploadResult {
        path: relative_path,
        original_name: filename.to_string(),
        size: data.len(),
        hash,
    })
}

pub fn resolve_asset_path(data_root: &str, relative_path: &str) -> Option<PathBuf> {
    let root = assets_root(data_root);
    let full_path = root.join(relative_path);
    let canonical_root = root.canonicalize().ok()?;
    let canonical_path = full_path.canonicalize().ok()?;
    if canonical_path.starts_with(&canonical_root) && canonical_path.is_file() {
        Some(canonical_path)
    } else {
        None
    }
}
