import { useCallback, useEffect, useRef } from 'react'
import type { TextBlockProps } from './types'

/**
 * 文本块共享逻辑 hook
 *
 * 抽取 ParagraphBlock / HeadingBlock / 未来的 ListItem 等的共同行为：
 * - mount/块切换时同步 DOM 内容
 * - input → onContentChange
 * - 键盘操作 → onAction（Enter/Backspace/ArrowUp/ArrowDown）
 */
export function useTextBlock({ block, onContentChange, onAction }: TextBlockProps) {
  const ref = useRef<HTMLElement>(null)

  // ── IME 组合输入守卫 ──
  // 中文/日文等输入法打拼音阶段不应触发 save，
  // 否则中间文本（如 "nihao"）会被 debounce 发送到后端。
  const isComposing = useRef(false)

  const handleCompositionStart = useCallback(() => {
    isComposing.current = true
  }, [])

  const handleCompositionEnd = useCallback(() => {
    isComposing.current = false
    // compositionend 后立即提交最终文本
    const el = ref.current
    if (!el) return
    onContentChange(block.id, el.textContent || '')
  }, [block.id, onContentChange])

  // 仅在 block.id 变化时同步 DOM
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const text = block.content ?? ''
    if (el.textContent !== text) {
      el.textContent = text
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [block.id])

  const handleInput = useCallback(() => {
    // IME 组合输入中，跳过（等 compositionend 统一提交）
    if (isComposing.current) return
    const el = ref.current
    if (!el) return
    onContentChange(block.id, el.textContent || '')
  }, [block.id, onContentChange])

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const el = ref.current
      if (!el) return

      const sel = window.getSelection()
      if (!sel || !sel.rangeCount) return

      const range = sel.getRangeAt(0)
      const text = el.textContent || ''

      // 计算光标偏移
      const preRange = range.cloneRange()
      preRange.selectNodeContents(el)
      preRange.setEnd(range.startContainer, range.startOffset)
      const offset = preRange.toString().length

      const atStart = offset === 0
      const atEnd = offset === text.length
      const isEmpty = text.length === 0

      // Enter → 拆分
      if (e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
        e.preventDefault()
        onAction({ type: 'split', blockId: block.id, offset })
        return
      }

      // Backspace at start of empty → 删除块
      if (e.key === 'Backspace' && atStart && isEmpty) {
        e.preventDefault()
        onAction({ type: 'delete', blockId: block.id })
        return
      }

      // Backspace at start with content → 与前块合并
      if (e.key === 'Backspace' && atStart && !isEmpty) {
        e.preventDefault()
        onAction({ type: 'merge-with-previous', blockId: block.id })
        return
      }

      // ArrowUp at start → 上一块
      if (e.key === 'ArrowUp' && atStart) {
        e.preventDefault()
        onAction({ type: 'focus-previous', blockId: block.id })
        return
      }

      // ArrowDown at end → 下一块
      if (e.key === 'ArrowDown' && atEnd) {
        e.preventDefault()
        onAction({ type: 'focus-next', blockId: block.id })
        return
      }
    },
    [block.id, onAction],
  )

  return { ref, handleInput, handleKeyDown, handleCompositionStart, handleCompositionEnd }
}
