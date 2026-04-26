import { useCallback, useEffect, useRef } from 'react'
import { makeHeadingType, makeListType, makeCodeBlockType, makeMathBlockType } from '@/types/api'
import type { BlockType } from '@/types/api'
import type { TextBlockProps } from './types'
import { useSlashMenu, type SlashMenuItem } from './SlashMenuContext'
import { focusBlock } from './SelectionManager'

/** 空块按 Enter/Backspace 应转为 paragraph 而非删除的类型 */
function shouldConvertWhenEmpty(blockType: string): boolean {
  return blockType === 'heading' || blockType === 'blockquote'
}

function getCursorOffset(el: HTMLElement): number {
  const sel = window.getSelection()
  if (!sel || !sel.rangeCount) return 0
  const range = sel.getRangeAt(0)
  const preRange = range.cloneRange()
  preRange.selectNodeContents(el)
  preRange.setEnd(range.startContainer, range.startOffset)
  return preRange.toString().length
}

/**
 * 文本块共享逻辑 hook
 *
 * 抽取 ParagraphBlock / HeadingBlock / 未来的 ListItem 等的共同行为：
 * - mount/块切换时同步 DOM 内容
 * - input → onContentChange
 * - 键盘操作 → onAction（Enter/Backspace/ArrowUp/ArrowDown）
 * - 斜杠命令菜单（/ 触发）
 */
