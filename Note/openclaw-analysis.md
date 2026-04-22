# OpenClaw 架构分析

## 1. 项目概况

| 项 | 值 |
|---|---|
| 仓库 | [openclaw/openclaw](https://github.com/openclaw/openclaw) |
| 作者 | Peter Steinberger（奥地利） |
| 语言 | TypeScript，Node 24 |
| 协议 | MIT |
| Stars | 329,000+（2026 年 1 月发布，4 个月内） |
| 定位 | 个人 AI 助手（不是编程专用，是生活 + 工作全覆盖） |
| 吉祥物 | Molty（一只龙虾） |

## 2. 整体架构

OpenClaw 的核心是一个 **本地 Gateway 守护进程**，常驻运行，
对外连接各种消息通道，对内调度 Agent 和工具。

```
┌─────────────────────────────────────────────────────────────┐
│                       OpenClaw                               │
│                                                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │                   Channels (26个)                    │    │
│  │  WhatsApp │ Telegram │ Slack │ Discord │ WeChat │ QQ │    │
│  │  Signal │ iMessage │ Teams │ Matrix │ IRC │ WebChat │    │
│  │  LINE │ Feishu │ Nostr │ Twitch │ ...               │    │
│  └──────────────────────────┬──────────────────────────┘    │
│                             │                                │
│                             ▼                                │
│  ┌─────────────────────────────────────────────────────┐    │
│  │                  Gateway（控制平面）                   │    │
│  │                                                       │    │
│  │  ┌──────────┐  ┌───────────┐  ┌──────────────────┐  │    │
│  │  │ Session  │  │ Agent     │  │ Channel          │  │    │
│  │  │ Router   │  │ Router    │  │ Adapter          │  │    │
│  │  └──────────┘  └─────┬─────┘  └──────────────────┘  │    │
│  │                      │                                │    │
│  │  ┌───────────────────┴───────────────────────────┐  │    │
│  │  │                 Agent Runtime                  │  │    │
│  │  │  ┌──────────┐ ┌─────────┐ ┌───────────────┐  │  │    │
│  │  │  │ LLM Call │ │ Tools   │ │ Skills        │  │  │    │
│  │  │  │ Loop     │ │ Engine  │ │ Registry      │  │  │    │
│  │  │  └──────────┘ └────┬────┘ └───────────────┘  │  │    │
│  │  │                    │                          │  │    │
│  │  │  ┌─────────────────┴─────────────────────┐   │  │    │
│  │  │  │ Built-in Tools                        │   │  │    │
│  │  │  │ bash │ browser │ canvas │ cron │ nodes │   │  │    │
│  │  │  │ sessions │ discord │ slack │ gateway  │   │  │    │
│  │  │  └───────────────────────────────────────┘   │  │    │
│  │  └──────────────────────────────────────────────┘  │    │
│  │                                                      │    │
│  │  ┌──────────────────────────────────────────────┐  │    │
│  │  │ Model Layer                                  │  │    │
│  │  │ Claude │ GPT │ Gemini │ DeepSeek │ Ollama    │  │    │
│  │  │ 故障转移 + 轮换 + fallback                    │  │    │
│  │  └──────────────────────────────────────────────┘  │    │
│  └─────────────────────────────────────────────────────┘    │
│                                                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │ Companion Apps（可选）                                │    │
│  │ macOS 菜单栏 │ iOS App │ Android App                 │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

## 3. 六大核心子系统

### 3.1 Gateway（控制平面守护进程）

Gateway 是整个 OpenClaw 的中枢，所有组件都通过它通信。

**运行方式**：
- macOS: launchd user service（`openclaw onboard --install-daemon`）
- Linux: systemd user service
- Windows: WSL2 内运行
- 进程管理：`pnpm gateway:watch`（开发模式，热重载）

**核心职责**：

| 职责 | 说明 |
|------|------|
| 生命周期管理 | 启动/停止/重启，crash 自动恢复 |
| WebSocket Hub | 所有 Channel 和 Companion App 通过 WS 连接到 Gateway |
| 会话路由 | 收到的消息分发到正确的 Agent Session |
| 事件广播 | Agent 事件推送到所有订阅的客户端 |
| 配置热加载 | `openclaw.toml` 变更后自动 reload |

**本质**：一个 Node 进程，跑 Hono HTTP Server + WebSocket Server。

### 3.2 Channel Adapter（通道适配器层）

OpenClaw 最显著的特点：**26 个消息通道**，每个通道一个适配器。

**适配器的职责**：

```
外部通道 ←──→ Channel Adapter ←──→ Gateway
(WhatsApp)    (协议翻译)           (统一内部格式)
```

| 适配器做的事 | 说明 |
|-------------|------|
| 协议翻译 | 把 WhatsApp/Telegram/... 的消息格式转成 OpenClaw 内部的统一格式 |
| 双向通信 | 收消息（通道→Gateway）+ 发消息（Gateway→通道） |
| 媒体转换 | 图片/语音/文件 转成 Agent 可处理的格式 |
| DM 安全 | 未知发件人 → 配对码验证 → 审批后才处理 |
| 状态同步 | 在线/离线/打字中 状态上报 |

**通道分类**：

| 类型 | 通道 | 协议 |
|------|------|------|
| 即时通讯 | WhatsApp, Telegram, Signal, iMessage, LINE, WeChat, QQ | 各自 SDK/API |
| 团队协作 | Slack, Discord, Teams, Mattermost, Google Chat | Bot API |
| 社区 | IRC, Matrix, Nostr, Twitch | 开放协议 |
| 企业 | Feishu, Zalo, Synology Chat, Nextcloud Talk | 企业 API |
| 内置 | WebChat | WebSocket |

**安全默认值**：
- `dmPolicy="pairing"` — 未知发件人收配对码
- `openclaw pairing approve <channel> <code>` — 管理员审批
- `openclaw doctor` — 扫描风险配置

### 3.3 Agent Router（多 Agent 路由）

OpenClaw 支持多个独立 Agent 并行运行，不同通道可路由到不同 Agent。

**路由规则**：

```
收到的消息
    │
    ▼
Agent Router 根据以下条件选择目标 Agent:
    ├── 来源通道（WhatsApp → work agent, Discord → code agent）
    ├── 来源账户/对等端
    └── 手动 @mention（@reviewer 帮我看看这段代码）
    │
    ▼
目标 Agent Session
```

**Agent 隔离**：

| 维度 | 说明 |
|------|------|
| Workspace | 每个 Agent 有独立的 `~/.openclaw/workspace/` 子目录 |
| Session | 每个 Agent 维护独立的对话历史 |
| Tools | 不同 Agent 可配置不同的工具集 |
| Model | 不同 Agent 可用不同模型 |
| Prompt | 每个 Agent 有独立的 SKILL.md / SOUL.md |

### 3.4 Agent Runtime（Agent 运行时）

这是 OpenClaw 的 Agentic Loop，和 Claude Code 类似但更轻量。

**核心流程**：

```
用户消息
    │
    ▼
1. 组装 System Prompt
   ├── SOUL.md（Agent 性格定义）
   ├── TOOLS.md（工具使用指南）
   ├── AGENTS.md（Agent 配置）
   └── Skill prompts（相关 skill 的 SKILL.md）
    │
    ▼
2. 调用 LLM（流式）
    │
    ▼
3. 解析响应
   ├── 纯文本 → 直接回复
   └── Tool Calls → 执行工具 → 结果喂回 → 回到第 2 步
    │
    ▼
4. 回复用户（通过 Channel Adapter 发到原通道）
```

**内置工具清单**：

| 工具 | 说明 |
|------|------|
| `bash` | Shell 命令执行 |
| `browser` | Web 浏览器（Puppeteer） |
| `canvas` | 可视化工作区（A2UI） |
| `cron` | 定时任务管理 |
| `nodes` | 控制 companion app（iOS/Android） |
| `sessions_list` | 列出所有会话 |
| `sessions_history` | 查看其他会话历史 |
| `sessions_send` | 向其他会话发消息 |
| `sessions_spawn` | 创建新会话 |
| `discord` | Discord 特有操作（发消息到频道等） |
| `slack` | Slack 特有操作 |
| `gateway` | Gateway 控制（重启、配置等） |
| `read` | 文件读取 |
| `write` | 文件写入 |
| `edit` | 文件编辑 |

**与 Claude Code 工具系统的对比**：

| 维度 | Claude Code | OpenClaw |
|------|------------|----------|
| 工具定义方式 | TypeScript 类，硬编码 | TypeScript 类 + 配置文件 |
| 工具数量 | ~40 个（编码专用） | ~15 个（生活+通用） |
| 工具过滤 | 按 Agent 配置限制 | 按 Agent + Skill 组合限制 |
| MCP 支持 | 有 | 无（用 Skill 替代） |

### 3.5 Skills 系统

Skills 是 OpenClaw 的扩展机制，类似 Claude Code 的 Skill 但更灵活。

**定义方式**：一个 SKILL.md 文件 + 可选的附属文件。

```
~/.openclaw/workspace/skills/
├── email/
│   └── SKILL.md          # 技能描述 + 使用指南
├── calendar/
│   └── SKILL.md
├── code-review/
│   └── SKILL.md
└── cooking/
    └── SKILL.md
```

**SKILL.md 的结构**：

```markdown
---
description: "管理邮件收发"
tools: [bash, browser]
trigger: "当用户提到邮件、email、收件箱时"
---

# Email Skill

你可以通过 Gmail API 读取和发送邮件。

## 可用操作
- 读取未读邮件
- 搜索邮件
- 发送邮件
- 回复邮件

## 使用方式
...
```

**Skill 生命周期**：
1. Agent 启动时扫描 skills 目录
2. 匹配用户消息到相关 Skill（通过 trigger 或 LLM 自行判断）
3. 将匹配的 SKILL.md 内容注入 System Prompt
4. Agent 获得 Skill 声明的工具使用权
5. 执行完成后 Skill 上下文保留在会话中

**与 Claude Code Skills 的区别**：

| 维度 | Claude Code | OpenClaw |
|------|------------|----------|
| 定义格式 | YAML frontmatter + 代码 | Markdown + YAML frontmatter |
| 工具绑定 | Skill 内声明允许的工具 | Skill 内声明，Agent Router 执行 |
| 分发 | 本地文件 | ClawHub（在线注册表） |
| 复杂度 | 可含复杂逻辑代码 | 主要是 prompt + 简单脚本 |

### 3.6 Model Layer（模型层）

OpenClaw 的模型层设计为完全可替换，支持多模型故障转移。

**模型配置**：

```json
{
  "agent": {
    "model": "anthropic/claude-sonnet-4-6"
  }
}
```

**故障转移机制**：
- 配置多个 auth profile
- 主模型失败（rate limit / 超时 / 错误）自动切换到备用模型
- 支持按任务类型选模型（简单任务用 Haiku，复杂任务用 Opus）

**支持的模型提供商**：

| 提供商 | 模型 | 接入方式 |
|--------|------|---------|
| Anthropic | Claude 全系列 | Messages API |
| OpenAI | GPT 系列 | Chat Completions |
| Google | Gemini | Google AI API |
| DeepSeek | DeepSeek | OpenAI 兼容 |
| 本地 | Ollama / llama.cpp | OpenAI 兼容 |

## 4. OpenClaw 的独特能力

### 4.1 Voice Wake + Talk Mode

| 平台 | 能力 |
|------|------|
| macOS | 唤醒词 "Hey Molty" + push-to-talk overlay |
| iOS | 语音触发转发给 Gateway |
| Android | 持续语音监听 + ElevenLabs TTS + 系统 TTS fallback |

### 4.2 Live Canvas

Agent 可以通过 `canvas` 工具渲染可视化界面（A2UI - Agent to UI）。

用户在聊天中可以实时看到 Agent 生成的图表、表单、代码预览等。
Canvas 在 Companion App 中展示。

### 4.3 跨会话通信

通过 `sessions_*` 工具集，Agent 可以：
- 查看其他会话的历史
- 向其他会话发消息
- 创建新会话

这使多 Agent 协作成为可能——一个 Agent 可以委托另一个 Agent 处理子任务。

### 4.4 Cron 调度

Agent 可以设置定时任务：

```
"每天早上 8 点给我发天气预报"
→ Agent 创建 cron job
→ 每天 8:00 Gateway 触发 Agent
→ Agent 查天气 → 发消息到用户的主通道
```

### 4.5 Webhook 集成

支持外部事件触发 Agent：
- Gmail Pub/Sub（收到邮件时触发）
- GitHub Webhook（PR / Issue 事件触发）
- 自定义 Webhook

## 5. 沙箱与安全

### 5.1 沙箱模型

```
主会话（main session）
    → 无沙箱，Agent 有完整权限（"你自己的电脑，你说了算"）

非主会话（group / DM / webhook 触发）
    → 可配置沙箱模式
    → 三种后端：Docker（默认）/ SSH / OpenShell
```

### 5.2 沙箱内的工具控制

```
沙箱默认允许:  bash, process, read, write, edit, sessions_list/history/send/spawn
沙箱默认禁止:  browser, canvas, nodes, cron, discord, gateway
```

### 5.3 与 Claude Code 权限系统的对比

| 维度 | Claude Code | OpenClaw |
|------|------------|----------|
| 模型 | 每个工具调用可单独审批 | 按会话类型整体控制 |
| 实现 | 细粒度：命令级白/黑名单 | 粗粒度：会话级沙箱 on/off |
| 适用场景 | 单用户终端 | 多通道、多用户、多 Agent |
| 安全边界 | 进程级（当前用户权限） | 容器级（Docker 隔离） |

OpenClaw 选择粗粒度是因为它面对的场景不同：
它不需要逐条命令审批（那在 WhatsApp 上体验极差），
而是通过沙箱在环境层面隔离。

## 6. Companion App 架构

```
┌──────────────┐     WebSocket      ┌──────────────┐
│  macOS App   │◄──────────────────►│              │
│  (菜单栏)     │   device pairing   │              │
├──────────────┤                    │   Gateway    │
│  iOS App     │◄──────────────────►│   (控制平面)  │
│  (Node)      │   device pairing   │              │
├──────────────┤                    │              │
│  Android App │◄──────────────────►│              │
│  (Node)      │   device pairing   │              │
└──────────────┘                    └──────────────┘
```

- macOS App: Swift/Objective-C，菜单栏常驻，控制 Gateway 生命周期
- iOS/Android App: React Native，通过 WS 配对为 "Node"
- 配对命令: `openclaw devices pair` / `openclaw nodes list`

## 7. 对 Wem Agent 设计的启示

### 7.1 值得借鉴的设计

| 设计 | 启示 | 对 Wem 的适用性 |
|------|------|----------------|
| **Gateway 常驻模式** | Agent 不应该是按需启动的 CLI，而是常驻服务 | 高 — Wem 已有 Axum server，Agent 可复用 |
| **多通道接入** | 不绑定终端，通过 IM 通道更自然 | 中 — 后期可加，初期专注终端 + 前端 |
| **Skills = Markdown** | 扩展机制用 Markdown 而非代码，降低门槛 | 高 — 简单实用 |
| **沙箱分级** | 主会话信任，非主会话自动隔离 | 中 — 单用户场景暂不需要，多用户时有用 |
| **模型故障转移** | 不绑定单一 Provider | 高 — 用户可能同时有多个 API key |
| **Cron / Webhook 触发** | Agent 不只响应消息，还能被事件触发 | 中 — 后期可加 |

### 7.2 不需要借鉴的设计

| 设计 | 原因 |
|------|------|
| 26 个 Channel Adapter | Wem 是知识管理工具，不是通用聊天机器人 |
| Voice Wake | 编码/知识管理场景不需要语音 |
| Live Canvas | 过于复杂，初期不需要 |
| Companion App | 初期不需要，Web 前端已够 |
| DM 配对安全 | 单用户场景不需要 |

### 7.3 Wem Agent 可以采用的 OpenClaw 模式

1. **Gateway 模式**：Wem 的 Axum server 本身就是 Gateway，Agent 跑在同一进程内
2. **Skills 系统**：`.wem/skills/` 目录下放 SKILL.md，Agent 启动时加载
3. **多 Agent 配置**：agent.toml 中定义多个 sub_agent，不同场景用不同配置
4. **模型故障转移**：配置多个 Provider，主模型失败自动切换
5. **Sessions 工具**：主 Agent 通过 task_spawn 派生子 Agent，子 Agent 结果返回主 Agent
