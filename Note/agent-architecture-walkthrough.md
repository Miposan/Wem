# Wem Agent 架构导读

> 目的：带你从头看懂 `wem-kernel/src/agent/` 的每一层设计。

---

## 一句话概括

Wem Agent 是一个 **LLM 驱动的自主循环**：用户发一条消息 → LLM 回复 → LLM 调用工具 → 工具结果反馈给 LLM → LLM 继续回复 → 直到任务完成或达到最大步数。

```
用户消息 → [LLM 思考 → 调用工具 → 获取结果] × N → 最终回复
```

---

## 分层总览

```
┌─────────────────────────────────────┐
│  handler.rs     HTTP 接口层         │  对接 Axum 路由
├─────────────────────────────────────┤
│  runtime.rs     运行时内核          │  不依赖 HTTP，CLI 也能用
├─────────────────────────────────────┤
│  loop_runner.rs 循环编排             │  "思考 → 调用 → 反馈" 的循环
├─────────────────────────────────────┤
│  session.rs     会话管理            │  多会话、持久化、事件广播
├─────────────────────────────────────┤
│  provider.rs    LLM 抽象层          │  Anthropic / OpenAI 统一接口
├─────────────────────────────────────┤
│  tools.rs       工具注册表          │  内置工具 + MCP 外部工具
├─────────────────────────────────────┤
│  prompt.rs      系统提示词组装       │  拼装 system prompt
├─────────────────────────────────────┤
│  context.rs     上下文窗口管理       │  超长对话自动压缩
├─────────────────────────────────────┤
│  permission.rs  权限控制            │  工具执行前检查：自动/询问/拒绝
└─────────────────────────────────────┘
```

---

## 一条消息的完整旅程

```
1. POST /api/v1/agent/sessions/{id}/chat  { message: "帮我读一下 config.toml" }
   │
   ▼
2. handler.rs :: chat()
   解析请求，调用 runtime.start_chat_stream()
   │
   ▼
3. runtime.rs :: AgentRuntime
   检查会话状态（不能重复提交）
   订阅事件流，启动 AgentLoop 任务
   返回 broadcast::Receiver<AgentEvent>
   │
   ▼
4. loop_runner.rs :: AgentLoop::run()
   ┌─────────── 循环开始 ───────────┐
   │                                 │
   │  a. prompt.rs 组装系统提示词     │
   │  b. context.rs 检查是否需要压缩  │
   │  c. provider.rs 发给 LLM        │
   │  d. 流式接收回复，emit TextDelta │
   │  e. LLM 要调工具？→ 收集 ToolCall│
   │  f. permission.rs 检查权限       │
   │  g. tools.rs 执行工具           │
   │  h. 工具结果 → 发回 LLM         │
   │  i. 回到 a. 继续循环            │
   │                                 │
   │  退出条件：无工具调用 / 达到最大步数 / 取消
   └─────────────────────────────────┘
   │
   ▼
5. emit Done 事件，会话状态 → Completed
   消息持久化到 SQLite
   │
   ▼
6. SSE 推送给客户端：TextDelta → ToolCallBegin → ToolCallEnd → Done
```

---

## 各模块详解

### provider.rs — LLM 抽象层

**做什么**：统一不同 LLM 厂商的 API 差异。

```rust
trait Provider {
    // 流式对话（agent 用这个）
    async fn stream(&self, system, messages, tools, temperature) -> StreamResult

    // 非流式（简单场景）
    async fn complete(&self, system, messages, temperature) -> String
}
```

**两个实现**：
- `anthropic.rs` — 直接调 Anthropic API，用 eventsource-stream 解析 SSE
- `openai_compatible.rs` — 通过 async-openai 库，兼容任何 OpenAI 格式的 API

**统一事件模型**（不管哪个 provider，上层看到的都一样）：
```rust
enum StreamEvent {
    TextDelta { text }          // 流式文本
    ToolCallBegin { id, name }  // LLM 决定调用工具
    ToolCallDelta { id, args }  // 工具参数（增量）
    ToolCallEnd { id }          // 工具调用结束
    Done { usage }              // 本轮回复完成
}
```

### tools.rs — 工具系统

**做什么**：定义工具接口，管理工具注册表。

```rust
trait Tool {
    fn name(&self) -> &str
    fn description(&self) -> &str
    fn input_schema(&self) -> serde_json::Value  // JSON Schema
    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult
}
```

**内置工具**（`tools/file_ops.rs`、`tools/shell_ops.rs`）：
- `file_read` — 读文件，支持分页
- `file_write` — 写文件，自动创建目录
- `file_edit` — 字符串替换编辑
- `shell_exec` — 执行 shell 命令，30s 超时保护

**外部工具**（`mcp.rs`）：
- 通过 MCP 协议连接外部工具服务器
- 配置在 `wem.toml` 的 `[agent.mcp_servers]` 里
- 工具名格式：`mcp__{服务器名}__{工具名}`

**注册表**（`ToolRegistry`）：
- 启动时注册所有工具（内置 + MCP）
- 生成 `ToolDef` 列表给 LLM（告诉它有哪些工具可用）
- 按会话的 `allowed_tools` 过滤

### session.rs — 会话管理

