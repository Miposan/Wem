//! Permission Gate — 工具执行前的权限拦截
//!
//! 当前实现为简化版：所有操作 auto。
//! 后续 P2 阶段实现完整的 auto/ask/deny 规则。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    Auto,
    Ask,
    Deny,
}

pub struct PermissionGate {
    /// 会话级缓存：已经批准过的 (tool_name, args_hash) → Auto
    approved: std::collections::HashSet<String>,
}

impl PermissionGate {
    pub fn new() -> Self {
        Self {
            approved: std::collections::HashSet::new(),
        }
    }

    pub fn check(&self, tool_name: &str, _args: &serde_json::Value) -> Permission {
        let _ = tool_name;
        Permission::Auto
    }

    pub fn check_with_cache(&mut self, tool_name: &str, args: &serde_json::Value) -> Permission {
        let cache_key = format!("{}:{}", tool_name, simple_hash(args));
        if self.approved.contains(&cache_key) {
            return Permission::Auto;
        }
        self.check(tool_name, args)
    }

    pub fn approve(&mut self, tool_name: &str, args: &serde_json::Value) {
        let cache_key = format!("{}:{}", tool_name, simple_hash(args));
        self.approved.insert(cache_key);
    }
}

fn simple_hash(v: &serde_json::Value) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let s = v.to_string();
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}
