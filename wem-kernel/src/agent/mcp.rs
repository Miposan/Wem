//! MCP (Model Context Protocol) Client Integration
//!
//! Connects to external MCP servers via stdio child-process transport,
//! discovers their tools, and wraps them as Tool trait implementations
//! for use alongside built-in tools in the Agent loop.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rmcp::ServiceExt;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::tools::{Tool, ToolContext, ToolResult};

// ─── MCP Server Config ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Logical name for this MCP server (used in tool name prefix)
    pub name: String,
    /// Command to launch the MCP server process
    pub command: String,
    /// Arguments to pass to the command
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables to set for the child process
    #[serde(default)]
    pub env: HashMap<String, String>,
}

// ─── Discovered Tool Info ──────────────────────────────────────────

struct DiscoveredTool {
    original_name: String,
    description: String,
    input_schema: serde_json::Value,
}

// ─── McpTool — Tool wrapper for an MCP remote tool ─────────────────

pub struct McpTool {
    prefixed_name: String,
    info: DiscoveredTool,
    service: Arc<McpServiceHandle>,
}

struct McpServiceHandle {
    inner: Mutex<rmcp::service::RunningService<rmcp::RoleClient, ()>>,
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.prefixed_name
    }

    fn description(&self) -> &str {
        &self.info.description
    }

    fn input_schema(&self) -> serde_json::Value {
        self.info.input_schema.clone()
    }

    fn prompt(&self) -> &str {
        ""
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let arguments = match args {
            serde_json::Value::Object(map) => Some(map),
            serde_json::Value::Null => None,
            other => {
                return ToolResult::error(format!(
                    "MCP tool arguments must be a JSON object, got: {}",
                    other
                ));
            }
        };

        let service = self.service.inner.lock().await;

        let result = service
            .call_tool(rmcp::model::CallToolRequestParam {
                name: Cow::Owned(self.info.original_name.clone()),
                arguments,
            })
            .await;

        match result {
            Ok(call_result) => {
                let text_parts: Vec<String> = call_result
                    .content
                    .iter()
                    .filter_map(|c| c.raw.as_text().map(|t| t.text.clone()))
                    .collect();

                let content = if text_parts.is_empty() {
                    serde_json::to_string(&call_result.structured_content)
                        .unwrap_or_else(|_| "(no text content)".to_string())
                } else {
                    text_parts.join("\n")
                };

                if call_result.is_error.unwrap_or(false) {
                    ToolResult::error(content)
                } else {
                    ToolResult::ok(content)
                }
            }
            Err(e) => ToolResult::error(format!("MCP call_tool failed: {}", e)),
        }
    }
}

// ─── McpManager — Manages MCP server connections ───────────────────

pub struct McpManager {
    servers: Vec<(McpServerConfig, Arc<McpServiceHandle>)>,
}

impl McpManager {
    pub fn new() -> Self {
        Self { servers: Vec::new() }
    }

    /// Connect to all configured MCP servers, discover tools, and return McpTool instances.
    pub async fn connect_all(
        configs: &[McpServerConfig],
    ) -> Result<(Self, Vec<Box<dyn Tool>>), String> {
        let mut manager = Self::new();
        let mut tools = Vec::new();

        for cfg in configs {
            match Self::connect_server(cfg).await {
                Ok((handle, discovered)) => {
                    let server_name = cfg.name.clone();
                    tracing::info!(
                        "MCP server '{}' connected, {} tools discovered",
                        server_name,
                        discovered.len()
                    );

                    for dt in discovered {
                        let prefixed_name = format!("mcp__{}__{}", server_name, dt.original_name);
                        tools.push(Box::new(McpTool {
                            prefixed_name,
                            info: dt,
                            service: handle.clone(),
                        }) as Box<dyn Tool>);
                    }

                    manager.servers.push((cfg.clone(), handle));
                }
                Err(e) => {
                    tracing::error!("Failed to connect MCP server '{}': {}", cfg.name, e);
                    return Err(format!(
                        "MCP server '{}' connection failed: {}",
                        cfg.name, e
                    ));
                }
            }
        }

        Ok((manager, tools))
    }

    async fn connect_server(
        cfg: &McpServerConfig,
    ) -> Result<(Arc<McpServiceHandle>, Vec<DiscoveredTool>), String> {
        let mut cmd = tokio::process::Command::new(&cfg.command);

        cmd.args(&cfg.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit());

        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }

        let transport = rmcp::transport::child_process::TokioChildProcess::new(cmd)
            .map_err(|e| format!("spawn '{}' failed: {}", cfg.command, e))?;

        let service: rmcp::service::RunningService<rmcp::RoleClient, ()> =
            ().serve(transport).await.map_err(|e| {
            format!(
                "initialize MCP connection to '{}' failed: {}",
                cfg.name, e
            )
        })?;

        let mcp_tools = service
            .list_all_tools()
            .await
            .map_err(|e| format!("list_tools from '{}' failed: {}", cfg.name, e))?;

        let discovered = mcp_tools
            .into_iter()
            .map(|t| DiscoveredTool {
                original_name: t.name.to_string(),
                description: t
                    .description
                    .map(|d: Cow<'static, str>| d.to_string())
                    .unwrap_or_default(),
                input_schema: serde_json::Value::Object(
                    (*t.input_schema).clone(),
                ),
            })
            .collect();

        Ok((Arc::new(McpServiceHandle {
            inner: Mutex::new(service),
        }), discovered))
    }
}
