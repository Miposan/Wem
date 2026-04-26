/**
 * CopilotPanel — AI Agent 聊天面板
 *
 * 类似 GitHub Copilot Chat 的右侧交互面板：
 * - 会话列表侧栏（创建 / 切换 / 删除）
 * - 消息列表（用户 + 助手，支持 Markdown）
 * - 流式接收助手回复（SSE）
 * - 工具调用展示（折叠式）
 * - 权限审批按钮
 * - localStorage 缓存（刷新不丢消息）
 */

import { useState, useRef, useCallback, useEffect, type KeyboardEvent } from 'react'
import { Plus, X, PanelLeft, Square, Lightbulb, ArrowRight } from 'lucide-react'
import { useCopilotSession } from '@/hooks/useCopilotSession'
import type { ChatMessage, ToolCallInfo } from '@/api/agent'

// ─── 简易 Markdown 渲染 ───

function SimpleMarkdown({ text }: { text: string }) {
  const lines = text.split('\n')
  return (
    <div className="whitespace-pre-wrap break-words text-sm leading-relaxed">
      {lines.map((line, i) => {
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

// ─── 会话列表侧栏 ───

function SessionSidebar({
  sessions,
  activeId,
  onSelect,
  onDelete,
  onNew,
}: {
  sessions: { id: string; title: string; updatedAt: number }[]
  activeId: string | null
  onSelect: (id: string) => void
  onDelete: (id: string) => void
  onNew: () => void
}) {
  return (
    <div className="flex flex-col h-full wem-copilot-sidebar">
      {/* 新建按钮 */}
      <div className="shrink-0 p-2 border-b border-border/50">
        <button
          type="button"
          className="flex items-center gap-1.5 w-full px-2 py-1.5 rounded hover:bg-accent/30 text-muted-foreground hover:text-foreground text-xs transition-colors"
          onClick={onNew}
        >
          <Plus className="h-3.5 w-3.5" />
          新对话
        </button>
      </div>

      {/* 会话列表 */}
      <div className="flex-1 overflow-y-auto">
        {sessions.map((s) => (
          <div
            key={s.id}
            className={`
              group flex items-center gap-1 px-2 py-1.5 cursor-pointer text-xs
              transition-colors
              ${s.id === activeId
                ? 'bg-accent/40 text-foreground'
                : 'text-muted-foreground hover:bg-accent/20 hover:text-foreground'
              }
            `}
            onClick={() => onSelect(s.id)}
          >
            <span className="flex-1 truncate">{s.title}</span>
            <button
              type="button"
              className="shrink-0 p-0.5 rounded opacity-0 group-hover:opacity-100 hover:bg-destructive/10 hover:text-destructive transition-all"
              title="删除此对话"
              onClick={(e) => {
                e.stopPropagation()
                onDelete(s.id)
              }}
            >
              <X className="h-3 w-3" />
            </button>
          </div>
        ))}

        {sessions.length === 0 && (
          <div className="px-2 py-4 text-center text-muted-foreground text-[10px]">
            暂无对话
          </div>
        )}
      </div>
    </div>
  )
}

// ─── 主面板组件 ───

export function CopilotPanel() {
  const {
    sessions,
    activeId,
    messages,
    isLoading,
    pendingPermission,
    createNewSession,
    switchTo,
    removeSession,
    sendMessage,
    abort,
    resolvePerm,
  } = useCopilotSession()

  const [input, setInput] = useState('')
  const [showSidebar, setShowSidebar] = useState(false)
  const messagesEndRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)

  // 自动滚动到底部
  const scrollToBottom = useCallback(() => {
    requestAnimationFrame(() => {
      messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' })
    })
  }, [])

  // 消息变化时自动滚动
  useEffect(() => {
    scrollToBottom()
  }, [messages.length, messages, scrollToBottom])

  // 发送
  const handleSend = useCallback(() => {
    const text = input.trim()
    if (!text || isLoading) return
    sendMessage(text)
    setInput('')
    inputRef.current?.focus()
  }, [input, isLoading, sendMessage])

  // 快捷键
  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault()
        handleSend()
      }
    },
    [handleSend],
  )

  return (
    <div className="flex h-full wem-copilot-panel">
      {/* ─── 会话列表侧栏 ─── */}
      {showSidebar && (
        <div className="w-48 shrink-0 border-r border-border/50 bg-background/50">
          <SessionSidebar
            sessions={sessions}
            activeId={activeId}
            onSelect={(id) => {
              switchTo(id)
              setShowSidebar(false)
            }}
            onDelete={removeSession}
            onNew={() => {
              createNewSession()
              setShowSidebar(false)
            }}
          />
        </div>
      )}

      {/* ─── 聊天主区域 ─── */}
      <div className="flex flex-col flex-1 min-w-0">
        {/* 工具栏 */}
        <div className="flex items-center gap-1 px-2 py-1.5 border-b border-border/50 shrink-0">
          {/* 侧栏切换 */}
          <button
            type="button"
            className={`p-1.5 rounded hover:bg-accent/30 transition-colors ${showSidebar ? 'text-foreground bg-accent/20' : 'text-muted-foreground'}`}
            onClick={() => setShowSidebar(!showSidebar)}
            title="对话列表"
          >
            <PanelLeft className="h-4 w-4" />
          </button>

          {/* 新对话 */}
          <button
            type="button"
            className="p-1.5 rounded hover:bg-accent/30 text-muted-foreground hover:text-foreground transition-colors"
            onClick={createNewSession}
            title="新对话"
          >
            <Plus className="h-4 w-4" />
          </button>

          {/* 停止 */}
          {isLoading && (
            <button
              type="button"
              className="p-1.5 rounded hover:bg-destructive/10 text-muted-foreground hover:text-destructive transition-colors"
              onClick={abort}
              title="停止"
            >
              <Square className="h-4 w-4" fill="currentColor" />
            </button>
          )}

          <div className="flex-1" />
          <span className="text-[10px] text-muted-foreground">
            {activeId ? activeId.slice(0, 8) : '...'}
          </span>
        </div>

        {/* 消息列表 */}
        <div className="flex-1 overflow-y-auto overflow-x-hidden px-3 py-3 space-y-4">
          {messages.length === 0 && (
            <div className="flex flex-col items-center justify-center h-full text-muted-foreground gap-3">
              <Lightbulb className="h-8 w-8 opacity-30" />
              <p className="text-xs text-center leading-relaxed">
                AI 助手就绪<br />
                输入问题开始对话
              </p>
            </div>
          )}

          {messages.map((msg) => (
            <MessageBubble key={msg.id} message={msg} />
          ))}

          {pendingPermission && (
            <PermissionBanner
              toolName={pendingPermission.toolName}
              onDecision={resolvePerm}
            />
          )}

          <div ref={messagesEndRef} />
        </div>

        {/* 输入区 */}
        <div className="shrink-0 border-t border-border/50 p-3">
          <div className="relative">
            <textarea
              ref={inputRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="输入消息… (Enter 发送, Shift+Enter 换行)"
              rows={2}
              disabled={isLoading || !activeId}
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
              type="button"
              disabled={isLoading || !input.trim() || !activeId}
              className="
                absolute right-2 bottom-2 p-1.5 rounded-md
                bg-primary text-primary-foreground
                disabled:opacity-30 disabled:cursor-not-allowed
                hover:opacity-90 transition-opacity
              "
              title="发送"
              onClick={handleSend}
            >
              <ArrowRight className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
