# Wem Agent 架构设计 v5（增量修订）

## 1. 定位

Agent 是 wem-kernel 的一个**平级子系统**，与 block_system 并列。

```
wem-kernel
├── block_system/   ← 已有：Block 操作引擎
├── api/            ← 已有：HTTP handler + 请求/响应 DTO
├── repo/           ← 已有：持久化层
└── agent/          ← 新增：AI Agent 子系统
```

Agent 通过 block_system 的 **service 公共 API** 操作 Block，
不直接访问 repo。这和 handler/cli 一样的消费关系。

## 2. 整体架构

```
                         用户（终端 / 前端 / IM 通道）
                                    │
                                    ▼
                    ┌───────────────────────────────┐
                    │         Transport Layer        │
                    │  SSE (前端) │ CLI │ 未来: IM   │
                    └───────────────┬───────────────┘
                                    │
                                    ▼
                    ┌───────────────────────────────┐
                    │     Task Context Manager      │
                    │  任务上下文 / 生命周期 / 持久化  │
                    └───────────────┬───────────────┘
                                    │
                          ┌─────────┴─────────┐
                          ▼                   ▼
              ┌─────────────────┐ ┌──────────────────┐
              │  Agentic Loop   │ │  Permission Gate  │
              │  (编排核心)      │ │  (权限拦截)        │
              └────────┬────────┘ └────────┬─────────┘
                       │                   │
              ┌────────┴────────┐    ┌─────┴──────┐
              ▼                 ▼    ▼            ▼
      ┌──────────────┐ ┌────────────┐ ┌────────────────┐
      │   Provider   │ │   Tools    │ │ Context Manager │
      │ (LLM 接入)   │ │ (能力集)   │ │ (上下文管理)     │
      └──────┬───────┘ └─────┬──────┘ └────────────────┘
             │               │
    ┌────────┼───┐     ┌─────┼──────────┐
    ▼        ▼   ▼     ▼     ▼          ▼
 Claude   GPT  Ollama  Block  File     Shell
                      Ops    Ops      Ops
                       │
                       ▼
              ┌──────────────────┐
              │  block_system    │
              │  service 层      │
              └──────────────────┘
```

## 3. 八大模块的职责与边界

### 3.1 Provider（LLM 接入层）

**职责**：把不同 LLM API 统一成一个接口。

**边界**：
- 入：消息列表 + 工具定义 + 生成参数
- 出：流式事件流
- 不感知 Agent 的 loop 逻辑、权限、上下文管理

**实现形态**：一个 trait + 多个实现

| 实现 | 协议 | 说明 |
|------|------|------|
| AnthropicProvider | Messages API | Claude 全系列 |
| OpenAIProvider | Chat Completions API | GPT 系列 |
| OllamaProvider | OpenAI 兼容 | 本地模型 |

**crate 选型决策：不使用 rig-core，自建 Provider trait**

调研了 rig-core（Rust 最成熟的 LLM 框架，20+ Provider、tool calling、streaming、agent builder）。
结论是 **自建**，原因：

| 维度 | rig-core | Wem 自建 |
|------|----------|---------|
| Provider 抽象 | 通用但耦合 rig 生态 | 完全贴合 Wem 的 StreamEvent 契约 |
| Tool 定义 | 框架级 trait，约束强 | Wem 四维定义（name/desc/schema/prompt），prompt 维度 rig 不支持 |
| Agent 构造 | Builder pattern，但限制行为 | Wem Task Context Manager 需要更细粒度的生命周期控制（原 Session，见附录 C） |
| SSE 推送 | rig 无此概念 | Wem 双通道 SSE 是核心设计 |
| 依赖重量 | rig-core + rig-sqlite + 各 provider crate | 仅 reqwest + serde + tokio |
| 维护风险 | 第三方，版本节奏不受控 | 自主可控 |

rig 值得借鉴的设计模式（已吸收到本架构）：
- **Provider trait 的 companion crate 模式**：Provider trait 在 agent::provider::mod.rs 定义，各实现在独立文件（anthropic.rs / openai.rs），方便按需编译
- **Agent Builder 模式**：Task 创建时的配置组装（原 Session 创建，见附录 C）
- **JSON Schema 工具定义**：Tool 的 input_schema 用 schemars crate 自动生成，避免手写

**流式事件契约**（所有 Provider 统一输出）：

```rust
// agent/provider/mod.rs
use futures::Stream; // 使用 futures crate 的 Stream trait

pub enum StreamEvent {
    TextDelta { text: String },
    ToolCallBegin { id: String, name: String },
    ToolCallDelta { id: String, args_json: String },
    ToolCallEnd { id: String },
    Done { usage: TokenUsage },
}

pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

pub enum Role { System, User, Assistant }

pub enum ContentBlock {
    Text(String),
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}

pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value, // JSON Schema
}

#[async_trait]
pub trait Provider: Send + Sync {
    /// 流式调用，返回 StreamEvent 流
    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolDef],
        temperature: f32,
        model: Option<&str>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>>;

    /// 非流式调用（Context Manager 压缩摘要时用，要求快且便宜）
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        temperature: f32,
        model: Option<&str>,
    ) -> Result<String>;
}
```

这个契约是 Loop、SSE、前端三方的协议基础。
Provider 实现内部用 `eventsource-stream` crate 解析 SSE 文本流，
转换为此枚举流。

### 3.2 Tools（能力层）

**职责**：把 Agent 能做的事情抽象为"工具"，每个工具有标准化的描述和执行接口。

**边界**：
- 入：工具名 + JSON 参数 + 执行上下文（Db、工作目录、会话 ID）
- 出：文本结果 + 是否出错
- 不感知 LLM 调用、权限判断、上下文管理

**工具定义的四个维度**（学习自 Claude Code）：

| 维度 | 说明 | 用途 |
|------|------|------|
| `name` | 工具名 | LLM 调用时指定 |
| `description` | 功能描述 | LLM 理解工具能力 |
| `input_schema` | JSON Schema 参数定义 | LLM 构造调用参数 |
| `prompt` | 详细使用指南 | 注入 System Prompt，指导 LLM 如何正确使用 |

第四个维度 `prompt` 是关键——Claude Code 证明 LLM 不能仅靠 JSON Schema
正确使用工具，需要详细的使用指南（何时用、怎么组合、常见陷阱）。

