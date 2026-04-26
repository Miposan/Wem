/**
 * useCopilotSession — Copilot 会话管理 Hook
 *
 * 职责：
 * - 加载 / 创建 / 切换 / 删除会话
 * - localStorage 缓存消息（刷新不丢失）
 * - SSE 流式发送消息
 * - 权限审批
 */

import { useState, useRef, useCallback, useEffect } from 'react'
import {
  createSession,
  listSessions,
  destroySession,
  chatStream,
  abortSession,
  resolvePermission as apiResolvePermission,
  type ChatMessage,
  type AgentEvent,
} from '@/api/agent'

// ─── 类型 ───

export interface SessionInfo {
  id: string
  /** 会话标题：取第一条用户消息的前 30 字，没有则为"新对话" */
  title: string
  /** 最后更新时间戳 */
  updatedAt: number
}

interface CacheData {
  sessions: SessionInfo[]
  messages: Record<string, ChatMessage[]>
}

// ─── Helpers ───

function genId(): string {
  return crypto.randomUUID()
}

const CACHE_KEY = 'wem-copilot-cache'

function loadCache(): CacheData {
  try {
    const raw = localStorage.getItem(CACHE_KEY)
    if (!raw) return { sessions: [], messages: {} }
    return JSON.parse(raw) as CacheData
  } catch {
    return { sessions: [], messages: {} }
  }
}

function saveCache(data: CacheData): void {
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify(data))
  } catch {
    // quota exceeded — 静默忽略
  }
}

/** 从消息列表推导标题 */
function deriveTitle(messages: ChatMessage[]): string {
  const first = messages.find((m) => m.role === 'user')
  if (!first) return '新对话'
  const text = first.content.trim().replace(/\n/g, ' ')
  return text.length > 30 ? text.slice(0, 30) + '…' : text
}

// ─── Hook ───

