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

use axum::{Router, routing::{get, post}, extract::DefaultBodyLimit};
use axum::http::header::HeaderValue;
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
        // 文档级操作：CRUD + 导入/导出 + 跨文档嫁接 + Oplog
        .route("/api/v1/documents", post(handler::create_document))
        .route("/api/v1/documents/list", post(handler::list_documents))
        .route("/api/v1/documents/get", post(handler::get_document))
        .route("/api/v1/documents/children", post(handler::get_document_children))
        .route("/api/v1/documents/delete", post(handler::delete_document))
        .route("/api/v1/documents/export", post(handler::export_text))
        .route("/api/v1/documents/import", post(handler::import_text))
        .route("/api/v1/documents/move-document-tree", post(handler::move_document_tree))
        .route("/api/v1/documents/history", post(handler::get_block_history))
        .route("/api/v1/documents/undo", post(handler::undo))
        .route("/api/v1/documents/redo", post(handler::redo))
        // SSE（EventSource 只支持 GET，保留路径参数）
        .route("/api/v1/documents/{id}/events", get(handler::document_events))

        // ─── Block RPC ────────────────────────────────
        // Block 级操作：CRUD + 移动 + 恢复 + 拆分/合并 + 批量
        .route("/api/v1/blocks", post(handler::create_block))
        .route("/api/v1/blocks/get", post(handler::get_block))
        .route("/api/v1/blocks/update", post(handler::update_block))
        .route("/api/v1/blocks/delete", post(handler::delete_block))
        .route("/api/v1/blocks/move", post(handler::move_block))
        .route("/api/v1/blocks/move-heading-tree", post(handler::move_heading_tree))
        .route("/api/v1/blocks/restore", post(handler::restore_block))
        .route("/api/v1/blocks/split", post(handler::split_block))
        .route("/api/v1/blocks/merge", post(handler::merge_block))
        .route("/api/v1/blocks/batch", post(handler::batch_blocks))

        // 注入数据库 State
        .with_state(db)

        // 请求体大小限制（10 MB）
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))

        // CORS 中间件
        .layer(
            CorsLayer::new()
                .allow_origin(if cfg.server.cors_origin.is_empty() {
                    tower_http::cors::AllowOrigin::any()
                } else {
                    tower_http::cors::AllowOrigin::exact(
                        cfg.server.cors_origin.parse::<HeaderValue>()
                            .expect("WEM_CORS_ORIGIN 不是合法的 Origin 值")
                    )
                })
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
    println!();
    println!("   ── Document RPC ─────────────────────────────");
    println!("   POST   /api/v1/documents");
    println!("   POST   /api/v1/documents/list");
    println!("   POST   /api/v1/documents/get");
    println!("   POST   /api/v1/documents/children");
    println!("   POST   /api/v1/documents/delete");
    println!("   POST   /api/v1/documents/export");
    println!("   POST   /api/v1/documents/import");
    println!("   POST   /api/v1/documents/move-document-tree");
    println!("   POST   /api/v1/documents/history");
    println!("   POST   /api/v1/documents/undo");
    println!("   POST   /api/v1/documents/redo");
    println!("   GET    /api/v1/documents/{{id}}/events  [SSE]");
    println!();
    println!("   ── Block RPC ────────────────────────────────");
    println!("   POST   /api/v1/blocks");
    println!("   POST   /api/v1/blocks/get");
    println!("   POST   /api/v1/blocks/update");
    println!("   POST   /api/v1/blocks/delete");
    println!("   POST   /api/v1/blocks/move");
    println!("   POST   /api/v1/blocks/move-heading-tree");
    println!("   POST   /api/v1/blocks/restore");
    println!("   POST   /api/v1/blocks/split");
    println!("   POST   /api/v1/blocks/merge");
    println!("   POST   /api/v1/blocks/batch");

    // 启动 HTTP 服务器
    axum::serve(listener, app)
        .await
        .expect("Server error");
}