**工具分组**：

| 组 | 工具数 | 说明 |
|----|--------|------|
| BlockOps | ~12 | 映射到 block_system::service 的公共 API |
| DocumentOps | ~5 | 文档级操作（创建/导出/导入/移动） |
| FileOps | ~5 | 文件系统（读/写/编辑/搜索/匹配） |
| ShellOps | ~1 | Shell 命令执行（白名单模式） |
| TaskOps | ~1 | 子 Agent 派生 |

**注册机制**：ToolRegistry 在启动时注册所有工具，
Agent 可按配置过滤（如"只读 Agent"只能用 BlockOps 的读操作 + FileOps 的读操作）。

**Tool trait Rust 定义**：

```rust
// agent/tools/mod.rs
use serde_json::Value;
use async_trait::async_trait;

/// 工具执行上下文，由 Agentic Loop 在调用时注入
pub struct ToolContext {
    pub db: sqlx::SqlitePool,       // 共享的数据库连接池
    pub working_dir: PathBuf,       // 当前工作目录
    pub session_id: String,         // 会话 ID
    pub event_tx: mpsc::Sender<AgentEvent>, // Agent SSE 事件发送端
}

pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;      // JSON Schema，可用 schemars 自动生成
    fn prompt(&self) -> &str;             // 注入 System Prompt 的使用指南
    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult;
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}
```

**input_schema 生成策略**：
每个 Tool 的参数用 Rust struct + `#[derive(JsonSchema)]`（schemars crate），
`input_schema()` 返回自动生成的 JSON Schema。
LLM 收到的是标准 JSON Schema tool definition，不暴露 Rust 实现细节。

### 3.3 Permission Gate（权限层）

**职责**：在工具执行前拦截，决定 auto / ask / deny。

**边界**：
- 入：工具名 + 参数
- 出：Auto（放行）/ Ask（暂停等用户）/ Deny（拒绝）
- 不执行工具本身，只做判断

**规则来源（优先级从高到低）**：
1. 本次会话的用户临时授权（approve 一次后同类操作自动放行）
2. 项目级配置 `.wem/agent.toml` 的 permission_overrides
3. 工具自身声明的默认权限级别

**Ask 状态的处理**：
权限返回 Ask 时，Agent Loop 挂起，通过 tokio channel 通知前端。
前端调 approve/deny API 后，Loop 恢复。

```rust
// Ask 状态的实现：tokio::sync::oneshot channel
//
// Permission Gate 返回 Ask 时，Loop 创建一个 oneshot channel，
// 将 sender 存入 Session 的 pending_approval 字段，
// await receiver，Loop task 挂起（不占 CPU）。
//
// Handler 收到 approve/deny API 请求时：
//   取出 sender → send(true) 或 send(false)
//   Loop 的 receiver.await 恢复，继续执行或返回 deny result
//
// 这比轮询高效：tokio 的 oneshot 是零成本抽象，挂起时不消耗线程。
pub struct PendingApproval {
    pub tool_name: String,
    pub args: serde_json::Value,
    pub tx: oneshot::Sender<bool>, // true=approve, false=deny
}
```

### 3.4 Prompt Assembly（提示组装）← v3 新增

**职责**：将 System Prompt 模块化拼接为最终发送给 LLM 的完整指令。

**边界**：
- 入：项目配置 + 工具列表 + 记忆索引 + 环境信息
- 出：拼接后的 system prompt 字符串
- 不感知 LLM 调用、工具执行

**Prompt 拼接顺序**：

```
最终 System Prompt
    │
    ├── 1. 基础角色指令
    │   └── "你是 Wem Agent，一个知识管理助手..."
    │
    ├── 2. 工具使用指南（每个工具的 prompt 字段）
    │   ├── BlockOps 工具使用规则
    │   ├── FileOps 工具使用规则
    │   └── ...
    │
    ├── 3. 项目上下文
    │   ├── .wem/CLAUDE.md 内容（项目规范、架构说明）
    │   └── block_system 当前状态摘要
    │
    ├── 4. 记忆索引
    │   └── MEMORY.md 内容
    │
    └── 5. 环境信息
        ├── 工作目录
        ├── 当前日期时间
        └── 可用文档列表
```

这个模块的学习来自 Claude Code：System Prompt 不是固定文本，
而是每次对话动态拼接的。工具的 prompt 占 System Prompt 最大比例。

**Token 预算控制**：

Prompt Assembly 拼接后必须检查总 token 数。
目标：System Prompt 占模型上下文窗口的 **不超过 25%**（保留 75% 给对话）。

```
预算分配（以 200K 上下文为例，System Prompt 上限 50K token）:
  ├── 1. 基础角色指令        ~500 token    ← 不可截断
  ├── 2. 工具使用指南       ~8,000 token   ← 按 Agent 配置的工具集裁剪
  ├── 3. 项目上下文         ~2,000 token   ← CLAUDE.md 超限则截断尾部
  ├── 4. 记忆索引           ~1,000 token   ← MEMORY.md 超限则截断尾部
  └── 5. 环境信息             ~200 token   ← 不可截断
```

超限时的截断优先级（从先截断到后截断）：
1. 记忆索引（最不重要）
2. 项目上下文（可截断尾部）
3. 工具使用指南（可裁剪低优先级工具的 prompt）
4. 基础角色指令 / 环境信息（不可截断）

### 3.5 Agentic Loop（编排核心）

**职责**：驱动"LLM 思考 → 工具调用 → 结果反馈 → 再思考"的循环。

**边界**：
- 不直接调 HTTP API（通过 Provider trait）
- 不直接操作文件/Block（通过 Tool trait）
- 不做权限判断（委托 Permission Gate）
- 不管理上下文窗口大小（委托 Context Manager）
- 不组装 prompt（委托 Prompt Assembly）

**Loop 状态机**：

