/**
 * useDocumentSSE — 文档级 SSE 事件订阅 Hook
 *
 * 连接后端 SSE 端点 `GET /api/v1/documents/{id}/events`，
 * 实时接收文档变更事件并回调。
 *
 * 设计理念：后端是唯一数据真相源，前端是投影。
 * 无论变更来自前端 REST 还是 Agent 后端操作，都通过此通道推送到 UI。
 */

import { useEffect, useRef } from 'react'
import type { Block } from '@/types/api'

// ─── SSE 事件类型（与后端 BlockEvent serde(flatten) 一一对应） ───
//
// 后端 BlockEvent 使用 #[serde(flatten)] 将 block 字段展平到 JSON 根级别，
// 所以 block 的 id/content/version 等字段与 type/document_id 平级。

/** 携带完整 block 数据的事件（block 字段被 serde flatten 展平到顶层） */
export type BlockEventPayload = Block & {
  document_id: string
  operation_id?: string
}

export interface BlockCreatedEvent extends BlockEventPayload {
  type: 'block_created'
}

export interface BlockUpdatedEvent extends BlockEventPayload {
  type: 'block_updated'
}

export interface BlockDeletedEvent {
  type: 'block_deleted'
  document_id: string
  block_id: string
  cascade_count: number
  operation_id?: string
}

export interface BlockMovedEvent extends BlockEventPayload {
  type: 'block_moved'
}

export interface BlockRestoredEvent extends BlockEventPayload {
  type: 'block_restored'
}

export interface BlocksBatchChangedEvent {
  type: 'blocks_batch_changed'
  document_id: string
  operation_id?: string
}

export type BlockEvent =
  | BlockCreatedEvent
  | BlockUpdatedEvent
  | BlockDeletedEvent
  | BlockMovedEvent
  | BlockRestoredEvent
  | BlocksBatchChangedEvent

// ─── Hook 回调 ───

export interface SSECallbacks {
  /** 块被创建（来自外部，如 Agent 操作） */
  onBlockCreated?: (event: BlockCreatedEvent) => void
  /** 块内容/属性被更新 */
  onBlockUpdated?: (event: BlockUpdatedEvent) => void
  /** 块被删除 */
  onBlockDeleted?: (event: BlockDeletedEvent) => void
  /** 块被移动 */
  onBlockMoved?: (event: BlockMovedEvent) => void
  /** 块被恢复 */
  onBlockRestored?: (event: BlockRestoredEvent) => void
  /** 批量操作完成，前端应 refetch */
  onBlocksBatchChanged?: (event: BlocksBatchChangedEvent) => void
}

// ─── Hook ───

const SSE_BASE = import.meta.env.VITE_API_BASE_URL ?? 'http://localhost:6809/api/v1'

/**
 * 订阅指定文档的 SSE 事件流
 *
 * @param documentId 文档 ID（切换文档时自动重连）
 * @param callbacks 事件回调
 */
export function useDocumentSSE(
  documentId: string | undefined,
  callbacks: SSECallbacks,
) {
  const callbacksRef = useRef(callbacks)
  callbacksRef.current = callbacks

  useEffect(() => {
    if (!documentId) return

    const url = `${SSE_BASE}/documents/${documentId}/events`
    const es = new EventSource(url)

    console.log(`[SSE] 连接: ${url}`)

    const handler = (e: MessageEvent) => {
      try {
        const event: BlockEvent = JSON.parse(e.data)
        const cbs = callbacksRef.current

        switch (event.type) {
          case 'block_created':
            cbs.onBlockCreated?.(event)
            break
          case 'block_updated':
            cbs.onBlockUpdated?.(event)
            break
          case 'block_deleted':
            cbs.onBlockDeleted?.(event)
            break
          case 'block_moved':
            cbs.onBlockMoved?.(event)
            break
          case 'block_restored':
            cbs.onBlockRestored?.(event)
            break
          case 'blocks_batch_changed':
            cbs.onBlocksBatchChanged?.(event)
            break
        }
      } catch (err) {
        console.error('[SSE] 解析事件失败:', err, e.data)
      }
    }

    // SSE 的 event type 对应后端的 event.event_type()
    es.addEventListener('block_created', handler)
    es.addEventListener('block_updated', handler)
    es.addEventListener('block_deleted', handler)
    es.addEventListener('block_moved', handler)
    es.addEventListener('block_restored', handler)
    es.addEventListener('blocks_batch_changed', handler)

    es.onerror = () => {
      console.warn('[SSE] 连接断开，将自动重连...')
    }

    return () => {
      console.log(`[SSE] 断开: ${url}`)
      es.close()
    }
  }, [documentId])
}
