//! Wem Kernel — Block 操作系统内核
//!
//! 这是整个 Wem 知识管理系统的 Rust 内核。
//! Block = 一切数据的原子单位（文档、段落、标题、列表项……都是 Block）。
//! 内核提供 REST API，人和 AI 通过同一套接口操作 Block 树。

// 声明模块（每个 mod 对应 src/ 下的同名文件或目录下的 mod.rs）
mod api;      // API 数据传输对象（请求/响应/查询参数）
mod config;   // 全局配置（端口、数据库路径）
mod error;    // 统一错误处理 + API 响应格式
mod model;    // 数据模型（Block、BlockType 等）
mod db;       // 数据库层（SQLite 连接、建表）
mod service;  // 业务逻辑层（Block CRUD、树操作）
mod handler;  // HTTP 处理层（Axum route handlers）

use axum::{Router, routing::{get, post}};

// ─── 启动入口 ───────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // 初始化数据库
    let db = db::init_db(config::DB_PATH)
        .expect("数据库初始化失败");

    // 创建 Axum 路由器，注册所有 API 路由
    let app = Router::new()
        // ─── 健康检查 ────────────────────────
        .route("/api/v1/health", get(handler::health))

        // ─── Root API ─────────────────────────
        // 获取全局根块
        .route("/api/v1/root", get(handler::get_root))

        // ─── Document API ─────────────────────
        // 列出根文档 / 创建文档
        .route("/api/v1/documents",
            get(handler::list_documents).post(handler::create_document))
        // 获取文档 / 删除文档
        .route("/api/v1/documents/{id}",
            get(handler::get_document).delete(handler::delete_document))
        // 获取文档树
        .route("/api/v1/documents/{id}/tree",
            get(handler::get_document_tree))

        // ─── Block API ────────────────────────
        // 创建 Block
        .route("/api/v1/blocks",
            post(handler::create_block))
        // 获取 / 更新 / 删除 Block
        .route("/api/v1/blocks/{id}",
            get(handler::get_block).put(handler::update_block).delete(handler::delete_block))
        // 移动 Block
        .route("/api/v1/blocks/{id}/move",
            post(handler::move_block))
        // 恢复 Block
        .route("/api/v1/blocks/{id}/restore",
            post(handler::restore_block))
        // 获取子块列表
        .route("/api/v1/blocks/{id}/children",
            get(handler::get_children))

        // 注入数据库 State
        .with_state(db);

    // 绑定 TCP 监听端口
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config::PORT))
        .await
        .expect("Failed to bind port");

    println!("🚀 Wem Kernel listening on port {}", config::PORT);
    println!("📋 API 端点:");
    println!("   GET    /api/v1/health");
    println!("   GET    /api/v1/root");
    println!("   GET    /api/v1/documents");
    println!("   POST   /api/v1/documents");
    println!("   GET    /api/v1/documents/{{id}}");
    println!("   GET    /api/v1/documents/{{id}}/tree");
    println!("   DELETE /api/v1/documents/{{id}}");
    println!("   POST   /api/v1/blocks");
    println!("   GET    /api/v1/blocks/{{id}}");
    println!("   PUT    /api/v1/blocks/{{id}}");
    println!("   DELETE /api/v1/blocks/{{id}}");
    println!("   POST   /api/v1/blocks/{{id}}/move");
    println!("   POST   /api/v1/blocks/{{id}}/restore");
    println!("   GET    /api/v1/blocks/{{id}}/children");

    // 启动 HTTP 服务器
    axum::serve(listener, app)
        .await
        .expect("Server error");
}