```
                    ┌──────────┐
                    │  Idle    │ ← 等待用户输入
                    └────┬─────┘
                         │ 用户发消息
                         ▼
                    ┌──────────┐
              ┌─────│ Thinking │ ← 调用 LLM
              │     └────┬─────┘
              │          │ 收到响应
              │          ▼
              │     ┌──────────┐
              │     │ Has Tools?│
              │     └──┬────┬──┘
              │   No    │    │ Yes
              │         │    ▼
              │         │  ┌────────────┐
              │         │  │ Permis-    │
              │         │  │ sion Check │
              │         │  └──┬────┬────┘
              │         │ Auto│    │ Ask
              │         │     │    ▼
              │         │     │  ┌──────────┐
              │         │     │  │ Waiting  │ ← 等用户审批
              │         │     │  │ Approval │
              │         │     │  └──┬──┬────┘
              │         │     │ Deny│  │ Approve
              │         │     │     │  │
              │         │     ▼     ▼  ▼
              │         │  ┌──────────────┐
              │         │  │ Exec Tool    │
              │         │  └──────┬───────┘
              │         │         │ 结果返回
              │         │         ▼
              │         │    回到 Thinking（带工具结果）
              │         ▼
              │    ┌──────────┐
              │    │ Respond  │ ← 输出文本给用户
              │    └────┬─────┘
              │         │
              └─────────┘ 回到 Idle
```

**关键设计决策**：

1. **流式优先**：Loop 调 Provider 的流式接口，逐字推给前端
2. **并行 tool_calls**：同一批次的 tool_calls 用 `tokio::join!` 并行执行（LLM 经常同时请求多个读操作）
3. **步数上限**：max_steps 默认 50，防止无限循环烧 token
4. **错误不中断**：单个工具失败记为 error result 喂回 LLM，让 LLM 自行调整策略
5. **取消支持**：Loop 接收 `tokio::sync::watch` 或 `CancellationToken`，用户 abort 时立即终止 Provider 调用和工具执行

**Agentic Loop 核心伪代码（Rust）**：

```rust
// agent/loop.rs
pub struct AgentLoop {
    provider: Box<dyn Provider>,
    tools: Arc<ToolRegistry>,
    permission: PermissionGate,
    prompt_assembly: PromptAssembly,
    context_manager: ContextManager,
    session: Session,
    event_tx: broadcast::Sender<AgentEvent>,  // Agent SSE 广播
    cancel: CancellationToken,
}

impl AgentLoop {
    pub async fn run(&mut self, user_msg: String) -> Result<()> {
        self.session.push_message(Role::User, user_msg);
        self.session.set_state(SessionState::Running);

        for step in 0..self.session.config.max_steps {
            // 1. Context Manager 检查并压缩
            self.context_manager.maybe_compress(&mut self.session).await?;

            // 2. Prompt Assembly 拼装 system prompt
            let system = self.prompt_assembly.build(&self.session).await?;

            // 3. 调用 Provider，收集流式响应
            let stream = self.provider.stream(
                &system,
                &self.session.messages,
                &self.tools.tool_defs(&self.session.config.allowed_tools),
                self.session.config.temperature,
                Some(self.session.config.model.as_str()),
            ).await?;

            // 4. 处理流式事件
            let tool_calls = self.consume_stream(stream).await?; // 也推 SSE 事件

            // 5. 无工具调用 → 完成
            if tool_calls.is_empty() {
                self.session.set_state(SessionState::Completed);
                self.emit(AgentEvent::Done).await?;
                return Ok(());
            }

            // 6. 执行工具（当前串行；后续可升级为受控并行）
            let results = self.execute_tools(tool_calls).await?;

            // 7. 工具结果喂回消息列表
            for result in results {
                self.session.push_tool_result(result);
            }
        }

        // 超出步数上限
        self.session.push_message(Role::Assistant,
            "达到最大步数限制，停止执行。".into());
        self.session.set_state(SessionState::Completed);
        Ok(())
    }
}
```

### 3.6 Context Manager（上下文管理）

**职责**：确保消息列表不超出模型的上下文窗口。

**边界**：
- 入：当前消息列表 + 模型上下文大小
- 出：压缩后的消息列表
- 不感知 LLM 调用、工具执行

**压缩策略（参考 Claude Code）**：

```
压缩前:
[System, User1, Asst1, Tool1, Result1, User2, Asst2, ..., UserN, AsstN]

压缩后:
[System, Summary("之前对话的摘要..."), UserN, AsstN]
  ↑ 保留        ↑ 替换中间部分                    ↑ 保留最近几轮
```

- **触发条件**：估算 token > 上下文窗口 * 0.9
- **摘要生成**：调 Provider 的 complete 方法（非流式，用便宜模型如 claude-haiku-4-5）
- **保底策略**：如果摘要也超限，直接丢弃最早的消息

**Token 估算策略**：

```rust
// 粗估：1 token ≈ 4 字符（英文）或 1.5 字符（中文）
// 精确估算在 Rust 端成本过高（需要 tokenizer 模型），
// 采用保守的字符数 / 3 作为上界，实际 token 数通常更少。
fn estimate_tokens(text: &str) -> u32 {
    let ascii_ratio = text.chars().filter(|c| c.is_ascii()).count() as f32
        / text.len().max(1) as f32;
    if ascii_ratio > 0.8 {
        (text.len() as f32 / 4.0) as u32 // 英文为主
    } else {
        (text.len() as f32 / 1.5) as u32 // 中文为主
    }
}
```

### 3.7 Memory System（记忆系统）← v3 新增

**职责**：跨会话持久化 Agent 对用户、项目、行为的认知。

**边界**：
- 入：对话中学习到的信息 + 用户明确指令
- 出：记忆文件 + MEMORY.md 索引
- 不感知 LLM 调用、工具执行

**学习自 Claude Code 的三层记忆**：

```
.wem/memory/
├── MEMORY.md          ← 索引文件，每次对话自动加载到 Prompt
├── user.md            ← 用户画像（角色、偏好、技能水平）
├── feedback.md        ← 行为反馈（"不要 X"，"继续 Y"）
├── project.md         ← 项目上下文（里程碑、决策、约束）
└── reference.md       ← 外部资源指针（文档链接、API 地址）
```

**MEMORY.md 是索引**，不是记忆本身。它被自动加载到 System Prompt 中，
让 Agent 知道有哪些记忆文件可供按需读取。实际内容在各主题文件中。

**记忆写入时机**：

| 触发 | 写入目标 | 示例 |
|------|---------|------|
| 用户说"记住 X" | 对应主题文件 | "记住我喜欢用 tab 缩进" → user.md |
| 用户纠正行为 | feedback.md | "不要自动加注释" → feedback.md |
| Agent 发现项目约束 | project.md | 发现项目用 Tokio runtime → project.md |
| 用户提到外部资源 | reference.md | "CI 在 Jenkins 上" → reference.md |

