/**
 * CopilotPanel — AI Agent 聊天面板
 *
 * 类似 GitHub Copilot Chat 的右侧交互面板：
 * - 消息列表（用户 + 助手，支持 Markdown）
 * - 流式接收助手回复（SSE）
 * - 工具调用展示（折叠式）
 * - 权限审批按钮
 * - 新建会话 / 中止对话
 */

import {
  useState,
  useRef,
  useCallback,
  useEffect,
  type FormEvent,
  type KeyboardEvent,
} from 'react'
import {
  createSession,
  chatStream,
  abortSession,
  resolvePermission,
  type ChatMessage,
  type ToolCallInfo,
  type AgentEvent,
} from '@/api/agent'

// ─── Helpers ───

function genId(): string {
  return crypto.randomUUID()
}

/** 简易 Markdown 渲染（粗体、代码、换行） */
function SimpleMarkdown({ text }: { text: string }) {
  // 逐行处理：粗体 **text** → <strong>，`code` → <code>，换行
  const lines = text.split('\n')
  return (
    <div className="whitespace-pre-wrap break-words text-sm leading-relaxed">
      {lines.map((line, i) => {
        // 粗体 + 行内代码
        const parts = line.split(/(\*\*[^*]+\*\*|`[^`]+`)/g)
        const rendered = parts.map((part, j) => {
          if (part.startsWith('**') && part.endsWith('**')) {
            return <strong key={j}>{part.slice(2, -2)}</strong>
          }
          if (part.startsWith('`') && part.endsWith('`')) {
            return (
              <code
                key={j}
                className="rounded bg-muted px-1 py-0.5 text-xs font-mono"
              >
                {part.slice(1, -1)}
              </code>
            )
          }
          return <span key={j}>{part}</span>
        })
        return (
          <span key={i}>
            {rendered}
            {i < lines.length - 1 && '\n'}
          </span>
        )
      })}
    </div>
  )
}

// ─── 消息气泡 ───

function MessageBubble({ message }: { message: ChatMessage }) {
  const isUser = message.role === 'user'

  return (
    <div className={`flex gap-2 ${isUser ? 'flex-row-reverse' : 'flex-row'}`}>
      {/* 头像 */}
      <div
        className={`
          shrink-0 w-6 h-6 rounded-full flex items-center justify-center text-xs font-medium
          ${isUser
            ? 'bg-primary text-primary-foreground'
            : 'bg-muted text-muted-foreground'
          }
        `}
      >
        {isUser ? 'U' : 'AI'}
      </div>

      {/* 内容 */}
      <div
        className={`
          flex-1 min-w-0 rounded-lg px-3 py-2
          ${isUser
            ? 'bg-primary/10 text-foreground'
            : 'bg-muted/50 text-foreground'
          }
        `}
      >
        {message.status === 'pending' && (
          <div className="flex items-center gap-1.5 text-muted-foreground text-sm">
            <span className="w-1.5 h-1.5 rounded-full bg-current animate-pulse" />
            <span>思考中…</span>
          </div>
        )}

        {message.content && <SimpleMarkdown text={message.content} />}
        {message.status === 'streaming' && (
          <span className="inline-block w-1.5 h-4 ml-0.5 bg-foreground/60 animate-pulse rounded-sm" />
        )}

        {/* 工具调用 */}
        {message.toolCalls.length > 0 && (
          <div className="mt-2 space-y-1">
            {message.toolCalls.map((tc) => (
              <ToolCallBubble key={tc.id} toolCall={tc} />
            ))}
          </div>
        )}

        {message.status === 'error' && message.error && (
          <div className="mt-1 text-xs text-destructive">{message.error}</div>
        )}
      </div>
    </div>
  )
}

// ─── 工具调用展示 ───

function ToolCallBubble({ toolCall }: { toolCall: ToolCallInfo }) {
  const [expanded, setExpanded] = useState(false)

  return (
    <div className="rounded border border-border/50 bg-background/50 text-xs">
      <button
        type="button"
        className="flex items-center gap-1.5 w-full px-2 py-1.5 text-left hover:bg-accent/30 rounded transition-colors"
        onClick={() => setExpanded(!expanded)}
      >
        <span className={`transition-transform ${expanded ? 'rotate-90' : ''}`}>▶</span>
        <span className="font-mono text-muted-foreground">{toolCall.name}</span>
        {toolCall.status === 'running' && (
          <span className="w-1.5 h-1.5 rounded-full bg-primary animate-pulse" />
        )}
        {toolCall.status === 'done' && (
          <span className="text-muted-foreground">✓</span>
        )}
      </button>
      {expanded && (
        <div className="px-2 pb-2 space-y-1">
          {toolCall.args != null && (
            <pre className="text-[10px] text-muted-foreground overflow-x-auto whitespace-pre-wrap break-all">
              {typeof toolCall.args === 'string'
                ? toolCall.args
                : JSON.stringify(toolCall.args, null, 2)}
            </pre>
          )}
          {toolCall.result != null && (
            <div className="text-[10px] text-muted-foreground border-t border-border/30 pt-1">
              {toolCall.result}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

// ─── 权限审批 ───

function PermissionBanner({
  toolName,
  onDecision,
}: {
  toolName: string
  onDecision: (approved: boolean) => void
}) {
  return (
    <div className="rounded-lg border border-yellow-500/30 bg-yellow-500/5 px-3 py-2 text-sm">
      <p className="mb-2 text-foreground">
        AI 请求使用工具：<code className="rounded bg-muted px-1 font-mono text-xs">{toolName}</code>
      </p>
      <div className="flex gap-2">
        <button
          type="button"
          className="px-3 py-1 rounded bg-primary text-primary-foreground text-xs hover:opacity-90 transition-opacity"
          onClick={() => onDecision(true)}
        >
          允许
        </button>
        <button
          type="button"
          className="px-3 py-1 rounded border border-border text-xs hover:bg-accent/30 transition-colors"
          onClick={() => onDecision(false)}
        >
          拒绝
        </button>
      </div>
    </div>
  )
}

// ─── 主面板组件 ───

export function CopilotPanel() {
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [input, setInput] = useState('')
  const [isLoading, setIsLoading] = useState(false)
  const [pendingPermission, setPendingPermission] = useState<{ toolName: string } | null>(null)

  const abortRef = useRef<AbortController | null>(null)
  const messagesEndRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)

  // 自动滚动到底部
  const scrollToBottom = useCallback(() => {
    requestAnimationFrame(() => {
      messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
    })
  }, [])

  // 初始化会话
  useEffect(() => {
    createSession()
      .then((id) => setSessionId(id))
      .catch((err) => console.error('[Copilot] 创建会话失败:', err))
  }, [])

  // 处理 SSE 事件
  const handleAgentEvent = useCallback(
    (assistantId: string, event: AgentEvent) => {
      switch (event.type) {
        case 'text_delta':
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? { ...m, content: m.content + event.text }
                : m,
            ),
          )
          scrollToBottom()
          break

        case 'tool_call_begin':
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? {
                    ...m,
                    toolCalls: [
                      ...m.toolCalls,
                      {
                        id: event.id,
                        name: event.name,
                        args: event.args,
                        status: 'running' as const,
                      },
                    ],
                  }
                : m,
            ),
          )
          break

        case 'tool_call_end':
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? {
                    ...m,
                    toolCalls: m.toolCalls.map((tc) =>
                      tc.id === event.id
                        ? { ...tc, result: event.result_summary, status: 'done' as const }
                        : tc,
                    ),
                  }
                : m,
            ),
          )
          break

        case 'permission_required':
          setPendingPermission({ toolName: event.tool_name })
          break

        case 'phase_changed':
          // 可选：根据 phase 显示进度
          break

        case 'step_progress':
          // 可选：显示步骤进度
          break
      }
    },
    [scrollToBottom],
  )

  // 发送消息
  const handleSend = useCallback(
    (e?: FormEvent) => {
      e?.preventDefault()
      const text = input.trim()
      if (!text || !sessionId || isLoading) return

      // 添加用户消息
      const userMsg: ChatMessage = {
        id: genId(),
        role: 'user',
        content: text,
        toolCalls: [],
        status: 'done',
      }

      // 添加助手占位消息
      const assistantId = genId()
      const assistantMsg: ChatMessage = {
        id: assistantId,
        role: 'assistant',
        content: '',
        toolCalls: [],
        status: 'pending',
      }

      setMessages((prev) => [...prev, userMsg, assistantMsg])
      setInput('')
      setIsLoading(true)

      // 调用流式 API
      const controller = chatStream(
        sessionId,
        text,
        (event) => {
          // 首次收到事件 → 切换为 streaming
          if (event.type === 'text_delta') {
            setMessages((prev) =>
              prev.map((m) =>
                m.id === assistantId && m.status === 'pending'
                  ? { ...m, status: 'streaming' }
                  : m,
              ),
            )
          }
          handleAgentEvent(assistantId, event)
        },
        (error) => {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? { ...m, status: 'error', error: error.message }
                : m,
            ),
          )
          setIsLoading(false)
        },
        () => {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? { ...m, status: 'done' }
                : m,
            ),
          )
          setIsLoading(false)
        },
      )

      abortRef.current = controller
    },
    [input, sessionId, isLoading, handleAgentEvent],
  )

  // 中止当前对话
  const handleAbort = useCallback(async () => {
    abortRef.current?.abort()
    if (sessionId) {
      await abortSession(sessionId).catch(() => {})
    }
    setIsLoading(false)
  }, [sessionId])

  // 权限审批
  const handlePermissionDecision = useCallback(
    async (approved: boolean) => {
      if (!sessionId) return
      setPendingPermission(null)
      await resolvePermission(sessionId, approved).catch(() => {})
    },
    [sessionId],
  )

  // 新建会话
  const handleNewSession = useCallback(async () => {
    abortRef.current?.abort()
    try {
      const id = await createSession()
      setSessionId(id)
      setMessages([])
      setPendingPermission(null)
      setIsLoading(false)
    } catch (err) {
      console.error('[Copilot] 新建会话失败:', err)
    }
  }, [])

  // 输入框快捷键
  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      // Enter 发送（Shift+Enter 换行）
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault()
        handleSend()
      }
    },
    [handleSend],
  )

  return (
    <div className="flex flex-col h-full wem-copilot-panel">
      {/* ─── 工具栏 ─── */}
      <div className="flex items-center gap-1 px-2 py-1.5 border-b border-border/50 shrink-0">
        <button
          type="button"
          className="p-1.5 rounded hover:bg-accent/30 text-muted-foreground hover:text-foreground transition-colors"
          onClick={handleNewSession}
          title="新对话"
        >
          <svg viewBox="0 0 24 24" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 5v14" />
            <path d="M5 12h14" />
          </svg>
        </button>
        {isLoading && (
          <button
            type="button"
            className="p-1.5 rounded hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-colors"
            onClick={handleAbort}
            title="停止"
          >
            <svg viewBox="0 0 24 24" className="h-4 w-4" fill="currentColor">
              <rect x="6" y="6" width="12" height="12" rx="2" />
            </svg>
          </button>
        )}
        <div className="flex-1" />
        <span className="text-[10px] text-muted-foreground">
          {sessionId ? sessionId.slice(0, 8) : '...'}
        </span>
      </div>

      {/* ─── 消息列表 ─── */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden px-3 py-3 space-y-4">
        {messages.length === 0 && (
          <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-3">
            <svg viewBox="0 0 24 24" className="h-8 w-8 opacity-30" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 2a8 8 0 0 1 8 8c0 3-1.5 5-3.5 6.5L16 21H8l-.5-4.5C5.5 15 4 13 4 10a8 8 0 0 1 8-8z" />
              <path d="M9 21v1" />
              <path d="M15 21v1" />
              <path d="M10 14h4" />
            </svg>
            <p className="text-xs text-center leading-relaxed">
              AI 助手就绪<br />
              输入问题开始对话
            </p>
          </div>
        )}

        {messages.map((msg) => (
          <MessageBubble key={msg.id} message={msg} />
        ))}

        {/* 权限审批 */}
        {pendingPermission && (
          <PermissionBanner
            toolName={pendingPermission.toolName}
            onDecision={handlePermissionDecision}
          />
        )}

        <div ref={messagesEndRef} />
      </div>

      {/* ─── 输入区 ─── */}
      <div className="shrink-0 border-t border-border/50 p-3">
        <form onSubmit={handleSend} className="relative">
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="输入消息… (Enter 发送, Shift+Enter 换行)"
            rows={2}
            disabled={isLoading || !sessionId}
            className="
              w-full resize-none rounded-lg border border-border bg-background
              px-3 py-2 text-sm leading-relaxed
              placeholder:text-muted-foreground
              focus:outline-none focus:ring-1 focus:ring-ring
              disabled:opacity-50 disabled:cursor-not-allowed
              transition-colors
            "
          />
          <button
            type="submit"
            disabled={isLoading || !input.trim() || !sessionId}
            className="
              absolute right-2 bottom-2 p-1.5 rounded-md
              bg-primary text-primary-foreground
              disabled:opacity-30 disabled:cursor-not-allowed
              hover:opacity-90 transition-opacity
            "
            title="发送"
          >
            <svg viewBox="0 0 24 24" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M5 12h14" />
              <path d="m12 5 7 7-7 7" />
            </svg>
          </button>
        </form>
      </div>
    </div>
  )
}
