//! ShellOps — Shell 命令执行工具

use async_trait::async_trait;
use serde::Deserialize;

use super::{Tool, ToolContext, ToolResult};

pub struct ShellExec;

#[derive(Deserialize)]
struct ShellExecArgs {
    command: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

#[async_trait]
impl Tool for ShellExec {
    fn name(&self) -> &str { "shell_exec" }
    fn description(&self) -> &str { "Execute a shell command and return output" }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default 30)" }
            },
            "required": ["command"]
        })
    }
    fn prompt(&self) -> &str {
        "Execute a shell command. Working directory is the session's working dir. Use with caution."
    }

    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let args: ShellExecArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(format!("Invalid args: {}", e)),
        };

        let timeout = tokio::time::Duration::from_secs(args.timeout_secs.unwrap_or(30));

        let output = tokio::time::timeout(
            timeout,
            if cfg!(target_os = "windows") {
                tokio::process::Command::new("cmd")
                    .args(["/C", &args.command])
                    .current_dir(&ctx.working_dir)
                    .output()
            } else {
                tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(&args.command)
                    .current_dir(&ctx.working_dir)
                    .output()
            },
        )
        .await;

        match output {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if output.status.success() {
                    ToolResult::ok(stdout.to_string())
                } else {
                    ToolResult::error(format!(
                        "exit code {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                        output.status.code().unwrap_or(-1),
                        stdout,
                        stderr,
                    ))
                }
            }
            Ok(Err(e)) => ToolResult::error(format!("Failed to execute: {}", e)),
            Err(_) => ToolResult::error("Command timed out"),
        }
    }
}