**写入机制**（学习自 Claude Code）：

采用 **LLM 自主写入**模式：Memory 文件目录是 FileOps 的合法写入范围，
LLM 通过基础角色的 System Prompt 被告知记忆文件的位置和格式规范。
LLM 在对话中判断需要记录信息时，主动调用 `file_write` 工具写入。

不做独立的后处理步骤（避免每轮额外消耗一次 LLM 调用）。
记忆文件的路径和格式在 Prompt Assembly 的基础角色指令中声明。

### 3.8 Session Manager（会话管理）

**职责**：管理 Agent 会话的生命周期。

**边界**：
- 创建/销毁/恢复会话
- 持久化消息历史到 SQLite（复用现有 repo 基础设施）
- 会话与 Agent Loop 的一对一绑定
- 不感知 LLM 调用细节

**会话状态**：

| 状态 | 说明 |
|------|------|
| `idle` | 等待用户输入 |
| `running` | Agent Loop 正在执行 |
| `waiting_approval` | 挂起等用户授权 |
| `completed` | 本轮对话结束 |
| `error` | 出错，可恢复 |

**Session Manager 的 Rust 数据结构（当前实现）**：

```rust
// agent/session.rs
pub struct SessionManager {
    sessions: DashMap<String, ActiveSession>, // 并发安全 HashMap（内存态）
}

struct ActiveSession {
    session: Arc<Mutex<Session>>,            // 会话元数据 + 消息历史
    event_tx: broadcast::Sender<AgentEvent>, // Agent SSE 广播端
    cancel: CancellationToken,              // 中止信号
}

pub struct Session {
    pub id: String,
    pub state: SessionState,
    pub messages: Vec<Message>,
    pub config: SessionConfig,
    pub pending_approval: Option<PendingApproval>,
}

pub struct SessionConfig {
    pub model: String,              // "anthropic/claude-sonnet-4-6"
    pub temperature: f32,
    pub max_steps: u32,
    pub allowed_tools: Vec<String>, // 该会话允许使用的工具名列表
    pub system_prompt_override: Option<String>, // 子 Agent 的 prompt 覆盖
    pub working_dir: PathBuf,
}
```

> 备注：SQLite 持久化与会话恢复是 v5 下一阶段能力，当前是内存会话管理。

**Session 创建借鉴 rig 的 Agent Builder 模式**：

```rust
// Builder 模式组装 Session，避免构造函数参数爆炸
let session = SessionBuilder::new()
    .model("anthropic/claude-sonnet-4-6")
    .temperature(0.3)
    .max_steps(50)
    .allowed_tools(vec!["block_read", "file_read"])
    .system_prompt_override(Some("你是代码审查员".into()))
    .build();
```

## 4. 模块间依赖规则

```
                 Session Manager
                      │
                      ▼
                 Agentic Loop ───────────────────┐
                   │   │   │   │                  │
          ┌────────┘   │   │   └──────┐           │
          ▼            │   │          ▼           ▼
     Provider          │   │    Permission     Prompt
       │               │   │       Gate      Assembly
       │          ┌────┘   │                     │
       │          ▼        ▼                     ▼
       │       Tools   Context               Memory
       │          │      Manager            System
       │          │
       │          ▼
       │    block_system
       │    service 层
       │          │
       │          ▼
       │      repo 层
       │
       ▼
  LLM APIs
```

**硬约束**：

| 模块 | 可以依赖 | 不可以依赖 |
|------|---------|-----------|
| Provider | reqwest, serde | Agent 其他模块 |
| Tools | block_system::service, std::fs | Provider, ContextManager |
| Permission Gate | 无外部依赖 | Tools, Provider |
| Prompt Assembly | Tools（读 prompt）, Memory（读索引） | Provider, ContextManager, Agentic Loop |
| Agentic Loop | Provider, Tools, Permission, Context, PromptAssembly | block_system, repo |
| Context Manager | Provider（调摘要） | Tools, Permission, PromptAssembly |
| Memory System | std::fs | Provider, Tools, Permission |
| Session Manager | Agentic Loop | Provider, Tools |

核心原则：**Agentic Loop 是唯一知道所有模块的编排者**，
其他模块互相不可见。

## 5. 数据流

### 5.1 一次完整的用户请求

```
用户: "帮我把今天的会议纪要整理成一个文档"
 │
 ▼
[Transport] 收到消息，转发给 Session Manager
 │
 ▼
[Session Manager] 找到/创建会话，调用 Agent Loop
 │
 ▼
[Prompt Assembly] 组装 System Prompt
 ├─ 基础角色 + 工具指南 + 项目上下文 + 记忆索引 + 环境信息
 │
 ▼
[Context Manager] 检查上下文，无需压缩
 │
 ▼
[Agent Loop - Thinking]
 组装 system prompt + 消息历史 + 工具定义
 调用 Provider.stream()
 │
 ▼
[Provider] 返回流式事件
 ├─ TextDelta: "我来帮你整理会议纪要..."
 ├─ ToolCallBegin: document_create
 ├─ ToolCallDelta: {"title":"会议纪要 2026-04-21",...}
 ├─ ToolCallEnd
 │
 ▼
[Agent Loop - Tool Execution]
 对每个 ToolCall:
   → [Permission Gate] 检查权限 → Auto
   → [Tool - DocumentOps] 调 block_system::service::document
   → 结果返回
 │
 ▼
[Agent Loop - 第二轮 Thinking]
 工具结果喂回 LLM
 LLM: "文档已创建，内容已填充。"
 │
 ▼
[Session Manager] 保存消息历史
 │
 ▼
[Transport] SSE 推送完成事件给前端
```

### 5.2 权限拦截流

```
用户: "删除所有测试文件"
 │
 ▼
[Agent Loop - Thinking]
 LLM 请求 ToolCall: shell_exec("rm -rf tests/*")
 │
 ▼
[Permission Gate]
 规则匹配: shell_exec → Ask
 │
 ▼
[Agent Loop] 状态切换: waiting_approval
 │
 ▼
[Transport] SSE 推送 PermissionRequired 事件
 前端弹窗: "Agent 请求执行: rm -rf tests/*"
 │
 ├─ 用户点"允许" → approve API → Session Manager 通知 Loop
 │   → Permission Gate 记录授权 → Loop 恢复执行
 │
 └─ 用户点"拒绝" → deny API → Session Manager 通知 Loop
     → ToolResult(error: "User denied") → 喂回 LLM
     → LLM 调整策略
```