export function useCopilotSession() {
  const [sessions, setSessions] = useState<SessionInfo[]>([])
  const [activeId, setActiveId] = useState<string | null>(null)
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [isLoading, setIsLoading] = useState(false)
  const [pendingPermission, setPendingPermission] = useState<{
    toolName: string
  } | null>(null)

  const abortRef = useRef<AbortController | null>(null)
  const assistantIdRef = useRef<string | null>(null)

  // ─── 持久化：每次 messages 或 sessions 变化时写入 localStorage ───

  const persistCache = useCallback(
    (sess: SessionInfo[], msgs: ChatMessage[], currentId: string | null) => {
      const cache = loadCache()
      if (currentId) {
        cache.messages[currentId] = msgs
        // 更新对应 session 的 title 和 updatedAt
        const idx = sess.findIndex((s) => s.id === currentId)
        if (idx !== -1) {
          sess[idx].title = deriveTitle(msgs)
          sess[idx].updatedAt = Date.now()
        }
      }
      cache.sessions = sess
      saveCache(cache)
    },
    [],
  )

  // 当 messages 变化时持久化
  const prevMsgsLen = useRef(0)
  useEffect(() => {
    if (messages.length === 0 && prevMsgsLen.current === 0) return
    prevMsgsLen.current = messages.length
    if (activeId) {
      persistCache(sessions, messages, activeId)
    }
  }, [messages, activeId, sessions, persistCache])

  // ─── 加载消息（从缓存恢复） ───

  const loadMessages = useCallback((sessionId: string): ChatMessage[] => {
    const cache = loadCache()
    return cache.messages[sessionId] ?? []
  }, [])

  // ─── 初始化 ───

  const init = useCallback(async () => {
    // 1) 从缓存恢复 sessions
    const cache = loadCache()
    const cachedSessions = cache.sessions

    // 2) 从后端拉取最新 sessions
    let remoteIds: string[] = []
    try {
      remoteIds = await listSessions()
    } catch {
      // 后端不可用 — 仍然允许离线浏览缓存
    }

    // 3) 合并：以远程为准，但保留缓存的 title
    const merged: SessionInfo[] = remoteIds.map((id) => {
      const cached = cachedSessions.find((s) => s.id === id)
      return cached ?? { id, title: '新对话', updatedAt: Date.now() }
    })

    // 按 updatedAt 降序排列
    merged.sort((a, b) => b.updatedAt - a.updatedAt)
    setSessions(merged)

    // 4) 选择活跃会话：优先恢复上次的，否则取最新的
    if (merged.length > 0) {
      const target = merged[0]
      setActiveId(target.id)
      setMessages(loadMessages(target.id))
    } else {
      // 没有任何会话 → 自动创建
      await createNewSession()
    }
  }, [loadMessages])

  // 组件挂载时自动初始化
  useEffect(() => {
    init().catch((err) => console.error('[Copilot] 初始化失败:', err))
    // eslint-disable-next-line react-hooks/exhaustive-deps — 只在挂载时执行
  }, [])

  // ─── 创建新会话 ───

  const createNewSession = useCallback(async () => {
    // 先中止当前流
    abortRef.current?.abort()
    setIsLoading(false)
    setPendingPermission(null)

    try {
      const id = await createSession()
      const info: SessionInfo = { id, title: '新对话', updatedAt: Date.now() }
      setSessions((prev) => [info, ...prev])
      setActiveId(id)
      setMessages([])
      // 写入缓存
      const cache = loadCache()
      cache.sessions.unshift(info)
      cache.messages[id] = []
      saveCache(cache)
    } catch (err) {
      console.error('[Copilot] 创建会话失败:', err)
    }
  }, [])

  // ─── 切换会话 ───

  const switchTo = useCallback(
    (id: string) => {
      if (id === activeId) return
      // 先中止当前流
      abortRef.current?.abort()
      setIsLoading(false)
      setPendingPermission(null)

      setActiveId(id)
      setMessages(loadMessages(id))
    },
    [activeId, loadMessages],
  )

  // ─── 删除会话 ───

  const removeSession = useCallback(
    async (id: string) => {
      // 如果删除的是当前会话，先中止流
      if (id === activeId) {
        abortRef.current?.abort()
        setIsLoading(false)
        setPendingPermission(null)
      }

      // 后端删除
      try {
        await destroySession(id)
      } catch {
        // 静默
      }

      // 更新 state
      setSessions((prev) => {
        const next = prev.filter((s) => s.id !== id)
        // 如果删的是当前会话，自动切换
        if (id === activeId && next.length > 0) {
          setActiveId(next[0].id)
          setMessages(loadMessages(next[0].id))
        } else if (id === activeId && next.length === 0) {
          setActiveId(null)
          setMessages([])
        }
        return next
      })

      // 清理缓存
      const cache = loadCache()
      delete cache.messages[id]
      cache.sessions = cache.sessions.filter((s) => s.id !== id)
      saveCache(cache)
    },
    [activeId, loadMessages],
  )

  // ─── SSE 事件处理 ───

  const handleAgentEvent = useCallback(
    (event: AgentEvent, asstId: string) => {
      switch (event.type) {
        case 'text_delta':
          setMessages((prev) =>
            prev.map((m) =>
              m.id === asstId ? { ...m, content: m.content + event.text } : m,
            ),
          )
          break

        case 'tool_call_begin':
          setMessages((prev) =>
            prev.map((m) =>
              m.id === asstId
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
              m.id === asstId
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
        case 'step_progress':
          // 可选扩展
          break
      }
    },
    [],
  )

  // ─── 发送消息 ───

  const sendMessage = useCallback(
    (text: string) => {
      if (!text.trim() || !activeId || isLoading) return

      const userMsg: ChatMessage = {
        id: genId(),
        role: 'user',
        content: text.trim(),
        toolCalls: [],
        status: 'done',
      }

      const asstId = genId()
      assistantIdRef.current = asstId
      const assistantMsg: ChatMessage = {
        id: asstId,
        role: 'assistant',
        content: '',
        toolCalls: [],
        status: 'pending',
      }

      setMessages((prev) => [...prev, userMsg, assistantMsg])
      setIsLoading(true)

      const controller = chatStream(
        activeId,
        text.trim(),
        (event) => {
          // 首次收到事件 → streaming
          if (event.type === 'text_delta') {
            setMessages((prev) =>
              prev.map((m) =>
                m.id === asstId && m.status === 'pending'
                  ? { ...m, status: 'streaming' }
                  : m,
              ),
            )
          }
          handleAgentEvent(event, asstId)
        },
        (error) => {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === asstId ? { ...m, status: 'error', error: error.message } : m,
            ),
          )
          setIsLoading(false)
        },
        () => {
          setMessages((prev) =>
            prev.map((m) => (m.id === asstId ? { ...m, status: 'done' } : m)),
          )
          setIsLoading(false)
        },
      )

      abortRef.current = controller
    },
    [activeId, isLoading, handleAgentEvent],
  )

  // ─── 中止当前轮 ───

  const abort = useCallback(async () => {
    abortRef.current?.abort()
    if (activeId) {
      await abortSession(activeId).catch(() => {})
    }
    // 将正在流式的 assistant 消息标记为 done
    if (assistantIdRef.current) {
      setMessages((prev) =>
        prev.map((m) =>
          m.id === assistantIdRef.current && m.status === 'streaming'
            ? { ...m, status: 'done' }
            : m,
        ),
      )
    }
    setIsLoading(false)
  }, [activeId])

  // ─── 权限审批 ───

  const resolvePerm = useCallback(
    async (approved: boolean) => {
      if (!activeId) return
      setPendingPermission(null)
      await apiResolvePermission(activeId, approved).catch(() => {})
    },
    [activeId],
  )

  return {
    // state
    sessions,
    activeId,
    messages,
    isLoading,
    pendingPermission,
    // actions
    createNewSession,
    switchTo,
    removeSession,
    sendMessage,
    abort,
    resolvePerm,
  }
}