export function useTextBlock({ block, onContentChange, onAction, selectedBlockIds }: TextBlockProps) {
  const ref = useRef<HTMLElement>(null)

  // ── IME 组合输入守卫 ──
  const isComposing = useRef(false)

  const handleCompositionStart = useCallback(() => {
    isComposing.current = true
  }, [])

  const handleCompositionEnd = useCallback(() => {
    isComposing.current = false
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

  // ── 斜杠命令 ──

  const slashMenu = useSlashMenu()
  const slashPending = useRef(false)
  const slashPendingOffset = useRef(0)

  const handleSlashSelect = useCallback((item: SlashMenuItem) => {
    const el = ref.current
    if (!el || !slashMenu) return
    const text = el.textContent || ''
    const offset = slashMenu.state.slashOffset
    const filterLen = slashMenu.state.filter.length
    const before = text.slice(0, offset)
    const after = text.slice(offset + 1 + filterLen)
    const newContent = before + after
    el.textContent = newContent
    slashMenu.close()
    onAction({
      type: 'convert-block',
      blockId: block.id,
      content: newContent,
      blockType: item.blockType,
    })
    setTimeout(() => focusBlock(block.id, 0), 0)
  }, [block.id, onAction, slashMenu])

  // ── 粘贴 → 剥离格式，仅保留纯文本 ──

  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    const text = e.clipboardData.getData('text/plain')
    if (!text) return
    e.preventDefault()
    document.execCommand('insertText', false, text)
  }, [])

  const handleInput = useCallback(() => {
    if (isComposing.current) return
    const el = ref.current
    if (!el) return

    onContentChange(block.id, el.textContent || '')

    // 斜杠命令：首次输入 / 触发菜单
    if (slashPending.current) {
      slashPending.current = false
      const sel = window.getSelection()
      if (sel && sel.rangeCount > 0) {
        const range = sel.getRangeAt(0)
        let rect = range.getBoundingClientRect()
        // 空块时光标 rect 可能为零，回退到元素位置
        if (rect.width === 0 && rect.height === 0) {
          rect = el.getBoundingClientRect()
        }
        slashMenu?.trigger({
          blockId: block.id,
          x: rect.left,
          y: rect.bottom,
          slashOffset: slashPendingOffset.current,
        })
      }
      return
    }

    // 斜杠菜单已打开：更新过滤文字
    if (slashMenu?.state.visible && slashMenu.state.blockId === block.id) {
      const text = el.textContent || ''
      const offset = getCursorOffset(el)
      if (text[slashMenu.state.slashOffset] !== '/') {
        slashMenu.close()
      } else {
        slashMenu.setFilter(text.slice(slashMenu.state.slashOffset + 1, offset))
      }
    }
  }, [block.id, onContentChange, slashMenu])

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const el = ref.current
      if (!el) return

      if (isComposing.current) return

      // ── 斜杠菜单键盘交互 ──
      if (slashMenu?.state.visible && slashMenu.state.blockId === block.id) {
        if (e.key === 'ArrowDown') { e.preventDefault(); slashMenu.navigate('down'); return }
        if (e.key === 'ArrowUp') { e.preventDefault(); slashMenu.navigate('up'); return }
        if (e.key === 'Enter') {
          e.preventDefault()
          const item = slashMenu.filteredItems[slashMenu.state.selectedIndex]
          if (item) handleSlashSelect(item)
          return
        }
        if (e.key === 'Escape') { e.preventDefault(); slashMenu.close(); return }
        // 其他键继续传递给正常处理（用于更新过滤文字）
      }

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

      const preRange = range.cloneRange()
      preRange.selectNodeContents(el)
      preRange.setEnd(range.startContainer, range.startOffset)
      const offset = preRange.toString().length

      const atStart = offset === 0
      const atEnd = offset === text.length
      const isEmpty = text.length === 0

      if (!sel.isCollapsed && (e.key === 'Backspace' || e.key === 'Delete')) {
        return
      }

      // ListItem: Tab / Shift+Tab 调整列表层级
      if (e.key === 'Tab' && block.block_type.type === 'listItem') {
        e.preventDefault()
        onAction({ type: e.shiftKey ? 'outdent-list-item' : 'indent-list-item', blockId: block.id })
        return
      }

      // ── / 触发斜杠菜单 ──
      if (e.key === '/') {
        slashPending.current = true
        slashPendingOffset.current = offset
        // 不阻止默认行为，让 / 插入到文本中，在 handleInput 中触发菜单
      }

      // ── Markdown 快捷键（仅对 paragraph 块生效）──
      if (e.key === ' ' && block.block_type.type === 'paragraph') {
        const beforeCursor = text.slice(0, offset)

        const headingMatch = beforeCursor.match(/^(#{1,6})$/)
        if (headingMatch) {
          e.preventDefault()
          const rest = text.slice(offset).replace(/^\s+/, '')
          el.textContent = rest
          onAction({ type: 'convert-block', blockId: block.id, content: rest, blockType: makeHeadingType(headingMatch[1].length) })
          return
        }

        const ulMatch = beforeCursor.match(/^[-*]$/)
        if (ulMatch) {
          e.preventDefault()
          const rest = text.slice(offset).replace(/^\s+/, '')
          el.textContent = rest
          onAction({ type: 'convert-block', blockId: block.id, content: rest, blockType: makeListType(false) })
          return
        }

        const olMatch = beforeCursor.match(/^(\d+)\.$/)
        if (olMatch) {
          e.preventDefault()
          const rest = text.slice(offset).replace(/^\s+/, '')
          el.textContent = rest
          onAction({ type: 'convert-block', blockId: block.id, content: rest, blockType: makeListType(true) })
          return
        }

        if (beforeCursor === '>') {
          e.preventDefault()
          const rest = text.slice(offset).replace(/^\s+/, '')
          el.textContent = rest
          onAction({ type: 'convert-block', blockId: block.id, content: rest, blockType: { type: 'blockquote' } as BlockType })
          return
        }
      }

      // ``` + Space → CodeBlock
      if (e.key === ' ' && block.block_type.type === 'paragraph') {
        const beforeCursor = text.slice(0, offset)
        const codeMatch = beforeCursor.match(/^```(\w*)$/)
        if (codeMatch) {
          e.preventDefault()
          const rest = text.slice(offset).replace(/^\s+/, '')
          el.textContent = rest
          onAction({ type: 'convert-block', blockId: block.id, content: rest, blockType: makeCodeBlockType(codeMatch[1] || 'text') })
          return
        }
        if (beforeCursor === '$$') {
          e.preventDefault()
          el.textContent = ''
          onAction({ type: 'convert-block', blockId: block.id, content: '', blockType: makeMathBlockType() })
          return
        }
      }

      // --- / *** / ___ + Enter → 分割线
      if (
        e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.metaKey &&
        block.block_type.type === 'paragraph' &&
        /^(---|\*\*\*|___)$/.test(text.trim())
      ) {
        e.preventDefault()
        el.textContent = ''
        onAction({ type: 'convert-block', blockId: block.id, content: '', blockType: { type: 'thematicBreak' } as BlockType })
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

      if (e.key === 'ArrowUp' && atStart) { e.preventDefault(); onAction({ type: 'focus-previous', blockId: block.id }); return }
      if (e.key === 'ArrowDown' && atEnd) { e.preventDefault(); onAction({ type: 'focus-next', blockId: block.id }); return }
      if (e.key === 'ArrowLeft' && atStart) { e.preventDefault(); onAction({ type: 'focus-previous', blockId: block.id }); return }
      if (e.key === 'ArrowRight' && atEnd) { e.preventDefault(); onAction({ type: 'focus-next', blockId: block.id }); return }

      // Delete at end → 与下一块合并
      if (e.key === 'Delete' && atEnd && !isEmpty) {
        e.preventDefault()
        onAction({ type: 'merge-with-next', blockId: block.id })
        return
      }

      // Delete at end of empty
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
    [block.id, block.block_type.type, onAction, selectedBlockIds, slashMenu, handleSlashSelect],
  )

  return { ref, handleInput, handleKeyDown, handlePaste, handleCompositionStart, handleCompositionEnd }
}
