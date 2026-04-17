//! 全局配置管理
//!
//! 支持三层配置覆盖（优先级从高到低）：
//! 1. 环境变量（`WEM_PORT`、`WEM_DB_PATH` 等）
//! 2. 配置文件（`wem.toml`，放在工作目录或 `WEM_CONFIG` 指定路径）
//! 3. 代码默认值
//!
//! 配置文件示例（wem.toml）：
//! ```toml
//! [server]
//! port = 6809
//! host = "0.0.0.0"
//!
//! [database]
//! path = "wem-data/wem.db"
//!
//! [oplog]
//! snapshot_threshold = 50
//! ```

use serde::Deserialize;
use std::sync::OnceLock;

// ─── 全局配置单例 ──────────────────────────────────────────────

/// 全局配置实例（程序启动时初始化一次，后续只读）
static CONFIG: OnceLock<Config> = OnceLock::new();

// ─── 配置结构体 ────────────────────────────────────────────────

/// 顶层配置
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub oplog: OplogConfig,
}

/// 服务器配置
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// HTTP 监听端口
    pub port: u16,
    /// HTTP 绑定地址
    pub host: String,
    /// CORS 允许的来源（空字符串表示允许所有）
    pub cors_origin: String,
}

/// 数据库配置
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    /// SQLite 数据库文件路径（相对于工作目录）
    pub path: String,
}

/// 操作日志配置
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct OplogConfig {
    /// 自动创建快照的操作数阈值（距上次快照多少次操作后触发）
    pub snapshot_threshold: i64,
}

// ─── 默认值实现 ────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            database: DatabaseConfig::default(),
            oplog: OplogConfig::default(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 6809,
            host: "0.0.0.0".to_string(),
            cors_origin: String::new(),
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: "wem-data/wem.db".to_string(),
        }
    }
}

impl Default for OplogConfig {
    fn default() -> Self {
        Self {
            snapshot_threshold: 50,
        }
    }
}

// ─── 配置加载 ──────────────────────────────────────────────────

/// 配置文件默认文件名
const DEFAULT_CONFIG_FILE: &str = "wem.toml";

/// 加载配置并初始化全局单例
///
/// 应在 `main()` 最开头调用一次。
/// 后续通过 `config::get()` 获取只读引用。
///
/// 加载顺序（后者覆盖前者）：
/// 1. 代码默认值
/// 2. 配置文件（`wem.toml` 或 `WEM_CONFIG` 环境变量指定路径）
/// 3. 环境变量覆盖
pub fn load() -> &'static Config {
    CONFIG.get_or_init(|| {
        // 1. 读取配置文件内容
        let config_path =
            std::env::var("WEM_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_FILE.to_string());
        let file_content = std::fs::read_to_string(&config_path).ok();

        // 2. 解析配置文件（如果有）
        let mut config: Config = match file_content {
            Some(content) => match toml::from_str(&content) {
                Ok(c) => {
                    println!("📋 配置文件已加载: {}", config_path);
                    c
                }
                Err(e) => {
                    eprintln!(
                        "⚠️  配置文件解析失败 ({}): {}，使用默认配置",
                        config_path, e
                    );
                    Config::default()
                }
            },
            None => {
                println!("📋 未找到配置文件 ({}), 使用默认配置", config_path);
                Config::default()
            }
        };

        // 3. 环境变量覆盖
        if let Ok(port) = std::env::var("WEM_PORT") {
            if let Ok(p) = port.parse::<u16>() {
                config.server.port = p;
            }
        }
        if let Ok(host) = std::env::var("WEM_HOST") {
            config.server.host = host;
        }
        if let Ok(origin) = std::env::var("WEM_CORS_ORIGIN") {
            config.server.cors_origin = origin;
        }
        if let Ok(path) = std::env::var("WEM_DB_PATH") {
            config.database.path = path;
        }
        if let Ok(threshold) = std::env::var("WEM_SNAPSHOT_THRESHOLD") {
            if let Ok(t) = threshold.parse::<i64>() {
                config.oplog.snapshot_threshold = t;
            }
        }

        config
    })
}

/// 获取全局配置（只读引用）
///
/// 如果 `load()` 尚未调用，会自动用默认值初始化。
/// 正常流程中 `main()` 已调用 `load()`，此处不会重复解析。
pub fn get() -> &'static Config {
    CONFIG.get_or_init(Config::default)
}
