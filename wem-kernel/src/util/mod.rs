//! 纯工具函数
//!
//! 零外部依赖的算法、辅助函数。
//! 与 service 层的区别：这里不涉及数据库、业务逻辑，只有纯计算。

/// 生成当前时间的 ISO 8601 字符串（毫秒精度）
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
