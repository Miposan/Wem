/**
 * 选区管理器 — 统一管理编辑器中的光标和选区
 *
 * 集中所有 DOM 光标操作，提供可靠的 focus/scrollIntoView 能力。
 * 同时负责 contentEditable 块的显式 DOM 内容同步。
 *
 * 关键原则：
 * 1. 先立即尝试聚焦；若元素尚未渲染，通过 MutationObserver 等待其出现
 * 2. 聚焦后自动 scrollIntoView，确保光标所在块始终可见
 * 3. 通过 block ID + offset 定位，不依赖 DOM 结构
 */

import { domToMarkdown, inlineMarkdownToHtml, renderMathInElement } from './InlineParser'

/** 查找 block 对应的 contentEditable 元素（排除 contenteditable="false" 的折叠按钮等） */
export function findEditable(blockId: string): HTMLElement | null {
  return document.querySelector(
    `[data-block-id="${blockId}"] [contenteditable="true"]`,
  ) as HTMLElement | null
}

/** 查找 block 的可聚焦元素（文本块用 contentEditable，非文本块用 tabIndex 容器） */
export function findFocusable(blockId: string): HTMLElement | null {
  const editable = findEditable(blockId)
  if (editable) return editable
  return document.querySelector(
    `[data-block-id="${blockId}"] > .wem-block-content[tabindex]`,
  ) as HTMLElement | null
}

/**
 * 直接同步块的 contentEditable DOM 内容
 *
 * contentEditable 不受 React 控制，useTextBlock 只在 block.id 变化时同步。
 * 当 Command 修改了块内容但 block.id 没变时，需要显式同步 DOM。
 */
export function syncBlockContent(blockId: string, content: string): void {
  const el = findEditable(blockId)
  if (!el) return
  el.innerHTML = inlineMarkdownToHtml(content)
  renderMathInElement(el)
}

/**
 * 从 DOM 读取当前光标位置
 *
 * 返回当前聚焦的 contentEditable 块的 blockId 和字符偏移量。
 * 用于 Command 在执行时获取真实光标位置（而非 keydown 时捕获的过期值）。
 */
export function getCursorPosition(): { blockId: string; offset: number } | null {
  const active = document.activeElement as HTMLElement | null
  if (!active) return null

  if (active.isContentEditable) {
    const blockEl = active.closest('[data-block-id]')
    if (!blockEl) return null
    const blockId = blockEl.getAttribute('data-block-id')!

    const sel = window.getSelection()
    if (!sel || !sel.rangeCount) return null

    const range = sel.getRangeAt(0)
    const preRange = range.cloneRange()
    preRange.selectNodeContents(active)
    preRange.setEnd(range.startContainer, range.startOffset)
    const offset = preRange.toString().length

    return { blockId, offset }
  }

  // Non-text block focus (tabIndex container)
  if (active.classList.contains('wem-block-content')) {
    const blockEl = active.closest('[data-block-id]')
    if (!blockEl) return null
    const blockId = blockEl.getAttribute('data-block-id')!
    return { blockId, offset: 0 }
  }

  return null
}

/**
 * 在光标位置拆分 DOM 内容为 before / after 两段 markdown
 *
 * 使用 Range.cloneContents() 直接操作 DOM，正确处理行内格式元素。
 * 用于 executeSplit 代替 block.content 字符串切片。
 */
export function splitContentAtCursor(el: HTMLElement): { before: string; after: string } {
  const sel = window.getSelection()
  if (!sel || !sel.rangeCount) {
    return { before: domToMarkdown(el), after: '' }
  }

  const range = sel.getRangeAt(0)

  const beforeRange = document.createRange()
  beforeRange.selectNodeContents(el)
  beforeRange.setEnd(range.startContainer, range.startOffset)
  const beforeFrag = beforeRange.cloneContents()

  const afterRange = document.createRange()
  afterRange.selectNodeContents(el)
  afterRange.setStart(range.startContainer, range.startOffset)
  const afterFrag = afterRange.cloneContents()

  const tempBefore = document.createElement('div')
  tempBefore.appendChild(beforeFrag)
  const tempAfter = document.createElement('div')
  tempAfter.appendChild(afterFrag)

  return { before: domToMarkdown(tempBefore), after: domToMarkdown(tempAfter) }
}

/** Observer 超时时间（ms），防止永久等待 */
const FOCUS_OBSERVER_TIMEOUT_MS = 2_000

/** 在 contentEditable 元素中，将光标设置到指定文本偏移位置（支持行内格式元素） */
function setCursorAtTextOffset(el: HTMLElement, offset: number): void {
  const sel = window.getSelection()
  if (!sel) return

  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT)
  let current = 0
  let node: Text | null
  while ((node = walker.nextNode() as Text | null)) {
    const len = node.textContent?.length ?? 0
    if (current + len >= offset) {
      const range = document.createRange()
      range.setStart(node, offset - current)
      range.collapse(true)
      sel.removeAllRanges()
      sel.addRange(range)
      return
    }
    current += len
  }

  // Offset beyond content — place at end
  const range = document.createRange()
  range.selectNodeContents(el)
  range.collapse(false)
  sel.removeAllRanges()
  sel.addRange(range)
}

/**
 * 将光标设置到指定元素的目标偏移位置
 *
 * 辅助函数：执行 focus + 选区设置 + scrollIntoView。
 * 返回 true 表示焦点成功设置到目标元素。
 */
function placeCursor(el: HTMLElement, offset: number): boolean {
  const prev = document.activeElement as HTMLElement | null
  if (prev && prev !== el && (prev.isContentEditable || prev.getAttribute('tabindex') !== null)) {
    prev.blur()
  }

  el.focus()

  if (el.isContentEditable) {
    setCursorAtTextOffset(el, offset)
  }

  el.scrollIntoView({ block: 'nearest', behavior: 'instant' })
  return document.activeElement === el
}

/**
 * 聚焦指定块的指定字符偏移位置
 *
 * 立即尝试聚焦；若元素尚未渲染，通过 MutationObserver 等待其出现后聚焦。
 * 比固定次数 rAF 重试更可靠：元素出现即响应，无需猜测渲染时机。
 */
export function focusBlock(blockId: string, offset: number = 0): void {
  // 1. 立即尝试：元素已存在时直接聚焦
  const el = findFocusable(blockId)
  if (el) {
    placeCursor(el, offset)
    return
  }

  // 2. 元素不存在：通过 MutationObserver 等待其出现
  const observer = new MutationObserver((_mutations, obs) => {
    const target = findFocusable(blockId)
    if (target) {
      obs.disconnect()
      clearTimeout(timeout)
      placeCursor(target, offset)
    }
  })

  // 超时保护：防止永久等待（元素永远不会出现的情况）
  const timeout = setTimeout(() => {
    observer.disconnect()
    console.warn(`[focusBlock] 等待元素超时: ${blockId}`)
  }, FOCUS_OBSERVER_TIMEOUT_MS)

  // 观察整个 body 的子树变化（块可能插入到任何位置）
  observer.observe(document.body, { childList: true, subtree: true })
}

/** 聚焦指定块的末尾 */
export function focusBlockEnd(blockId: string): void {
  const el = findEditable(blockId)
  focusBlock(blockId, el?.textContent?.length ?? 0)
}
