import { useCallback, useEffect, useRef } from 'react'
import { makeHeadingType, makeListType, makeCodeBlockType, makeMathBlockType } from '@/types/api'
import type { BlockType } from '@/types/api'
import type { TextBlockProps } from './types'

/** 空块按 Enter/Backspace 应转为 paragraph 而非删除的类型 */
function shouldConvertWhenEmpty(blockType: string): boolean {
  return blockType === 'heading' || blockType === 'blockquote'
}

/**
 * 文本块共享逻辑 hook
 *
 * 抽取 ParagraphBlock / HeadingBlock / 未来的 ListItem 等的共同行为：
 * - mount/块切换时同步 DOM 内容
 * - input → onContentChange
 * - 键盘操作 → onAction（Enter/Backspace/ArrowUp/ArrowDown）
 */
export function useTextBlock({ block, onContentChange, onAction, selectedBlockIds }: TextBlockProps) {
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

  // ── 粘贴 → 剥离格式，仅保留纯文本 ──
  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    const text = e.clipboardData.getData('text/plain')
    if (!text) return
    e.preventDefault()
    document.execCommand('insertText', false, text)
  }, [])

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

      // IME 组合输入中，跳过所有键处理（等 compositionend）
      if (isComposing.current) return

      // ── 跨块选区 Backspace / Delete → delete-range ──
      if ((e.key === 'Backspace' || e.key === 'Delete') && selectedBlockIds.size > 1) {
        e.preventDefault()
        onAction({ type: 'delete-range', blockIds: Array.from(selectedBlockIds) })
        return
      }

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

      // ── 有选区时：Backspace/Delete 交给浏览器原生处理 ──
      if (!sel.isCollapsed && (e.key === 'Backspace' || e.key === 'Delete')) {
        // 原生删除选中文本，不走到边界逻辑
        return
      }

      // ListItem: Tab / Shift+Tab 调整列表层级
      if (e.key === 'Tab' && block.block_type.type === 'listItem') {
        e.preventDefault()
        onAction({
          type: e.shiftKey ? 'outdent-list-item' : 'indent-list-item',
          blockId: block.id,
        })
        return
      }

      // ── Markdown 快捷键（仅对 paragraph 块生效）──
      if (e.key === ' ' && block.block_type.type === 'paragraph') {
        const beforeCursor = text.slice(0, offset)

        // # + Space → Heading
        const headingMatch = beforeCursor.match(/^(#{1,6})$/)
        if (headingMatch) {
          e.preventDefault()
          const level = headingMatch[1].length
          const rest = text.slice(offset).replace(/^\s+/, '')
          el.textContent = rest
          onAction({
            type: 'convert-block',
            blockId: block.id,
            content: rest,
            blockType: makeHeadingType(level),
          })
          return
        }

        // - + Space 或 * + Space → 无序列表
        const ulMatch = beforeCursor.match(/^[-*]$/)
        if (ulMatch) {
          e.preventDefault()
          const rest = text.slice(offset).replace(/^\s+/, '')
          el.textContent = rest
          onAction({
            type: 'convert-block',
            blockId: block.id,
            content: rest,
            blockType: makeListType(false),
          })
          return
        }

        // 1. + Space → 有序列表
        const olMatch = beforeCursor.match(/^(\d+)\.$/)
        if (olMatch) {
          e.preventDefault()
          const rest = text.slice(offset).replace(/^\s+/, '')
          el.textContent = rest
          onAction({
            type: 'convert-block',
            blockId: block.id,
            content: rest,
            blockType: makeListType(true),
          })
          return
        }

        // > + Space → 引用块
        if (beforeCursor === '>') {
          e.preventDefault()
          const rest = text.slice(offset).replace(/^\s+/, '')
          el.textContent = rest
          onAction({
            type: 'convert-block',
            blockId: block.id,
            content: rest,
            blockType: { type: 'blockquote' } as BlockType,
          })
          return
        }
      }

      // ``` + Space → CodeBlock（注意 ``` 末尾可能带语言名如 ```rust）
      if (e.key === ' ' && block.block_type.type === 'paragraph') {
        const beforeCursor = text.slice(0, offset)
        const codeMatch = beforeCursor.match(/^```(\w*)$/)
        if (codeMatch) {
          e.preventDefault()
          const language = codeMatch[1] || 'text'
          const rest = text.slice(offset).replace(/^\s+/, '')
          el.textContent = rest
          onAction({
            type: 'convert-block',
            blockId: block.id,
            content: rest,
            blockType: makeCodeBlockType(language),
          })
          return
        }

        // $$ + Space → MathBlock
        if (beforeCursor === '$$') {
          e.preventDefault()
          el.textContent = ''
          onAction({
            type: 'convert-block',
            blockId: block.id,
            content: '',
            blockType: makeMathBlockType(),
          })
          return
        }
      }

      // --- / *** / ___ + Enter → 分割线
      if (
        e.key === 'Enter' &&
        !e.shiftKey &&
        !e.ctrlKey &&
        !e.metaKey &&
        block.block_type.type === 'paragraph' &&
        /^(---|\*\*\*|___)$/.test(text.trim())
      ) {
        e.preventDefault()
        el.textContent = ''
        onAction({
          type: 'convert-block',
          blockId: block.id,
          content: '',
          blockType: { type: 'thematicBreak' } as BlockType,
        })
        return
      }

      // Enter → 拆分
      if (e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
        e.preventDefault()
        if (block.block_type.type === 'listItem' && isEmpty) {
          onAction({ type: 'exit-list', blockId: block.id })
        } else if (isEmpty && shouldConvertWhenEmpty(block.block_type.type)) {
          onAction({ type: 'convert-block', blockId: block.id, content: '', blockType: { type: 'paragraph' } as BlockType })
        } else {
          onAction({ type: 'split' })
        }
        return
      }

      // Backspace at start of empty → 转换或删除块
      if (e.key === 'Backspace' && atStart && isEmpty) {
        e.preventDefault()
        if (block.block_type.type === 'listItem') {
          onAction({ type: 'outdent-list-item', blockId: block.id })
        } else if (shouldConvertWhenEmpty(block.block_type.type)) {
          onAction({ type: 'convert-block', blockId: block.id, content: '', blockType: { type: 'paragraph' } as BlockType })
        } else {
          onAction({ type: 'delete', blockId: block.id })
        }
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

      // ArrowLeft at start → 上一块末尾
      if (e.key === 'ArrowLeft' && atStart) {
        e.preventDefault()
        onAction({ type: 'focus-previous', blockId: block.id })
        return
      }

      // ArrowRight at end → 下一块开头
      if (e.key === 'ArrowRight' && atEnd) {
        e.preventDefault()
        onAction({ type: 'focus-next', blockId: block.id })
        return
      }

      // Home → 光标移到块首（浏览器原生处理）
      // End → 光标移到块末（浏览器原生处理）
      // Ctrl+Home → 已由浏览器滚动到页面顶部
      // Ctrl+End → 已由浏览器滚动到页面底部

      // Delete at end → 与下一块合并
      if (e.key === 'Delete' && atEnd && !isEmpty) {
        e.preventDefault()
        onAction({ type: 'merge-with-next', blockId: block.id })
        return
      }

      // Delete at end of empty → 转换或删除块
      if (e.key === 'Delete' && atEnd && isEmpty) {
        e.preventDefault()
        if (block.block_type.type === 'listItem') {
          onAction({ type: 'exit-list', blockId: block.id })
        } else if (shouldConvertWhenEmpty(block.block_type.type)) {
          onAction({ type: 'convert-block', blockId: block.id, content: '', blockType: { type: 'paragraph' } as BlockType })
        } else {
          onAction({ type: 'delete', blockId: block.id })
        }
        return
      }
    },
    [block.id, onAction, selectedBlockIds],
  )

  return { ref, handleInput, handleKeyDown, handlePaste, handleCompositionStart, handleCompositionEnd }
}
