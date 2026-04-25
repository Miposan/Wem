//! Wem Agent CLI — 独立的 Agent 对话客户端
//!
//! 直接调用 Provider + Agent Loop，不经过 HTTP。
//!
//! 用法:
//!   wem-agent              # 进入交互 REPL
//!   wem-agent "问题"       # 单次问答，输出后退出

use std::io::{self, Write};
use std::sync::Arc;

use wem_kernel::agent::loop_runner::AgentLoop;
use wem_kernel::agent::provider::anthropic::AnthropicProvider;
use wem_kernel::agent::provider::openai_compatible::OpenAICompatibleProvider;
use wem_kernel::agent::provider::Provider;
use wem_kernel::agent::session::{AgentEvent, Session, SessionConfig};
use wem_kernel::agent::tools::ToolRegistry;
use wem_kernel::agent::mcp::McpManager;

// ─── 配置 ──────────────────────────────────────────────────────────

/// 嵌入一份最小编译期配置结构（避免依赖 wem_kernel::config 中的 OnceLock 全局单例）
mod config {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(default)]
    pub struct Config {
        pub agent: AgentConfig,
    }

    #[derive(Debug, Deserialize)]
    #[serde(default)]
    pub struct AgentConfig {
        pub provider: String,
        pub api_key: String,
        pub base_url: String,
        pub model: String,
        pub max_tokens: u32,
        pub temperature: f32,
        pub max_steps: u32,
        pub api_key_env: String,
        #[serde(default)]
        pub custom_headers: std::collections::HashMap<String, String>,
        #[serde(default)]
        pub mcp_servers: Vec<wem_kernel::agent::mcp::McpServerConfig>,
    }

    impl Default for Config {
        fn default() -> Self {
            Self { agent: AgentConfig::default() }
        }
    }

    impl Default for AgentConfig {
        fn default() -> Self {
            Self {
                provider: "anthropic".to_string(),
                api_key: String::new(),
                base_url: "https://api.anthropic.com".to_string(),
                model: "claude-sonnet-4-20250514".to_string(),
                max_tokens: 16384,
                temperature: 0.3,
                max_steps: 50,
                api_key_env: "ANTHROPIC_API_KEY".to_string(),
                custom_headers: std::collections::HashMap::new(),
                mcp_servers: Vec::new(),
            }
        }
    }

    pub fn load() -> Config {
        let config_path =
            std::env::var("WEM_CONFIG").unwrap_or_else(|_| "wem.toml".to_string());
        let file_content = std::fs::read_to_string(&config_path).ok();

        let mut config: Config = match file_content {
            Some(content) => match toml::from_str(&content) {
                Ok(c) => {
                    eprintln!("config loaded: {}", config_path);
                    c
                }
                Err(e) => {
                    eprintln!("config parse error ({}): {}, using defaults", config_path, e);
                    Config::default()
                }
            },
            None => Config::default(),
        };

        // 环境变量覆盖
        if let Ok(v) = std::env::var("WEM_AGENT_PROVIDER") { config.agent.provider = v; }
        if let Ok(v) = std::env::var("WEM_AGENT_API_KEY") { config.agent.api_key = v; }
        if let Ok(v) = std::env::var("WEM_AGENT_BASE_URL") { config.agent.base_url = v; }
        if let Ok(v) = std::env::var("WEM_AGENT_MODEL") { config.agent.model = v; }
        // 优先从 api_key_env 指定的环境变量读 key
        if config.agent.api_key.is_empty() && !config.agent.api_key_env.is_empty() {
            if let Ok(key) = std::env::var(&config.agent.api_key_env) {
                config.agent.api_key = key;
            }
        }
        config
    }
}

// ─── Provider 构造 ─────────────────────────────────────────────────

fn build_provider(cfg: &config::AgentConfig) -> Arc<dyn Provider> {
    match cfg.provider.as_str() {
        "openai_compatible" => {
            let p = OpenAICompatibleProvider::with_headers(
                cfg.api_key.clone(),
                cfg.base_url.clone(),
                cfg.model.clone(),
                cfg.custom_headers.clone(),
            ).with_max_tokens(cfg.max_tokens);
            Arc::new(p)
        }
        _ => {
            let mut p = AnthropicProvider::new(cfg.api_key.clone());
            if cfg.base_url != "https://api.anthropic.com" {
                p = p.with_base_url(cfg.base_url.clone());
            }
            if cfg.model != "claude-sonnet-4-20250514" {
                p = p.with_model(cfg.model.clone());
            }
            Arc::new(p)
        }
    }
}

// ─── 主入口 ────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let cfg = config::load();
    eprintln!("provider: {} | model: {}", cfg.agent.provider, cfg.agent.model);

    let provider = build_provider(&cfg.agent);
    let mut registry = ToolRegistry::new();
    if !cfg.agent.mcp_servers.is_empty() {
        match McpManager::connect_all(&cfg.agent.mcp_servers).await {
            Ok((_manager, mcp_tools)) => {
                for tool in mcp_tools {
                    registry.register(tool);
                }
            }
            Err(e) => eprintln!("MCP connection failed: {}", e),
        }
    }
    let tools = Arc::new(registry);

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        run_repl(provider, tools, &cfg.agent).await;
    } else {
        run_once(provider, tools, &cfg.agent, &args.join(" ")).await;
    }
}

