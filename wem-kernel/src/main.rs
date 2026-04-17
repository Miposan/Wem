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
        // ─── 健康检查 ────────────────────────
        .route("/api/v1/health", get(handler::health))

        // ─── Document API ─────────────────────
        // 列出根文档 / 创建文档
        .route("/api/v1/documents",
            get(handler::list_documents).post(handler::create_document))
        // 获取文档 / 删除文档
        .route("/api/v1/documents/{id}",
            get(handler::get_document).delete(handler::delete_document))
        // 获取文档直系子文档（侧边栏导航用）
        .route("/api/v1/documents/{id}/children",
            get(handler::get_document_children))

        // ─── Block API ────────────────────────
        // 批量操作（必须在 /blocks/{id} 之前注册，避免路径冲突）
        .route("/api/v1/blocks/batch",
            post(handler::batch_blocks))
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

        // ─── 文本导入/导出 API ────────────
        // 导入文本（Markdown → Block 树）
        .route("/api/v1/blocks/import",
            post(handler::import_text))
        // 导出文档（Block 树 → Markdown）
        .route("/api/v1/documents/{id}/export",
            get(handler::export_text))

        // ─── SSE 实时事件 ─────────────────
        // 前端 EventSource 连接，接收文档变更推送
        .route("/api/v1/documents/{id}/events",
            get(handler::document_events))

        // ─── Oplog / 历史版本 API ─────────
        // 获取 Block 变更历史
        .route("/api/v1/blocks/{id}/history",
            get(handler::get_block_history))
        // 获取 Block 指定版本内容
        .route("/api/v1/blocks/{id}/versions/{version}",
            get(handler::get_block_version))
        // 回滚 Block 到指定版本
        .route("/api/v1/blocks/{id}/rollback",
            post(handler::rollback_block))
        // 手动创建快照
        .route("/api/v1/blocks/{id}/snapshot",
            post(handler::create_block_snapshot))

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
