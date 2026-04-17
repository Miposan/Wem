//! Wem Kernel — Block 操作系统内核
//!
//! 这是整个 Wem 知识管理系统的 Rust 内核。
//! Block = 一切数据的原子单位（文档、段落、标题、列表项……都是 Block）。
//! 内核提供 REST API，人和 AI 通过同一套接口操作 Block 树。

// 声明模块（每个 mod 对应 src/ 下的同名文件或目录下的 mod.rs）
mod api;        // API 数据传输对象（请求/响应/查询参数）
mod config;     // 全局配置（端口、数据库路径）
mod error;      // 统一错误处理 + API 响应格式
mod model;      // 数据模型（Block、BlockType 等）
mod repo;        // 数据访问层（SQLite 连接、建表、查询）
mod service;    // 业务逻辑层（Block CRUD、树操作）
mod handler;    // HTTP 处理层（Axum route handlers）
mod parser;     // 文本格式转换（Markdown ↔ Block 树，可扩展）
mod util;       // 纯工具函数（零外部依赖的算法）

use axum::{Router, routing::{get, post}};
use tower_http::cors::{CorsLayer, Any};

// ─── 启动入口 ───────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // 加载配置（配置文件 + 环境变量）
    let cfg = config::load();
    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let db = repo::init_db(&cfg.database.path)
        .expect("数据库初始化失败");


    // 创建 Axum 路由器，注册所有 API 路由
    let app = Router::new()
        // ─── 健康检查（唯一保留 GET 的非 SSE 端点） ──────
        .route("/api/v1/health", get(handler::health))

        // ─── Document RPC ─────────────────────────────
        .route("/api/v1/documents", post(handler::create_document))
        .route("/api/v1/documents/list", post(handler::list_documents))
        .route("/api/v1/documents/get", post(handler::get_document))
        .route("/api/v1/documents/children", post(handler::get_document_children))
        .route("/api/v1/documents/delete", post(handler::delete_document))
        .route("/api/v1/documents/export", post(handler::export_text))
        // SSE（EventSource 只支持 GET，保留路径参数）
        .route("/api/v1/documents/{id}/events", get(handler::document_events))

        // ─── Block RPC ────────────────────────────────
        .route("/api/v1/blocks", post(handler::create_block))
        .route("/api/v1/blocks/get", post(handler::get_block))
        .route("/api/v1/blocks/update", post(handler::update_block))
        .route("/api/v1/blocks/delete", post(handler::delete_block))
        .route("/api/v1/blocks/move", post(handler::move_block))
        .route("/api/v1/blocks/restore", post(handler::restore_block))
        .route("/api/v1/blocks/split", post(handler::split_block))
        .route("/api/v1/blocks/merge", post(handler::merge_block))
        .route("/api/v1/blocks/batch", post(handler::batch_blocks))
        .route("/api/v1/blocks/import", post(handler::import_text))

        // ─── Oplog / 历史版本 RPC ────────────────────
        .route("/api/v1/blocks/history", post(handler::get_block_history))
        .route("/api/v1/blocks/version", post(handler::get_block_version))
        .route("/api/v1/blocks/rollback", post(handler::rollback_block))
        .route("/api/v1/blocks/snapshot", post(handler::create_block_snapshot))

        // 注入数据库 State
        .with_state(db)

        // CORS 中间件：允许前端开发服务器（如 Vite localhost:5173）跨域访问
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    // 绑定 TCP 监听端口
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind port");

    println!("🚀 Wem Kernel listening on {}", addr);
    println!("📋 API 端点:");
    println!("   GET    /api/v1/health");
    println!("   GET    /api/v1/root");
    println!("   GET    /api/v1/documents");
    println!("   POST   /api/v1/documents");
    println!("   GET    /api/v1/documents/{{id}}");
    println!("   GET    /api/v1/documents/{{id}}/tree");
    println!("   DELETE /api/v1/documents/{{id}}");
    println!("   POST   /api/v1/blocks");
    println!("   POST   /api/v1/blocks/batch");
    println!("   GET    /api/v1/blocks/{{id}}");
    println!("   PUT    /api/v1/blocks/{{id}}");
    println!("   DELETE /api/v1/blocks/{{id}}");
    println!("   POST   /api/v1/blocks/{{id}}/move");
    println!("   POST   /api/v1/blocks/{{id}}/restore");
    println!("   GET    /api/v1/blocks/{{id}}/children");
    println!("   POST   /api/v1/blocks/import");
    println!("   GET    /api/v1/documents/{{id}}/export");
    println!("   GET    /api/v1/documents/{{id}}/events  [SSE]");

    // 启动 HTTP 服务器
    axum::serve(listener, app)
        .await
        .expect("Server error");
}
