//! Wem Kernel — 启动 HTTP server + CLI TUI

use std::sync::Arc;

use wem_kernel::agent::handler::AgentState;
use wem_kernel::agent::mcp::McpManager;
use wem_kernel::agent::provider::anthropic_compatible::AnthropicProvider;
use wem_kernel::agent::provider::openai_compatible::OpenAICompatibleProvider;
use wem_kernel::agent::provider::Provider;
use wem_kernel::agent::runtime::AgentRuntime;
use wem_kernel::agent::session::SessionManager;
use wem_kernel::agent::tools::ToolRegistry;
use wem_kernel::config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::load();
    let addr = format!("{}:{}", cfg.server.host, cfg.server.port);
    let db = wem_kernel::repo::init_db(&cfg.database.path)?;

    // ─── 构建 Agent 核心 ──────────────────────────────────

    let provider: Arc<dyn Provider> = match cfg.agent.provider.as_str() {
        "openai_compatible" => {
            let mut p = OpenAICompatibleProvider::with_headers(
                cfg.agent.api_key.clone(),
                cfg.agent.base_url.clone(),
                cfg.agent.model.clone(),
                cfg.agent.custom_headers.clone(),
            );
            p = p.with_max_tokens(cfg.agent.max_tokens);
            Arc::new(p)
        }
        _ => {
            let mut p = AnthropicProvider::new(cfg.agent.api_key.clone());
            if cfg.agent.base_url != "https://api.anthropic.com" {
                p = p.with_base_url(cfg.agent.base_url.clone());
            }
            if cfg.agent.model != "claude-sonnet-4-20250514" {
                p = p.with_model(cfg.agent.model.clone());
            }
            Arc::new(p)
        }
    };

    let mut registry = ToolRegistry::new();
    if !cfg.agent.mcp_servers.is_empty() {
        match McpManager::connect_all(&cfg.agent.mcp_servers).await {
            Ok((_manager, mcp_tools)) => {
                for tool in mcp_tools { registry.register(tool); }
            }
            Err(e) => eprintln!("MCP connection failed: {}", e),
        }
    }
    let tools = Arc::new(registry);
    let runtime = Arc::new(AgentRuntime::new(
        Arc::new(SessionManager::with_db(db.clone())),
        provider,
        tools,
        200_000,
    ));

    // ─── 后台启动 HTTP server ──────────────────────────────

    let agent_state = Arc::new(AgentState { runtime: runtime.clone() });
    let asset_data_root = std::path::Path::new(&cfg.database.path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    let app = wem_kernel::server::build_app(
        db, agent_state, asset_data_root, &cfg.server.cors_origin,
    );
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("HTTP server on {}", addr);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("HTTP server error: {}", e);
        }
    });

    // ─── 前台启动 CLI TUI ──────────────────────────────────

    let model = cfg.agent.model.clone();
    wem_kernel::cli::run(runtime, model).await?;

    Ok(())
}