// ─── 单次问答 ──────────────────────────────────────────────────────

async fn run_once(
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    agent_cfg: &config::AgentConfig,
    question: &str,
) {
    let (event_tx, _) = tokio::sync::broadcast::channel::<AgentEvent>(256);
    let cancel = tokio_util::sync::CancellationToken::new();

    let mut session_cfg = SessionConfig::default();
    session_cfg.model = agent_cfg.model.clone();
    session_cfg.temperature = agent_cfg.temperature;
    session_cfg.max_steps = agent_cfg.max_steps;
    session_cfg.allowed_tools = tools.all_names();

    let session = Arc::new(tokio::sync::Mutex::new(Session::new(session_cfg)));

    let agent = AgentLoop::new(provider, tools, event_tx.clone(), cancel, 200_000);
    let question = question.to_string();

    let loop_handle = tokio::spawn(async move {
        if let Err(e) = agent.run(&session, question).await {
            eprintln!("agent error: {}", e);
        }
    });

    print_events(event_tx).await;
    let _ = loop_handle.await;
}

// ─── REPL 模式 ─────────────────────────────────────────────────────

async fn run_repl(
    provider: Arc<dyn Provider>,
    tools: Arc<ToolRegistry>,
    agent_cfg: &config::AgentConfig,
) {
    eprintln!("Wem Agent CLI — type your message, /help for commands, /quit to exit");
    eprintln!();

    let mut session_cfg = SessionConfig::default();
    session_cfg.model = agent_cfg.model.clone();
    session_cfg.temperature = agent_cfg.temperature;
    session_cfg.max_steps = agent_cfg.max_steps;
    session_cfg.allowed_tools = tools.all_names();
    session_cfg.working_dir = std::env::current_dir().unwrap_or_else(|_| ".".into());

    let session = Arc::new(tokio::sync::Mutex::new(Session::new(session_cfg)));

    loop {
        print!("you> ");
        io::stdout().flush().unwrap();

        let mut line = String::new();
        if io::stdin().read_line(&mut line).unwrap() == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line {
            "/quit" | "/exit" => break,
            "/help" => {
                eprintln!("  /help    show this help");
                eprintln!("  /quit    exit");
                eprintln!("  /clear   clear conversation history");
                eprintln!("  /history show message count");
                continue;
            }
            "/clear" => {
                let mut s = session.lock().await;
                s.messages.clear();
                eprintln!("(history cleared)");
                continue;
            }
            "/history" => {
                let s = session.lock().await;
                eprintln!("{} messages in session", s.messages.len());
                continue;
            }
            _ => {}
        }

        let (event_tx, _) = tokio::sync::broadcast::channel::<AgentEvent>(256);
        let cancel = tokio_util::sync::CancellationToken::new();

        let agent = AgentLoop::new(
            provider.clone(),
            tools.clone(),
            event_tx.clone(),
            cancel,
            200_000,
        );
        let user_msg = line.to_string();
        let session_clone = session.clone();

        let loop_handle = tokio::spawn(async move {
            if let Err(e) = agent.run(&session_clone, user_msg).await {
                eprintln!("agent error: {}", e);
            }
        });

        print_events(event_tx).await;
        let _ = loop_handle.await;
        println!();
    }
}

// ─── 事件打印 ──────────────────────────────────────────────────────

fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

async fn print_events(event_tx: tokio::sync::broadcast::Sender<AgentEvent>) {
    let mut rx = event_tx.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) => match event {
                AgentEvent::TextDelta { text } => {
                    print!("{}", text);
                    io::stdout().flush().ok();
                }
                AgentEvent::ToolCallBegin { name, args, .. } => {
                    eprintln!();
                    eprint!("  [tool: {} ", name);
                    let args_str = args.to_string();
                    if args_str.chars().count() > 120 {
                        eprint!("{}...", truncate(&args_str, 120));
                    } else {
                        eprint!("{}", args_str);
                    }
                    eprint!("]");
                }
                AgentEvent::ToolCallEnd { result_summary, .. } => {
                    if result_summary.chars().count() > 120 {
                        eprintln!(" -> {}...", truncate(&result_summary, 120));
                    } else {
                        eprintln!(" -> {}", result_summary);
                    }
                }
                AgentEvent::StepProgress { step, max_steps } => {
                    eprintln!("\n  --- step {}/{} ---", step + 1, max_steps);
                }
                AgentEvent::PhaseChanged { phase } => {
                    tracing::debug!("loop phase: {}", phase);
                }
                AgentEvent::PermissionRequired { tool_name, .. } => {
                    eprintln!("\n  [permission required: {} (auto-denied in CLI)]", tool_name);
                }
                AgentEvent::Done => {
                    println!();
                    break;
                }
                AgentEvent::Error { message } => {
                    eprintln!("\n  error: {}", message);
                    break;
                }
            },
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                continue;
            }
            Err(_) => break,
        }
    }
}
