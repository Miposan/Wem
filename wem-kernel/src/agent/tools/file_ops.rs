//! FileOps — 文件系统工具

use async_trait::async_trait;
use serde::Deserialize;

use super::{Tool, ToolContext, ToolResult};

// ─── FileRead ──────────────────────────────────────────────────

pub struct FileRead;

#[derive(Deserialize)]
struct FileReadArgs {
    path: String,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for FileRead {
    fn name(&self) -> &str { "file_read" }
    fn description(&self) -> &str { "Read file content from disk" }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path (absolute or relative to working dir)" },
                "offset": { "type": "integer", "description": "Start line (0-based), default 0" },
                "limit": { "type": "integer", "description": "Max lines to read, default all" }
            },
            "required": ["path"]
        })
    }
    fn prompt(&self) -> &str {
        "Read a file from disk. Use absolute paths when possible. Returns file content as text."
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let args: FileReadArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(format!("Invalid args: {}", e)),
        };
        match tokio::fs::read_to_string(&args.path).await {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let start = args.offset.unwrap_or(0);
                let end = args.limit.map(|l| (start + l).min(lines.len()))
                    .unwrap_or(lines.len());
                if start >= lines.len() {
                    ToolResult::ok("(empty range)")
                } else {
                    let result: Vec<&str> = lines[start..end].to_vec();
                    ToolResult::ok(result.join("\n"))
                }
            }
            Err(e) => ToolResult::error(format!("Failed to read {}: {}", args.path, e)),
        }
    }
}

// ─── FileWrite ─────────────────────────────────────────────────

pub struct FileWrite;

#[derive(Deserialize)]
struct FileWriteArgs {
    path: String,
    content: String,
}

#[async_trait]
impl Tool for FileWrite {
    fn name(&self) -> &str { "file_write" }
    fn description(&self) -> &str { "Write content to a file (creates or overwrites)" }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
        })
    }
    fn prompt(&self) -> &str {
        "Write content to a file. Will create parent directories if needed. Overwrites existing files."
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let args: FileWriteArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(format!("Invalid args: {}", e)),
        };
        if let Some(parent) = std::path::Path::new(&args.path).parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::error(format!("Failed to create dirs: {}", e));
            }
        }
        match tokio::fs::write(&args.path, &args.content).await {
            Ok(()) => ToolResult::ok(format!("Written {} bytes to {}", args.content.len(), args.path)),
            Err(e) => ToolResult::error(format!("Failed to write {}: {}", args.path, e)),
        }
    }
}

// ─── FileEdit ──────────────────────────────────────────────────

pub struct FileEdit;

#[derive(Deserialize)]
struct FileEditArgs {
    path: String,
    old_text: String,
    new_text: String,
    #[serde(default)]
    replace_all: bool,
}

#[async_trait]
impl Tool for FileEdit {
    fn name(&self) -> &str { "file_edit" }
    fn description(&self) -> &str { "Replace text in a file (exact string match)" }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path" },
                "old_text": { "type": "string", "description": "Text to find (exact match)" },
                "new_text": { "type": "string", "description": "Replacement text" },
                "replace_all": { "type": "boolean", "description": "Replace all occurrences (default false)" }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }
    fn prompt(&self) -> &str {
        "Replace exact text in a file. old_text must match exactly (including whitespace). Use replace_all for renaming."
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let args: FileEditArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(format!("Invalid args: {}", e)),
        };
        let content = match tokio::fs::read_to_string(&args.path).await {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Failed to read {}: {}", args.path, e)),
        };

        let count = if args.replace_all {
            content.matches(&args.old_text).count()
        } else {
            if content.contains(&args.old_text) { 1 } else { 0 }
        };

        if count == 0 {
            return ToolResult::error(format!(
                "old_text not found in {}. Make sure the text matches exactly.",
                args.path
            ));
        }

        let new_content = if args.replace_all {
            content.replace(&args.old_text, &args.new_text)
        } else {
            content.replacen(&args.old_text, &args.new_text, 1)
        };

        match tokio::fs::write(&args.path, &new_content).await {
            Ok(()) => ToolResult::ok(format!("Replaced {} occurrence(s) in {}", count, args.path)),
            Err(e) => ToolResult::error(format!("Failed to write {}: {}", args.path, e)),
        }
    }
}
