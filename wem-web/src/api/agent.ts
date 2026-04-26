/**
 * Agent API — AI Agent 客户端
 *
 * 封装后端 Agent SSE 流式聊天接口：
 * - createSession: 创建对话会话
 * - chat: 发送消息 + SSE 流式接收事件
 * - abort: 中止当前对话轮次
 * - resolvePermission: 权限审批
 */

import axios from 'axios'
import { API_BASE_URL } from '@/lib/utils'

// ─── Axios 实例 ───

const agentApi = axios.create({
  baseURL: API_BASE_URL,
  headers: { 'Content-Type': 'application/json' },
  timeout: 30_000,
})

// ─── SSE 事件类型（与后端 AgentEvent 一一对应） ───

export type AgentEventType =
  | 'text_delta'
  | 'tool_call_begin'
  | 'tool_call_end'
  | 'permission_required'
  | 'step_progress'
  | 'phase_changed'
  | 'done'
  | 'error'

export interface TextDeltaEvent {
  type: 'text_delta'
  text: string
}

export interface ToolCallBeginEvent {
  type: 'tool_call_begin'
  id: string
  name: string
  args: unknown
}

export interface ToolCallEndEvent {
  type: 'tool_call_end'
  id: string
  result_summary: string
}

export interface PermissionRequiredEvent {
  type: 'permission_required'
  tool_name: string
  args: unknown
}

export interface StepProgressEvent {
  type: 'step_progress'
  step: number
  max_steps: number
}

export interface PhaseChangedEvent {
  type: 'phase_changed'
  phase: AgentPhase
}

export interface DoneEvent {
  type: 'done'
}

export interface ErrorEvent {
  type: 'error'
  message: string
}

export type AgentEvent =
  | TextDeltaEvent
  | ToolCallBeginEvent
  | ToolCallEndEvent
  | PermissionRequiredEvent
  | StepProgressEvent
  | PhaseChangedEvent
  | DoneEvent
  | ErrorEvent

export type AgentPhase =
  | 'initializing'
  | 'preparing_turn'
  | 'streaming_model'
  | 'executing_tools'
  | 'completed'
  | 'cancelled'
  | 'failed'

// ─── 聊天消息（本地 UI 状态） ───

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  toolCalls: ToolCallInfo[]
  status: 'pending' | 'streaming' | 'done' | 'error'
  error?: string
}

export interface ToolCallInfo {
  id: string
  name: string
  args: unknown
  result?: string
  status: 'running' | 'done'
}

// ─── API 方法 ───

/** 创建会话 */
export async function createSession(opts?: {
  model?: string
  maxSteps?: number
}): Promise<string> {
  const res = await agentApi.post('/agent/sessions', {
    model: opts?.model,
    max_steps: opts?.maxSteps,
  })
  return res.data.session_id as string
}

/** 列出所有会话 */
export async function listSessions(): Promise<string[]> {
  const res = await agentApi.post('/agent/sessions/list')
  return res.data.sessions as string[]
}

/** 销毁会话 */
export async function destroySession(id: string): Promise<void> {
  await agentApi.post(`/agent/sessions/${id}`)
}

/**
 * 发送聊天消息（SSE 流式）
 *
 * 返回一个 AbortController 供取消，以及事件回调。
 */
export function chatStream(
  sessionId: string,
  message: string,
  onEvent: (event: AgentEvent) => void,
  onError: (error: Error) => void,
  onDone: () => void,
): AbortController {
  const controller = new AbortController()

  // 用 fetch 做 SSE，axios 不原生支持 streaming
  const url = `${agentApi.defaults.baseURL}/agent/sessions/${sessionId}/chat`

  fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ message }),
    signal: controller.signal,
  })
    .then(async (response) => {
      if (!response.ok) {
        const text = await response.text()
        throw new Error(`Agent chat failed: ${response.status} ${text}`)
      }

      const reader = response.body?.getReader()
      if (!reader) throw new Error('No response body')

      const decoder = new TextDecoder()
      let buffer = ''

      while (true) {
        const { done, value } = await reader.read()
        if (done) break

        buffer += decoder.decode(value, { stream: true })

        // SSE 协议：按 \n\n 分割事件
        const parts = buffer.split('\n\n')
        buffer = parts.pop() ?? ''

        for (const part of parts) {
          const lines = part.split('\n')
          for (const line of lines) {
            if (!line.startsWith('data:')) continue
            const jsonStr = line.slice(5).trim()
            if (!jsonStr) continue

            try {
              const event = JSON.parse(jsonStr) as AgentEvent
              onEvent(event)
              if (event.type === 'done') {
                onDone()
                return
              }
              if (event.type === 'error') {
                onError(new Error(event.message))
                return
              }
            } catch {
              // JSON 解析失败，跳过
            }
          }
        }
      }

      // 流结束但没收到 done
      onDone()
    })
    .catch((err) => {
      if (err.name === 'AbortError') return
      onError(err)
    })

  return controller
}

/** 中止当前对话轮次 */
export async function abortSession(sessionId: string): Promise<void> {
  await agentApi.post(`/agent/sessions/${sessionId}/abort`)
}

/** 权限审批 */
export async function resolvePermission(
  sessionId: string,
  approved: boolean,
): Promise<void> {
  await agentApi.post(`/agent/sessions/${sessionId}/permission`, { approved })
}
