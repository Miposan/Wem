/**
 * 跨块选区工具 — 计算、查询、序列化
 *
 * 编辑器中每个块是独立的 contentEditable，浏览器原生 Selection 无法跨块。
 * 本模块维护一个自定义的 EditorSelection（锚点 + 焦点），
 * 提供「从 DOM 解析」「计算被选中的块」「生成复制文本」等纯函数。
 */

import type { BlockNode } from '@/types/api'
import type { EditorSelection } from './types'
import { flattenTree } from './BlockOperations'

/**
 * 获取选区覆盖的所有块 ID（DFS 前序）
 *
 * 从 anchor 到 focus（按文档顺序），包含两端及中间所有块。
 */
export function getSelectedBlockIds(
  tree: BlockNode[],
  selection: EditorSelection,
): string[] {
  const flat = flattenTree(tree)
  const aIdx = flat.findIndex((b) => b.id === selection.anchorBlockId)
  const bIdx = flat.findIndex((b) => b.id === selection.focusBlockId)
  if (aIdx === -1 || bIdx === -1) return []

  const start = Math.min(aIdx, bIdx)
  const end = Math.max(aIdx, bIdx)
  return flat.slice(start, end + 1).map((b) => b.id)
}

/**
 * 从选区构建 Set<string>，便于 BlockContainer O(1) 判断
 */
export function getSelectedBlockIdsSet(
  tree: BlockNode[],
  selection: EditorSelection | null,
): Set<string> {
  if (!selection) return new Set()
  return new Set(getSelectedBlockIds(tree, selection))
}

/**
 * 从鼠标事件的 DOM 位置解析 blockId
 *
 * 遍历 DOM 向上查找 `[data-block-id]` 属性。
 */
export function getBlockIdFromPoint(el: Node | null): string | null {
  if (!el) return null
  const blockEl = (el instanceof HTMLElement ? el : el.parentElement)?.closest(
    '[data-block-id]',
  ) as HTMLElement | null
  return blockEl?.getAttribute('data-block-id') ?? null
}

/**
 * 从鼠标事件所在的 contentEditable 元素中计算字符偏移量
 *
 * 如果鼠标不在 contentEditable 内部，返回 0（块起始）或 Infinity（块末尾），
 * 取决于鼠标在块的上半部分还是下半部分。
 */
export function getOffsetFromMouseEvent(e: MouseEvent, blockId: string): number {
  const editable = document.querySelector(
    `[data-block-id="${blockId}"] [contenteditable="true"]`,
  ) as HTMLElement | null
  if (!editable) return 0

  // 如果事件目标在 contentEditable 内，尝试用 caretRangeFromPoint
  const range = document.caretRangeFromPoint?.(e.clientX, e.clientY)
  if (range) {
    const preRange = range.cloneRange()
    preRange.selectNodeContents(editable)
    preRange.setEnd(range.startContainer, range.startOffset)
    return preRange.toString().length
  }

  // 退化：无法精确获取，根据鼠标 Y 坐标判断头部或尾部
  const rect = editable.getBoundingClientRect()
  const midY = rect.top + rect.height / 2
  return e.clientY < midY ? 0 : (editable.textContent?.length ?? 0)
}
