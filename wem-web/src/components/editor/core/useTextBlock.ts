import { useCallback, useEffect, useRef } from 'react'
import { makeHeadingType, makeListType, makeCodeBlockType, makeMathBlockType, makeBlockquoteType, makeThematicBreakType, makeParagraphType } from '@/types/api'
import type { TextBlockProps } from './types'
import { useSlashMenuDispatch, type SlashMenuItem } from './SlashMenuContext'
import { focusBlock } from './SelectionManager'
import { inlineMarkdownToHtml, domToMarkdown, toggleInlineWrap, removeAllFormats, normalizeInline, removeTextRange, renderMathInElement } from './InlineParser'

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
 * 抽取 ParagraphBlock / HeadingBlock / ListItem 等的共同行为：
 * - mount/块切换时同步 DOM 内容
 * - input → onContentChange
 * - 键盘操作 → onAction（Enter/Backspace/ArrowUp/ArrowDown）
 * - 斜杠命令菜单（/ 触发）
 */
export function useTextBlock({ block, onContentChange, onAction, selectedBlockIds }: TextBlockProps) {
  const ref = useRef<HTMLElement>(null)

  // ── IME 组合输入守卫 ──
  const isComposing = useRef(false)

  // format 快捷键已主动调用 onContentChange，需跳过后续 input 事件触发的重复调用
  const skipInput = useRef(false)

  const handleCompositionStart = useCallback(() => {
    isComposing.current = true
  }, [])

  const handleCompositionEnd = useCallback(() => {
    isComposing.current = false
    const el = ref.current
    if (!el) return
    onContentChange(block.id, domToMarkdown(el))
  }, [block.id, onContentChange])

  // 仅在 block.id 变化时同步 DOM（将 markdown 解析为 HTML）
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const html = inlineMarkdownToHtml(block.content ?? '')
    el.innerHTML = html
    renderMathInElement(el)
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [block.id])

  // ── 斜杠命令 ──
  // 只订阅 dispatch（stable，不随 state 变化而 re-render）
  const slashMenu = useSlashMenuDispatch()

  const slashPending = useRef(false)
  const slashPendingOffset = useRef(0)

  // 稳定的 onAction ref
  const onActionRef = useRef(onAction)
  onActionRef.current = onAction

  const handleSlashSelect = useCallback((item: SlashMenuItem) => {
    const el = ref.current
    if (!el) return
    const st = slashMenu.getState()
    const offset = st.slashOffset
    const filterLen = st.filter.length
    // Remove "/filter" from DOM preserving inline formatting
    removeTextRange(el, offset, offset + 1 + filterLen)
    slashMenu.close()
    onActionRef.current({
      type: 'convert-block',
      blockId: block.id,
      content: domToMarkdown(el),
      blockType: item.blockType,
    })
    setTimeout(() => focusBlock(block.id, 0), 0)
  }, [block.id])

  // ── 粘贴 → 解析行内 markdown，保留格式 ──

  const handlePaste = useCallback((e: React.ClipboardEvent) => {
    const text = e.clipboardData.getData('text/plain')
    if (!text) return
    e.preventDefault()
    if (/[*`$=+]/.test(text)) {
      document.execCommand('insertHTML', false, inlineMarkdownToHtml(text))
      const el = ref.current
      if (el) renderMathInElement(el)
    } else {
      document.execCommand('insertText', false, text)
    }
  }, [])

  const handleInput = useCallback(() => {
    if (isComposing.current) return
    if (skipInput.current) { skipInput.current = false; return }
    const el = ref.current
    if (!el) return

    // InlineToolbar applyFormat 的 DOM 操作也会触发 input 事件
    if (el.dataset.skipInput) {
      delete el.dataset.skipInput
      return
    }

    onContentChange(block.id, domToMarkdown(el))

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
        slashMenu.trigger({
          blockId: block.id,
          x: rect.left,
          y: rect.bottom,
          slashOffset: slashPendingOffset.current,
        })
      }
      return
    }

    // 斜杠菜单已打开：更新过滤文字
    const smState = slashMenu.getState()
    if (smState.visible && smState.blockId === block.id) {
      const text = el.textContent || ''
      const cursorOffset = getCursorOffset(el)
      if (text[smState.slashOffset] !== '/') {
        slashMenu.close()
      } else {
        slashMenu.setFilter(text.slice(smState.slashOffset + 1, cursorOffset))
      }
    }
  }, [block.id, onContentChange])

  const handleEmptyBlockAction = useCallback((mode: 'backspace' | 'delete' = 'backspace') => {
    const t = block.block_type.type
    if (t === 'listItem') {
      onActionRef.current({ type: mode === 'delete' ? 'exit-list' : 'outdent-list-item', blockId: block.id })
    } else if (shouldConvertWhenEmpty(t)) {
      onActionRef.current({ type: 'convert-block', blockId: block.id, content: '', blockType: makeParagraphType() })
    } else {
      onActionRef.current({ type: 'delete', blockId: block.id })
    }
  }, [block.id, block.block_type.type])

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      const el = ref.current
      if (!el) return

      if (isComposing.current) return

      // ── 行内格式快捷键 ──
      if ((e.ctrlKey || e.metaKey) && !e.altKey) {
        const execCommands: Record<string, string> = { b: 'bold', i: 'italic', u: 'underline' }
        if (execCommands[e.key]) {
          e.preventDefault()
          skipInput.current = true
          document.execCommand(execCommands[e.key])
          normalizeInline(el)
          onContentChange(block.id, domToMarkdown(el))
          return
        }
        if (e.key === 'e' && !e.shiftKey) {
          e.preventDefault()
          skipInput.current = true
          toggleInlineWrap(el, 'code')
          onContentChange(block.id, domToMarkdown(el))
          return
        }
        if (e.shiftKey && e.key === 'H') {
          e.preventDefault()
          skipInput.current = true
          toggleInlineWrap(el, 'mark')
          onContentChange(block.id, domToMarkdown(el))
          return
        }
        if (e.key === 'm') {
          e.preventDefault()
          skipInput.current = true
          toggleInlineWrap(el, 'span', 'inline-math')
          renderMathInElement(el)
          onContentChange(block.id, domToMarkdown(el))
          return
        }
        // Ctrl+\ → clear all formatting
        if (e.key === '\\') {
          e.preventDefault()
          skipInput.current = true
          removeAllFormats(el)
          normalizeInline(el)
          onContentChange(block.id, domToMarkdown(el))
          return
        }
      }

      const sm = slashMenuRef.current

      // ── 斜杠菜单键盘交互 ──
      if (sm?.state.visible && sm.state.blockId === block.id) {
        if (e.key === 'ArrowDown') { e.preventDefault(); sm.navigate('down'); return }
        if (e.key === 'ArrowUp') { e.preventDefault(); sm.navigate('up'); return }
        if (e.key === 'Enter') {
          e.preventDefault()
          const item = sm.filteredItems[sm.state.selectedIndex]
          if (item) handleSlashSelect(item)
          return
        }
        if (e.key === 'Escape') { e.preventDefault(); sm.close(); return }
        // 其他键继续传递给正常处理（用于更新过滤文字）
      }

      // ── 跨块选区 Backspace / Delete → delete-range ──
      if ((e.key === 'Backspace' || e.key === 'Delete') && selectedBlockIds.size > 1) {
        e.preventDefault()
        onActionRef.current({ type: 'delete-range', blockIds: Array.from(selectedBlockIds) })
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
        onActionRef.current({ type: e.shiftKey ? 'outdent-list-item' : 'indent-list-item', blockId: block.id })
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
          removeTextRange(el, 0, offset)
          onActionRef.current({ type: 'convert-block', blockId: block.id, content: domToMarkdown(el), blockType: makeHeadingType(headingMatch[1].length) })
          return
        }

        const ulMatch = beforeCursor.match(/^[-*]$/)
        if (ulMatch) {
          e.preventDefault()
          removeTextRange(el, 0, offset)
          onActionRef.current({ type: 'convert-block', blockId: block.id, content: domToMarkdown(el), blockType: makeListType(false) })
          return
        }

        const olMatch = beforeCursor.match(/^(\d+)\.$/)
        if (olMatch) {
          e.preventDefault()
          removeTextRange(el, 0, offset)
          onActionRef.current({ type: 'convert-block', blockId: block.id, content: domToMarkdown(el), blockType: makeListType(true) })
          return
        }

        if (beforeCursor === '>') {
          e.preventDefault()
          removeTextRange(el, 0, offset)
          onActionRef.current({ type: 'convert-block', blockId: block.id, content: domToMarkdown(el), blockType: makeBlockquoteType() })
          return
        }

        const codeMatch = beforeCursor.match(/^```(\w*)$/)
        if (codeMatch) {
          e.preventDefault()
          removeTextRange(el, 0, offset)
          onActionRef.current({ type: 'convert-block', blockId: block.id, content: domToMarkdown(el), blockType: makeCodeBlockType(codeMatch[1] || 'text') })
          return
        }

        if (beforeCursor === '$$') {
          e.preventDefault()
          el.textContent = ''
          onActionRef.current({ type: 'convert-block', blockId: block.id, content: '', blockType: makeMathBlockType() })
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
        onActionRef.current({ type: 'convert-block', blockId: block.id, content: '', blockType: makeThematicBreakType() })
        return
      }

      // Enter → 拆分
      if (e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
        e.preventDefault()
        if (block.block_type.type === 'listItem' && isEmpty) {
          onActionRef.current({ type: 'exit-list', blockId: block.id })
        } else if (isEmpty && shouldConvertWhenEmpty(block.block_type.type)) {
          onActionRef.current({ type: 'convert-block', blockId: block.id, content: '', blockType: makeParagraphType() })
        } else {
          onActionRef.current({ type: 'split' })
        }
        return
      }

      // Backspace at start of empty → 转换或删除块
      if (e.key === 'Backspace' && atStart && isEmpty) {
        e.preventDefault()
        handleEmptyBlockAction()
        return
      }

      // Backspace at start with content → 与前块合并
      if (e.key === 'Backspace' && atStart && !isEmpty) {
        e.preventDefault()
        onActionRef.current({ type: 'merge-with-previous', blockId: block.id })
        return
      }

      if (e.key === 'ArrowUp' && atStart) { e.preventDefault(); onActionRef.current({ type: 'focus-previous', blockId: block.id }); return }
      if (e.key === 'ArrowDown' && atEnd) { e.preventDefault(); onActionRef.current({ type: 'focus-next', blockId: block.id }); return }
      if (e.key === 'ArrowLeft' && atStart) { e.preventDefault(); onActionRef.current({ type: 'focus-previous', blockId: block.id }); return }
      if (e.key === 'ArrowRight' && atEnd) { e.preventDefault(); onActionRef.current({ type: 'focus-next', blockId: block.id }); return }

      // Delete at end → 与下一块合并
      if (e.key === 'Delete' && atEnd && !isEmpty) {
        e.preventDefault()
        onActionRef.current({ type: 'merge-with-next', blockId: block.id })
        return
      }

      // Delete at end of empty
      if (e.key === 'Delete' && atEnd && isEmpty) {
        e.preventDefault()
        handleEmptyBlockAction('delete')
        return
      }
    },
    [block.id, block.block_type.type, selectedBlockIds, handleSlashSelect, handleEmptyBlockAction, onContentChange],
  )

  return { ref, handleInput, handleKeyDown, handlePaste, handleCompositionStart, handleCompositionEnd }
}
