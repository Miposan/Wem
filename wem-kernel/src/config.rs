//! 全局配置常量
//!
//! 集中管理端口号、数据库路径等配置。
//! 后续需要配置文件时再改成从文件读取。

/// HTTP 服务监听端口（参考 00-architecture.md，沿用 SiYuan 的 6809）
pub const PORT: u16 = 6809;

/// SQLite 数据库文件路径（相对于工作目录）
/// 启动时会自动创建 wem-data/ 目录和 wem.db 文件
pub const DB_PATH: &str = "wem-data/wem.db";