## 6. 与现有 block_system 的集成

### 6.1 Tool → Service 映射

| Tool (Agent 侧) | Service 函数 (block_system 侧) |
|------------------|-------------------------------|
| `block_create` | `service::block::create_block` |
| `block_get` | `service::block::get_block` |
| `block_update` | `service::block::update_block` |
| `block_delete` | `service::block::delete_block` |
| `block_move` | `service::block::move_block` |
| `block_export` | `service::block::export_block` |
| `document_create` | `service::document::create_document` |
| `document_get` | `service::document::get_document` |
| `document_export` | `service::document::export_text` |
| `document_import` | `service::document::import_text` |
| `document_delete` | `service::document::delete_document` |

### 6.2 事件复用与双通道协调

Agent 对 Block 的操作会触发 block_system 已有的 SSE 事件（create/update/delete）。
前端不需要区分"人操作"和"Agent 操作"——同样的 SSE 事件驱动同样的 UI 更新。

**双通道模型**：

```
前端同时监听两个独立的 SSE 通道：

通道 1: /api/v1/documents/{id}/events  （block_system 已有）
  → block_created / block_updated / block_deleted
  → 来源：人类操作 OR Agent 操作，前端不区分

通道 2: /api/v1/agent/sessions/{id}/chat  （Agent 当前实现）
    → POST 请求直接返回 SSE 响应流
    → Agent 自身状态事件（思考过程、工具调用、权限请求等）
    → 每次 chat 请求独立消费该轮事件
```

**为什么分开**：block_system SSE 是所有消费者共享的（人类、Agent、未来其他触发器），
Agent SSE 只关注 Agent 的编排状态。两者职责不同，不应合并。
前端各听各的，block SSE 驱动 UI 更新，Agent SSE 驱动 Agent 面板更新。

> 规划：独立 `events` 订阅路由可作为增强项补充，用于会话级持续监听和断线重连回放。

Agent SSE 事件类型：

| 事件类型 | 说明 |
|---------|------|
| `text_delta` | Agent 思考文本增量 |
| `tool_call_begin` | 开始执行工具 |
| `tool_call_end` | 工具执行完成（含结果摘要） |
| `permission_required` | 需要用户授权（含工具名 + 参数） |
| `step_progress` | 当前步数 / 最大步数 |
| `phase_changed` | Loop 阶段变化（strong-typed phase） |
| `done` | 本轮对话结束 |
| `error` | 本轮对话失败 |

### 6.3 新增 API 路由

```
# Agent 会话管理
POST /api/v1/agent/sessions              创建会话
POST /api/v1/agent/sessions/list         列出会话
POST /api/v1/agent/sessions/{id}         销毁会话

# Agent 对话
POST /api/v1/agent/sessions/{id}/chat    发消息（SSE 响应流）
GET  /api/v1/agent/sessions/{id}/events  会话级事件订阅（SSE）
POST /api/v1/agent/sessions/{id}/abort   中止执行

# Agent 权限交互
POST /api/v1/agent/sessions/{id}/permission  授权决策（{ approved: bool }）

# Agent 健康检查
GET  /api/v1/agent/health                Agent 子系统健康
```

> 规划：后续可补充 REST 风格路由（GET/DELETE）。

`POST /chat` 请求体（当前实现）支持可选 `request_id` 字段：

```json
{
    "message": "请总结今天的变更",
    "request_id": "req-20260423-001"
}
```

幂等语义：
- 同一 session 内，若相同 `request_id` 仍在执行中，重试请求会复用同一轮事件流（不重复启动 Loop）。
- 若相同 `request_id` 已执行完成，再次提交会返回 `409 Conflict`。
- 未提供 `request_id` 时，行为保持原有语义（并发 chat 仍返回 `409 Conflict`）。

## 7. 子 Agent（Task Tool）

主 Agent 通过 `task_spawn` 工具派生子 Agent。

**学习自 Claude Code**：子 Agent 不是框架级抽象，
而是通过 **prompt 控制行为差异**，共享同一套 Loop 代码。

```
主 Agent
    │
    └─ task_spawn(prompt="你是代码审查员", tools=[block_get, file_read])
        │
        └─ 新 AgentLoop（独立上下文窗口，限定工具集，自定义 prompt）
            │
            └─ 完成后返回文本摘要给主 Agent
```

**与主 Agent 的关系**：
- 子 Agent 有独立的上下文窗口（不占主 Agent 的额度）
- 子 Agent 的工具集由主 Agent 在 task_spawn 时指定
- 子 Agent 完成后返回一个文本结果给主 Agent
- 子 Agent 的消息历史不合并到主 Agent
- 子 Agent 的 prompt 覆盖基础角色指令，其他模块（工具指南等）不变

## 8. 配置

项目级 `.wem/agent.toml`（不存在则用内置默认值）：

```toml
[agent]
model = "anthropic/claude-sonnet-4-6"
temperature = 0.3
max_steps = 50

[permissions]
# auto = 自动放行, ask = 需确认, deny = 禁止
file_read = "auto"
file_write = "ask"
file_edit = "ask"
shell_exec = "ask"
block_read = "auto"
block_write = "ask"

[providers.anthropic]
api_key_env = "ANTHROPIC_API_KEY"
base_url = "https://api.anthropic.com"

[providers.openai]
api_key_env = "OPENAI_API_KEY"

[providers.ollama]
base_url = "http://localhost:11434"
```

## 9. 错误处理策略

| 错误场景 | 处理方式 |
|---------|---------|
| Provider 网络超时 | 重试 3 次，间隔 1s/2s/4s，仍失败则报错给用户 |
| Provider 返回 rate limit | 指数退避重试，最多等 60s |
| Provider 返回 API 错误 | 将错误文本作为 ToolResult(error) 喂回 LLM |
| 工具执行失败 | ToolResult(is_error=true) 喂回 LLM，不中断 Loop |
| 权限被拒绝 | ToolResult("User denied") 喂回 LLM，LLM 自行调整 |
| 上下文超限 | Context Manager 自动压缩，压缩也失败则截断最早消息 |
| 用户中止（abort） | 通过 CancellationToken 取消正在执行的 Provider 调用和工具 |
| SSE 连接断开 | Agent Loop 继续执行，结果保存在 Session 中，重连后可获取 |