**做什么**：管理多个对话会话的生命周期。

```
会话状态流转：
Idle → Running → Completed
              → Error
              → WaitingApproval（等待用户批准工具执行）
```

**核心结构**：
- `SessionManager` — DashMap 并发存储，SQLite 持久化
- `ActiveSession` — 会话 + 取消令牌 + 事件广播通道
- `SessionConfig` — 模型、温度、最大步数、允许的工具列表

**幂等性**：
- 每个请求带 `request_id`
- 相同 request_id + 正在运行 → 返回已有流（重试安全）
- 不同 request_id + 正在运行 → 拒绝（并发保护）
- 已完成的 request_id → 拒绝（防重放）

### loop_runner.rs — 循环编排

**做什么**：Agent 的核心循环——"LLM 思考 → 调用工具 → 结果反馈"。

```rust
fn run() {
    for step in 0..max_steps {
        if cancelled → break

        // 1. 组装提示词 + 压缩上下文
        let system = build_prompt(tools, working_dir)
        compress_if_needed(messages)

        // 2. 流式调用 LLM
        let (text, tool_calls) = stream_model(provider, system, messages, tools)

        // 3. 记录助手消息
        messages.push(assistant_message)

        // 4. 没有工具调用 → 完成
        if tool_calls.is_empty() → break

        // 5. 执行工具（带权限检查）
        for tool_call in tool_calls {
            match permission.check(&tool_call) {
                Auto  → execute(tool_call)
                Ask   → 等用户批准（120s 超时）
                Deny  → 返回错误
            }
            messages.push(tool_result)
        }
    }
}
```

### permission.rs — 权限控制

**做什么**：工具执行前的安全门。

```rust
enum Permission {
    Auto,   // 自动允许
    Ask,    // 需要用户批准
    Deny,   // 自动拒绝
}
```

**流程**：
1. 查缓存（同一工具 + 相同参数，之前批准过就自动通过）
2. 缓存未命中 → 调用 `check(tool_name, args)`
3. 如果是 `Ask` → 设会话状态为 `WaitingApproval`，发 `PermissionRequired` 事件
4. 用户通过 `POST /permission { approved: true }` 回应
5. 批准后缓存结果，执行工具

### prompt.rs — 提示词组装

**做什么**：动态组装发给 LLM 的系统提示词。

```
最终 system prompt = 基础角色定义
                   + 工作目录 + 当前时间
                   + 所有可用工具的使用说明
                   + 行为规则
```

工具的使用说明来自每个 Tool 的 `prompt()` 方法，按 `allowed_tools` 过滤。

### context.rs — 上下文压缩

**做什么**：对话太长时自动压缩，避免超出 token 限制。

**策略**：
1. 估算当前 token 数（字符数 / 3 的启发式）
2. 超过阈值 → 保留最近 4 条消息，把更早的对话让 LLM 总结成一条
3. 总结失败 → 兜底截断

---

## SSE 事件类型

客户端通过 SSE 收到的事件：

| 事件 | 含义 |
|------|------|
| `TextDelta` | 流式文本片段 |
| `ToolCallBegin` | LLM 开始调用工具 |
| `ToolCallEnd` | 工具执行完毕 |
| `PermissionRequired` | 需要用户批准 |
| `StepProgress` | 当前循环进度 |
| `PhaseChanged` | 阶段变化 |
| `Done` | 会话完成 |
| `Error` | 出错 |

---

## 持久化

`repo/session_repo.rs` 管理 SQLite 存储：

- `agent_sessions` 表：会话配置（模型、温度、工具列表）
- `agent_messages` 表：消息历史（按 session_id + seq 排列）

每轮对话结束后，助手消息和工具结果都会写入 SQLite，下次打开同一个会话可以接着聊。

---

## 配置

`wem.toml` 中的 `[agent]` 部分：

```toml
[agent]
provider = "openai_compatible"        # 或 "anthropic"
base_url = "https://api.xxx.com/v1"
api_key = "your-key"
model = "glm-5.1"
max_tokens = 16384
temperature = 0.3
max_steps = 50                         # 最大循环步数

[agent.custom_headers]
User-Agent = "custom-header"           # 额外 HTTP 头

[[agent.mcp_servers]]                   # 外部工具服务器
name = "my-tools"
command = "node"
args = ["server.js"]
```

---

## API 端点一览

| 方法 | 路径 | 作用 |
|------|------|------|
| POST | `/agent/sessions` | 创建会话 |
| POST | `/agent/sessions/list` | 列出所有会话 |
| POST | `/agent/sessions/{id}/chat` | 发消息（返回 SSE 流） |
| GET | `/agent/sessions/{id}/events` | 持久订阅事件 |
| POST | `/agent/sessions/{id}/abort` | 中止运行中的会话 |
| POST | `/agent/sessions/{id}/permission` | 回应权限请求 |
| DELETE | `/agent/sessions/{id}` | 销毁会话 |

---

## 扩展点

- **新 Provider**：实现 `Provider` trait
- **新工具**：实现 `Tool` trait，在启动时注册
- **新传输层**：直接用 `AgentRuntime`，不依赖 HTTP
- **新权限规则**：扩展 `PermissionGate`
