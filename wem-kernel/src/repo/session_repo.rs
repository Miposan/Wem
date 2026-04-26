//! Agent Session 数据访问层
//!
//! 持久化 AI 对话会话和消息到 SQLite。
//! 与 block_repo 遵循相同模式：接收 `&Connection`，加锁由调用方负责。

use rusqlite::{params, Connection};

use crate::agent::provider::Message;
use crate::agent::session::SessionConfig;

// ─── Session CRUD ──────────────────────────────────────────────────

/// 插入一条会话记录
pub fn insert_session(
    conn: &Connection,
    id: &str,
    config: &SessionConfig,
    created_at: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO agent_sessions (id, model, temperature, max_steps, working_dir, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![id, config.model, config.temperature, config.max_steps, config.working_dir.to_string_lossy().to_string(), created_at],
    )?;
    Ok(())
}

/// 删除会话及其所有消息（CASCADE）
pub fn delete_session(conn: &Connection, id: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM agent_sessions WHERE id = ?1", params![id])?;
    Ok(())
}

/// 加载所有会话的 (id, config, created_at)
pub fn list_sessions(conn: &Connection) -> Result<Vec<(String, SessionConfig, String)>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, model, temperature, max_steps, working_dir, created_at FROM agent_sessions ORDER BY updated_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let model: String = row.get(1)?;
        let temperature: f32 = row.get(2)?;
        let max_steps: u32 = row.get(3)?;
        let working_dir: String = row.get(4)?;
        let created_at: String = row.get(5)?;
        let config = SessionConfig {
            model,
            temperature,
            max_steps,
            allowed_tools: vec![],
            system_prompt_override: None,
            working_dir: std::path::PathBuf::from(working_dir),
        };
        Ok((id, config, created_at))
    })?;
    rows.collect()
}

/// 更新会话的 updated_at 时间戳
pub fn touch_session(conn: &Connection, id: &str, updated_at: &str) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE agent_sessions SET updated_at = ?2 WHERE id = ?1",
        params![id, updated_at],
    )?;
    Ok(())
}

// ─── Messages CRUD ─────────────────────────────────────────────────

/// 替换会话的全部消息（先删后插，简单可靠）
pub fn replace_messages(
    conn: &Connection,
    session_id: &str,
    messages: &[Message],
    now: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM agent_messages WHERE session_id = ?1", params![session_id])?;
    for (seq, msg) in messages.iter().enumerate() {
        let role = match msg.role {
            crate::agent::provider::Role::System => "system",
            crate::agent::provider::Role::User => "user",
            crate::agent::provider::Role::Assistant => "assistant",
        };
        let content = serde_json::to_string(&msg.content).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO agent_messages (session_id, seq, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, seq as i64, role, content, now],
        )?;
    }
    Ok(())
}

/// 加载会话的全部消息（按 seq 排序）
pub fn load_messages(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<Message>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT role, content FROM agent_messages WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![session_id], |row| {
        let role_str: String = row.get(0)?;
        let content_json: String = row.get(1)?;
        let role = match role_str.as_str() {
            "system" => crate::agent::provider::Role::System,
            "assistant" => crate::agent::provider::Role::Assistant,
            _ => crate::agent::provider::Role::User,
        };
        let content: Vec<crate::agent::provider::ContentBlock> =
            serde_json::from_str(&content_json).unwrap_or_default();
        Ok(Message { role, content })
    })?;
    rows.collect()
}