## 10. 并发模型

```
一个 Session 对应一个 AgentLoop，一次只处理一个用户请求。

并发场景:
- 多个 Session 并行（不同用户/不同任务）→ 各自独立的 AgentLoop
- 同一批次多个 tool_calls → 当前串行执行（稳定优先）
- SSE 推送 → 独立 task，不阻塞 AgentLoop

不可并发的场景:
- 同一个 Session 不允许并发 chat（返回 409 Conflict）

v5 迭代目标:
- 同一批次 tool_calls 引入受控并行（Semaphore 限流 + 可取消）
- 引入工具级超时和熔断，避免单工具拖垮整轮 Loop
```

## 11. Rust crate 依赖

每个模块对应的 Rust crate 依赖（仅列关键依赖，不列标准库）：

| 模块 | 关键 crate | 用途 |
|------|-----------|------|
| Provider | `reqwest`（HTTP）, `eventsource-stream`（SSE 解析）, `serde` / `serde_json` | API 调用 + 流式解析 |
| Tools | `schemars`（JSON Schema 生成）, `async-trait` | 工具参数 Schema 自动生成 |
| Permission Gate | `tokio::sync::oneshot` | Ask 状态的挂起/恢复通道 |
| Prompt Assembly | `serde_json`, `tokio::fs` | 读取配置文件、拼接 prompt |
| Agentic Loop | `tokio`（runtime/task/channel）, `tokio-util`（CancellationToken）, `futures::Stream` | 异步编排核心 |
| Context Manager | 依赖 Provider trait | 调用摘要生成 |
| Memory System | `tokio::fs`, `serde` | 文件读写 |
| Session Manager | `dashmap`（并发 HashMap）, `tokio::sync::broadcast` | 多会话并发管理 |
| SSE Handler | `axum`, `tokio-stream` | HTTP SSE 推送 |

**不引入的 crate 及原因**：

| crate | 不引入原因 |
|-------|-----------|
| `rig-core` | 框架级抽象过重，与 Wem 的 StreamEvent/tool 定义冲突 |
| `langchain-rust` | LangChain 的抽象层对 Wem 无价值，增加学习成本 |
| `tiktoken` / `tokenizers` | Token 精确计数需加载模型文件（~1MB），初期用字符估算即可 |
| `anyhow` | Wem 已有统一的错误类型体系，不需要 anyhow 的笼统错误 |

## 12. 模块目录结构

```
wem-kernel/src/agent/
├── mod.rs                  ← 模块入口，pub mod 声明
├── provider/
│   ├── mod.rs              ← Provider trait 定义
│   ├── anthropic.rs        ← Claude 实现
│   ├── openai.rs           ← GPT 实现（P4）
│   └── ollama.rs           ← 本地模型实现（P4）
├── tools/
│   ├── mod.rs              ← Tool trait + ToolRegistry
│   ├── block_ops.rs        ← Block CRUD 工具
│   ├── document_ops.rs     ← Document 级工具
│   ├── file_ops.rs         ← 文件系统工具
│   ├── shell_ops.rs        ← Shell 执行工具
│   └── task_ops.rs         ← 子 Agent 派生工具
├── permission.rs           ← Permission Gate
├── prompt.rs               ← Prompt Assembly
├── memory.rs               ← Memory System（文件读写）
├── context.rs              ← Context Manager（压缩/摘要）
├── loop.rs                 ← Agentic Loop（状态机 + 编排）
├── session.rs              ← Session Manager
└── handler.rs              ← Axum handler（API 路由）
```

## 13. 执行路线

| 阶段 | 目标 | 交付物 | 新增 crate |
|------|------|--------|-----------|
| **P0a** | 能调 API | Provider trait + Anthropic 流式实现（StreamEvent 枚举、SSE 解析） | reqwest, eventsource-stream |
| **P0b** | 能对话 | Agentic Loop 裸循环 + Session Manager（纯聊天，无工具，CLI 测试） | tokio-util (CancellationToken), dashmap |
| **P1** | 能操作 | Tool trait + BlockOps + DocumentOps + FileOps（通过 service 层操作 Block） | schemars, async-trait |
| **P2** | 能控制 | Permission Gate + Handler API + SSE（前端可接入） | axum (已有), tokio-stream |
| **P3** | 能记忆 | Prompt Assembly + Memory System + Context Manager（压缩） | 无新依赖 |
| **P4** | 能扩展 | OpenAI/Ollama Provider + TaskOps（子 Agent）+ ShellOps + 配置文件 | 无新依赖 |
| **P5** | 工程化 | 受控并行工具执行 + 会话持久化 + 独立 events 订阅 + 观测指标 | 无新依赖 |
| **P6** | 代理学习 | 对外 LLM 代理转发 + 对话问题采集 + 学习记忆入库 | 无新依赖 |
| **P7** | 任务上下文 | 移除 Session → TaskContext 为顶层单元 + 多任务 Freeze/Thaw + 边界检测 | 无新依赖 |

P0a 结束：单元测试可调 Anthropic API、解析流式响应。
P0b 结束：终端里可以和 Agent 纯聊天（不操作 Block）。
P1 结束：Agent 可以创建/读取/编辑 Block。
P2 结束：前端可以通过 HTTP API + SSE 使用 Agent。
P3 结束：Agent 有跨会话记忆，上下文自动管理。
P4 结束：完整功能，多模型、子 Agent、Shell。
P5 结束：系统具备高并发稳定性、可恢复性与可观测性。
P6 结束：可在用户授权下对编码对话进行学习复盘并沉淀成长记忆。
P7 结束：用户在单一会话窗口中处理多个任务时，Agent 自动识别任务边界、隔离上下文并无感切换，消除多窗口切换负担。

## 附录A. 下一轮迭代优先级（建议）

### 15.1 P5-A：协议与会话层完善（优先级最高）

1. 已完成：独立事件订阅接口 `GET /api/v1/agent/sessions/{id}/events`
2. 已完成：`chat` 增加 `request_id` 与幂等策略（防重放）
3. 增加会话快照持久化（消息、状态、pending approval）

