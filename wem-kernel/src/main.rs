//! Wem Kernel — Block 操作系统内核
//!
//! 这是整个 Wem 知识管理系统的 Rust 内核。
//! Block = 一切数据的原子单位（文档、段落、标题、列表项……都是 Block）。
//! 内核提供 REST API，人和 AI 通过同一套接口操作 Block 树。

// 模块声明在 lib.rs 中，main.rs 只保留 config（服务器专用）
mod config;     // 全局配置（端口、数据库路径）

// handler 直接在 main.rs 中使用 lib 的路径
use wem_kernel::block_system::handler;
use wem_kernel::agent::handler as agent_handler;
use wem_kernel::agent::provider::anthropic::AnthropicProvider;
use wem_kernel::agent::provider::openai_compatible::OpenAICompatibleProvider;
use wem_kernel::agent::provider::Provider;
use wem_kernel::agent::runtime::AgentRuntime;
use wem_kernel::agent::session::SessionManager;
use wem_kernel::agent::tools::ToolRegistry;
use wem_kernel::agent::handler::AgentState;
use wem_kernel::agent::mcp::McpManager;

use axum::{Router, routing::{get, post}, extract::DefaultBodyLimit};
use axum::http::header::HeaderValue;
use tower_http::cors::{CorsLayer, Any};

// ─── 启动入口 ───────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载配置（配置文件 + 环境变量）
    let cfg = config::load();
    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let db = wem_kernel::repo::init_db(&cfg.database.path)?;


    // 创建 Axum 路由器，注册所有 API 路由

    // Agent 子系统状态
    let provider: std::sync::Arc<dyn Provider> = match cfg.agent.provider.as_str() {
        "openai_compatible" => {
            let mut p = OpenAICompatibleProvider::with_headers(
                cfg.agent.api_key.clone(),
                cfg.agent.base_url.clone(),
                cfg.agent.model.clone(),
                cfg.agent.custom_headers.clone(),
            );
            p = p.with_max_tokens(cfg.agent.max_tokens);
            std::sync::Arc::new(p)
        }
        _ => {
            let mut p = AnthropicProvider::new(cfg.agent.api_key.clone());
            if cfg.agent.base_url != "https://api.anthropic.com" {
                p = p.with_base_url(cfg.agent.base_url.clone());
            }
            if cfg.agent.model != "claude-sonnet-4-20250514" {
                p = p.with_model(cfg.agent.model.clone());
            }
            std::sync::Arc::new(p)
        }
    };
    let agent_state = {
        let mut registry = ToolRegistry::new();
        if !cfg.agent.mcp_servers.is_empty() {
            match McpManager::connect_all(&cfg.agent.mcp_servers).await {
                Ok((_manager, mcp_tools)) => {
                    for tool in mcp_tools {
                        registry.register(tool);
                    }
                }
                Err(e) => eprintln!("⚠️  MCP 连接失败: {}", e),
            }
        }
        let tools = std::sync::Arc::new(registry);
        let runtime = std::sync::Arc::new(AgentRuntime::new(
            std::sync::Arc::new(SessionManager::new()),
            provider,
            tools,
            200_000,
        ));
        std::sync::Arc::new(AgentState {
            runtime,
        })
    };

    let block_routes = Router::new()
        // ─── 健康检查（唯一保留 GET 的非 SSE 端点） ──────
        .route("/api/v1/health", get(handler::health))

        // ─── Document RPC ─────────────────────────────
        // 文档级操作：CRUD + 导入/导出 + Oplog
        .route("/api/v1/documents", post(handler::create_document))
        .route("/api/v1/documents/list", post(handler::list_documents))
        .route("/api/v1/documents/get", post(handler::get_document))
        .route("/api/v1/documents/children", post(handler::get_document_children))
        .route("/api/v1/documents/delete", post(handler::delete_document))
        .route("/api/v1/documents/export", post(handler::export_text))
        .route("/api/v1/documents/import", post(handler::import_text))
        .route("/api/v1/documents/move", post(handler::move_document_tree))
        .route("/api/v1/documents/history", post(handler::get_block_history))
        .route("/api/v1/documents/undo", post(handler::undo))
        .route("/api/v1/documents/redo", post(handler::redo))
        // SSE（EventSource 只支持 GET，保留路径参数）
        .route("/api/v1/documents/{id}/events", get(handler::document_events))

        // ─── Block RPC ────────────────────────────────
        // Block 级操作：CRUD + 移动 + 恢复 + 拆分/合并 + 批量 + 导出
        .route("/api/v1/blocks", post(handler::create_block))
        .route("/api/v1/blocks/get", post(handler::get_block))
        .route("/api/v1/blocks/update", post(handler::update_block))
        .route("/api/v1/blocks/delete", post(handler::delete_block))
        .route("/api/v1/blocks/delete-tree", post(handler::delete_tree))
        .route("/api/v1/blocks/move", post(handler::move_block))
        .route("/api/v1/blocks/move-tree", post(handler::move_heading_tree))
        .route("/api/v1/blocks/export", post(handler::export_block))
        .route("/api/v1/blocks/restore", post(handler::restore_block))
        .route("/api/v1/blocks/split", post(handler::split_block))
        .route("/api/v1/blocks/merge", post(handler::merge_block))
        .route("/api/v1/blocks/batch", post(handler::batch_blocks))

        // 注入数据库 State
        .with_state(db);

    let agent_routes = Router::new()
        .route("/api/v1/agent/health", get(agent_handler::health))
        .route("/api/v1/agent/sessions", post(agent_handler::create_session))
        .route("/api/v1/agent/sessions/list", post(agent_handler::list_sessions))
        .route("/api/v1/agent/sessions/{id}", post(agent_handler::destroy_session))
        .route("/api/v1/agent/sessions/{id}/chat", post(agent_handler::chat))
        .route("/api/v1/agent/sessions/{id}/abort", post(agent_handler::abort_session))
        .with_state(agent_state);

    let app = block_routes
        .merge(agent_routes)

        // 请求体大小限制（10 MB）
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))

        // CORS 中间件
        .layer(
            CorsLayer::new()
                .allow_origin(if cfg.server.cors_origin.is_empty() {
                    tower_http::cors::AllowOrigin::any()
                } else {
                    tower_http::cors::AllowOrigin::exact(
                        cfg.server.cors_origin.parse::<HeaderValue>()?
                    )
                })
                .allow_methods(Any)
                .allow_headers(Any),
        );

    // 绑定 TCP 监听端口
    let listener = tokio::net::TcpListener::bind(&addr).await?;

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
    println!("   POST   /api/v1/documents/move");
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
    println!("   POST   /api/v1/blocks/move-tree");
    println!("   POST   /api/v1/blocks/export");
    println!("   POST   /api/v1/blocks/restore");
    println!("   POST   /api/v1/blocks/split");
    println!("   POST   /api/v1/blocks/merge");
    println!("   POST   /api/v1/blocks/batch");
    println!("   POST   /api/v1/blocks/delete-tree");
    println!();
    println!("   ── Agent ───────────────────────────────────");
    println!("   GET    /api/v1/agent/health");
    println!("   POST   /api/v1/agent/sessions");
    println!("   POST   /api/v1/agent/sessions/list");
    println!("   POST   /api/v1/agent/sessions/{{id}}");
    println!("   POST   /api/v1/agent/sessions/{{id}}/chat");
    println!("   POST   /api/v1/agent/sessions/{{id}}/abort");

    // 启动 HTTP 服务器
    axum::serve(listener, app).await?;
    Ok(())
}