### 15.2 P5-B：执行层稳定性

1. 工具调用受控并行：`Semaphore + JoinSet`
2. 单工具超时（如 30s）+ 每轮最大并行数（如 4）
3. 权限等待支持可配置超时（当前 120s 可配置化）

### 15.3 P5-C：可观测性与测试

1. 指标：每轮 token、工具耗时分布、审批等待时长、失败率
2. 测试：状态机迁移表驱动测试、权限超时测试、断流恢复测试
3. 日志：按 `session_id/request_id/tool_call_id` 结构化串联

### 15.4 P5-D：文档与前后端契约

1. 事件 schema 单独成文（包含 `phase_changed` 的枚举值）
2. 权限 API 明确：`permission` 单接口语义与错误码约定
3. 路由版本化计划：当前实现与 REST 风格目标双轨说明

## 附录B. 新增需求：AI 代理转发与学习记忆

本附录记录用户新增需求（2026-04-23）：

- Wem 可作为对外 LLM API 代理层，转发第三方模型服务请求与流式响应。
- 在用户明确授权前提下，采集用户与编码助手的对话过程用于学习复盘。
- 将编码过程中的问题、认知不足、待学习项沉淀到记忆库，形成成长型知识资产。

### B.1 合规与安全前提（硬约束）

1. 默认关闭采集，必须用户显式开启（opt-in）。
2. 采集范围、用途、保存时长必须可见且可审计。
3. 用户可随时暂停、导出、删除采集记录。
4. 对敏感信息（密钥、口令、令牌、隐私数据）做脱敏后再入库。
5. 遵守第三方 LLM API 的服务条款与数据合规要求。

### B.2 架构增量模块

1. Proxy Gateway：统一接收并转发第三方 LLM 请求（含 SSE 透传）。
2. Conversation Tap：在代理链路中旁路采集对话事件（不阻塞主响应）。
3. Insight Extractor：从对话中抽取问题点、误区、学习项。
4. Learning Memory Writer：将结构化洞察写入 memory 系统（按主题文件）。

### B.3 目标数据结构（建议）

```json
{
    "session_id": "...",
    "request_id": "...",
    "issue": "用户遇到的具体问题",
    "knowledge_gap": "暴露出的知识盲区",
    "learning_item": "建议学习的知识点",
    "evidence": "脱敏后的对话片段",
    "confidence": 0.0,
    "created_at": "2026-04-23T00:00:00Z"
}
```

### B.4 API 草案（建议）

```text
POST   /api/v1/agent/proxy/sessions                      创建代理会话
POST   /api/v1/agent/proxy/sessions/{id}/forward         转发 LLM 请求
POST   /api/v1/agent/proxy/sessions/{id}/consent         更新采集授权状态
GET    /api/v1/agent/proxy/sessions/{id}/insights        查询学习洞察
DELETE /api/v1/agent/proxy/sessions/{id}/records         删除采集记录
```

### B.5 非目标（明确排除）

1. 不做无授权的后台监听或静默采集。
2. 不记录未经脱敏的敏感信息。
3. 不将采集数据用于与用户授权范围无关的用途。

## 附录C. 架构重构：移除 Session，以 Task Context 为顶层概念

本附录记录架构决策（2026-04-24）：

**决策**：移除 Session（会话）概念，以 Task Context（任务上下文）作为用户交互和系统管理的唯一顶层单元。

**理由**：

1. Session 是一个**人为引入的中间层**——用户并不关心"创建会话"，只关心"我在做什么任务"。
2. 自动上下文切换需要多任务并行，而 Session 的设计是"一对一"（一个会话 = 一个上下文），强行在 Session 里嵌 Slot 是在修补一个不该存在的概念。
3. 去掉 Session 后，概念模型变成：**一个窗口 = 多个任务上下文**，简单直接。

### C.1 概念对照：Before → After

| Before（Session 模型） | After（Task Context 模型） |
|------------------------|--------------------------|
| 用户手动创建 Session | 用户直接发消息，系统自动创建 Task |
| 一个 Session = 一个上下文 | 一个窗口 = 多个 Task Context |
| 切换任务 = 切换 Session 窗口 | 切换任务 = 自动 Freeze/Thaw |
| SessionManager 管理多个 Session | TaskContextManager 管理多个 Task |
| `session_id` 贯穿所有 API | `task_id` 贯穿所有 API |

### C.2 核心概念

| 概念 | 说明 |
|------|------|
| **Task Context** | 唯一的顶层单元，持有消息历史、工具状态、工作目录、system prompt 变体。替代原 Session 的角色 |
| **Task Boundary Detection** | 通过用户输入语义分析，判断当前消息属于已有 Task 还是新任务 |
| **Context Freeze/Thaw** | 冻结 = 将 Task 的完整状态序列化；恢复 = 反序列化并注入运行时 |
| **Task Summary** | 每个 Task 维护一段短摘要（≤200 字），用于前端侧边栏预览和 Boundary Detection |

### C.3 Session → Task Context 迁移映射

| 原 Session 字段/能力 | 迁移到 Task Context | 备注 |
|----------------------|---------------------|------|
| `messages` | `TaskContext.messages` | 直接搬移 |
| `config`（model, temperature, max_steps） | `TaskContext.config` | 每个 Task 可独立配置 |
| `state`（Idle/Running/...） | `TaskContext.state` | 状态机不变 |
| `active_request_id` + `recent_request_ids` | `TaskContext.request_dedup` | 幂等去重逻辑不变 |
| `pending_approval` | `TaskContext.pending_approval` | 权限等待不变 |
| `CancellationToken` | `TaskContext.cancel` | 取消机制不变 |
| `broadcast::Sender<AgentEvent>` | `TaskContext.event_tx` | 事件广播不变 |
| `SessionManager`（DashMap） | `TaskContextManager`（DashMap） | 容器改名，结构不变 |

### C.4 架构增量模块

1. **Task Context Manager**：替代原 SessionManager，管理多个 Task 的生命周期（创建、冻结、恢复、归档、销毁）。
2. **Boundary Detector**：判断用户输入属于哪个已有 Task 或需要新建。
3. **Context Serializer**：将 Task 状态（消息历史、tool state、working set）序列化为可恢复的快照。
4. **Task Sidebar API**：向前端暴露当前 Task 列表、摘要、切换指令。

### C.5 数据结构（目标）

```rust
/// 任务上下文 — 唯一的顶层单元
struct TaskContext {
    id: String,
    title: String,                       // 自动生成或用户命名
    summary: String,                     // ≤200 字摘要，用于预览和匹配
    state: TaskState,                    // Idle / Running / WaitingApproval / Completed / Error
    messages: Vec<Message>,              // 该任务的完整消息历史
    config: TaskConfig,                  // model, temperature, max_steps, working_dir
    request_dedup: RequestDedup,         // 幂等去重（active + recent）
    pending_approval: Option<PendingApproval>,
    cancel: CancellationToken,
    event_tx: broadcast::Sender<AgentEvent>,
    created_at: String,
    last_active_at: String,
}

/// 任务状态
enum TaskState {
    Active,      // 当前正在使用的任务
    Frozen,      // 冻结等待恢复
    Completed,   // 任务完成
    Archived,    // 用户主动归档
}

/// 幂等去重（从 Session 中提取）
struct RequestDedup {
    active_request_id: Option<String>,
    recent_request_ids: VecDeque<String>,  // max 128
}

/// 任务配置（从 SessionConfig 提取）
struct TaskConfig {
    model: String,
    temperature: f32,
    max_steps: u32,
    allowed_tools: Vec<String>,
    system_prompt_override: Option<String>,
    working_dir: PathBuf,
}
```

### C.6 交互流程

```text
用户发消息
    │
    ▼
Boundary Detector 判断
    ├── 属于当前 Active Task → 正常处理
    ├── 属于已有 Frozen Task → Freeze 当前 → Thaw 目标 Task → 处理
    └── 全新任务 → Freeze 当前 → Create New Task → 处理
    │
    ▼
每个 Task 独立维护消息历史、幂等去重、token 预算
```

### C.7 API 草案（目标）

```text
POST   /api/v1/agent/tasks                              发送消息（自动路由到对应 Task）
GET    /api/v1/agent/tasks                              列出所有任务
GET    /api/v1/agent/tasks/{id}                         获取任务详情
PATCH  /api/v1/agent/tasks/{id}                         重命名/归档任务
POST   /api/v1/agent/tasks/{id}/activate                手动切换到指定任务
GET    /api/v1/agent/tasks/{id}/events                  SSE 订阅任务事件
POST   /api/v1/agent/tasks/{id}/permission              权限审批
DELETE /api/v1/agent/tasks/{id}                         销毁任务
```

> 注：自动切换是默认行为，手动 API 用于前端侧边栏覆盖。

### C.8 边界检测策略（渐进式）

| 阶段 | 策略 | 说明 |
|------|------|------|
| V1 | 显式命令 + 规则 | `/task 新任务` 显式创建 + 检测文档路径变化 |
| V2 | 关键词模式匹配 | "换个话题"、"回到之前的XX" + 工作目录切换信号 |
| V3 | 嵌入向量相似度 | 用户输入与各 Task Summary 语义相似度 |
| V4 | LLM 辅助分类 | 对模糊情况用轻量模型做二分类 |

### C.9 实施阶段

| 阶段 | 内容 | 影响 |
|------|------|------|
| **P7-A** | 重命名：Session → TaskContext，所有 API 路径 `/sessions/` → `/tasks/` | session.rs 重写，handler.rs 路由更新，前端对接更新 |
| **P7-B** | 多任务容器：TaskContextManager 支持多 Task 共存 + Freeze/Thaw | 核心逻辑新增 |
| **P7-C** | 显式切换：`/task` 命令 + 前端侧边栏 | 前端 + API 新增 |
| **P7-D** | 自动检测：Boundary Detector V1→V4 渐进 | 新增模块 |

### C.10 非目标（明确排除）

1. 不做跨用户的任务共享（单人场景）。
2. 不做自动任务优先级排序（由用户自己决定）。
3. 不做任务间的自动信息流转（需用户主动提及"参考之前 XX 任务的结果"）。

## 14. Rust 生态调研总结

本节记录调研 Rust AI Agent 生态后的决策依据。

### 调研项目清单

| 项目 | Stars | 特点 | 对 Wem 的价值 |
|------|-------|------|-------------|
| rig-core (0xPlaygrounds) | 4k+ | 最成熟的 Rust LLM 框架，20+ Provider，tool calling，streaming，agent builder，WASM 兼容 | Provider trait 设计参考，决定不依赖但借鉴模式 |
| swarms-rs | 1k+ | 企业级多 Agent 编排，near-zero latency | 多 Agent 路由模式参考（Wem 远期） |
| autogpt (Rust) | 500+ | 纯 Rust AI Agent 框架 | 简单直接，无特殊启发 |
| rswarm | 200+ | XML 定义 tool 的 Agent 式 LLM workflow | XML tool 定义方式不值得借鉴 |
| langchain-ai-rust | 300+ | LangChain 的 Rust 移植 | 抽象过重，不适合 Wem |
| AutoAgents (Rust) | 论文项目 | Benchmark: 4.97 rps vs Python avg 3.66 rps (36% faster) | 确认 Rust 在 Agent 领域的性能优势 |

### rig-core 的关键设计模式（已吸收）

1. **Provider trait + 独立实现文件**：每个 Provider 一个 .rs 文件，按需编译。Wem 采用同样结构。

2. **Agent Builder 模式**：`client.agent("model").preamble("...").build()` 的链式配置。
   Wem 的 SessionBuilder 借鉴此模式，但增加了 Wem 特有的 allowed_tools 和 system_prompt_override。

3. **JSON Schema 工具定义**：rig 用 serde + schemars 自动生成 JSON Schema。
   Wem 的 Tool trait 的 input_schema 也采用此方案，避免手写 Schema。

4. **stream + Pin<Box<dyn Stream>>**：rig 的 Provider 返回 pinned boxed stream。
   Wem 的 Provider trait 也采用此签名，这是 Rust 异步流的标准做法。

### rig-sqlite 的启发（远期）

rig-sqlite 提供了 SQLite 向量存储能力（用于 RAG）。
Wem 已使用 SQLite 作为主数据库，未来可以考虑：
- P5 阶段：在现有 SQLite 上加向量扩展，实现 Block 语义搜索
- 不需要引入新数据库，复用现有基础设施
