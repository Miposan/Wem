import { useEffect, useRef, useState, useCallback } from 'react'
import { Bold, Italic, Underline, Code, Highlighter, Eraser } from 'lucide-react'
import { toggleInlineWrap, domToMarkdown, normalizeInline, removeAllFormats, renderMathInElement, findScrollParent } from '../core/InlineParser'

interface InlineToolbarProps {
  onContentChange: (blockId: string, content: string) => void
}

const GROUPS = [
  [
    { key: 'bold', label: '加粗', shortcut: 'Ctrl+B', icon: Bold, command: 'bold', tag: 'strong', altTag: 'b' },
    { key: 'italic', label: '斜体', shortcut: 'Ctrl+I', icon: Italic, command: 'italic', tag: 'em', altTag: 'i' },
    { key: 'underline', label: '下划线', shortcut: 'Ctrl+U', icon: Underline, command: 'underline', tag: 'u' },
  ],
  [
    { key: 'code', label: '行内代码', shortcut: 'Ctrl+E', icon: Code, tag: 'code' },
    { key: 'highlight', label: '高亮', shortcut: 'Ctrl+Shift+H', icon: Highlighter, tag: 'mark' },
  ],
  [
    { key: 'math', label: '行内公式', shortcut: 'Ctrl+M', tag: 'span', className: 'inline-math' },
  ],
  [
    { key: 'clear', label: '清除样式', shortcut: 'Ctrl+\\', icon: Eraser, action: 'clear' as const },
  ],
] as const

type FlatFormat = (typeof GROUPS)[number][number]
const ALL_FORMATS: readonly FlatFormat[] = GROUPS.flat()

function MathIcon({ size = 15 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <text x="4" y="18" fontSize="16" fill="currentColor" stroke="none" fontFamily="serif">&#x2211;</text>
    </svg>
  )
}

const ICON_MAP: Record<string, React.FC<{ size?: number }>> = { math: MathIcon }

function isFormatActive(
  sel: Selection,
  opts: { tag: string; altTag?: string; className?: string },
): boolean {
  for (let node: Node | null = sel.anchorNode; node; node = node.parentNode) {
    if (node instanceof HTMLElement) {
      const t = node.tagName.toLowerCase()
      if (t === opts.tag || (opts.altTag && t === opts.altTag)) {
        if (!opts.className || node.classList.contains(opts.className)) return true
      }
      if (node.hasAttribute('contenteditable')) break
    }
  }
  return false
}

export function InlineToolbar({ onContentChange }: InlineToolbarProps) {
  const [visible, setVisible] = useState(false)
  const [activeFormats, setActiveFormats] = useState<Set<string>>(new Set())
  const [flipDown, setFlipDown] = useState(false)
  const [pos, setPos] = useState({ top: 0, left: 0 })
  const rootRef = useRef<HTMLElement | null>(null)
  const toolbarRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    rootRef.current = document.querySelector('.wem-editor-root')
  }, [])

  /** Show toolbar at current selection position */
  const show = useCallback(() => {
    const sel = window.getSelection()
    if (!sel || sel.isCollapsed || sel.rangeCount === 0) return

    const range = sel.getRangeAt(0)
    const root = rootRef.current
    if (!root || !root.contains(range.commonAncestorContainer)) return

    const anchorEl = sel.anchorNode instanceof HTMLElement ? sel.anchorNode : sel.anchorNode?.parentElement
    const focusEl = sel.focusNode instanceof HTMLElement ? sel.focusNode : sel.focusNode?.parentElement
    const anchorEdit = anchorEl?.closest('[contenteditable="true"]')
    const focusEdit = focusEl?.closest('[contenteditable="true"]')
    if (!anchorEdit || anchorEdit !== focusEdit) return

    const rect = range.getBoundingClientRect()
    if (rect.width < 2) return

    const formats = new Set<string>()
    for (const fmt of ALL_FORMATS) {
      if (isFormatActive(sel, fmt)) formats.add(fmt.key)
    }
    setActiveFormats(formats)

    const gap = 6
    const barH = 36
    const fd = rect.top < barH + gap + 8
    const top = fd ? rect.bottom + gap : rect.top - barH - gap
    const left = rect.left + rect.width / 2

    setFlipDown(fd)
    setPos({ top, left })
    setVisible(true)
  }, [])

  /** Directly update toolbar position from current selection rect (no React render) */
  const syncPosition = useCallback(() => {
    const el = toolbarRef.current
    if (!el) return false

    const sel = window.getSelection()
    if (!sel || sel.isCollapsed || sel.rangeCount === 0) {
      el.style.display = 'none'
      return false
    }

    const range = sel.getRangeAt(0)
    const rect = range.getBoundingClientRect()
    const gap = 6
    const barH = 36
    const fd = rect.top < barH + gap + 8
    el.style.display = ''
    el.style.top = `${fd ? rect.bottom + gap : rect.top - barH - gap}px`
    el.style.left = `${rect.left + rect.width / 2}px`
    el.classList.toggle('flip-down', fd)
    return true
  }, [])

  useEffect(() => {
    const handleSelectionEnd = () => requestAnimationFrame(() => show())
    const handleSelectionChange = () => {
      if (!rootRef.current?.contains(document.activeElement)) return
      const sel = window.getSelection()
      if (!sel || sel.isCollapsed) setVisible(false)
    }

    document.addEventListener('mouseup', handleSelectionEnd)
    document.addEventListener('selectionchange', handleSelectionChange)
    window.addEventListener('resize', handleSelectionChange)

    const scrollParent = rootRef.current ? findScrollParent(rootRef.current) : null
    const handleScroll = () => {
      if (!toolbarRef.current) return
      const range = window.getSelection()?.getRangeAt(0)
      if (!range) { toolbarRef.current.style.display = 'none'; return }
      const rect = range.getBoundingClientRect()
      const parentRect = scrollParent?.getBoundingClientRect()
      if (parentRect && (rect.bottom < parentRect.top || rect.top > parentRect.bottom)) {
        toolbarRef.current.style.display = 'none'
        return
      }
      syncPosition()
    }
    scrollParent?.addEventListener('scroll', handleScroll, { passive: true })

    return () => {
      document.removeEventListener('mouseup', handleSelectionEnd)
      document.removeEventListener('selectionchange', handleSelectionChange)
      window.removeEventListener('resize', handleSelectionChange)
      scrollParent?.removeEventListener('scroll', handleScroll)
    }
  }, [show, syncPosition])

  const applyFormat = useCallback(
    (fmt: FlatFormat) => {
      const sel = window.getSelection()
      if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return

      const anchorEl = sel.anchorNode instanceof HTMLElement ? sel.anchorNode : sel.anchorNode?.parentElement
      const editable = anchorEl?.closest('[contenteditable="true"]') as HTMLElement | null
      if (!editable) return

      if ('action' in fmt && fmt.action === 'clear') {
        removeAllFormats(editable)
      } else if ('command' in fmt && fmt.command) {
        document.execCommand(fmt.command)
      } else {
        toggleInlineWrap(editable, fmt.tag, fmt.className)
      }

      if (fmt.key === 'math') renderMathInElement(editable)
      normalizeInline(editable)

      const blockEl = editable.closest('[data-block-id]')
      const blockId = blockEl?.getAttribute('data-block-id')
      if (blockId) onContentChange(blockId, domToMarkdown(editable))

      // Re-sync position + active formats after DOM change
      show()
    },
    [onContentChange, show],
  )

  if (!visible) return null

  return (
    <div
      ref={toolbarRef}
      className={`wem-inline-toolbar${flipDown ? ' flip-down' : ''}`}
      style={{ position: 'fixed', top: pos.top, left: pos.left }}
      onMouseDown={e => e.preventDefault()}
    >
      {GROUPS.map((group, gi) => (
        <div className="wem-inline-toolbar-group" key={gi}>
          {gi > 0 && <div className="wem-inline-toolbar-divider" />}
          {group.map((fmt) => {
            const Icon = ICON_MAP[fmt.key] ?? fmt.icon
            const isActive = activeFormats.has(fmt.key)
            return (
              <button
                key={fmt.key}
                className={`wem-inline-toolbar-btn${isActive ? ' active' : ''}`}
                onMouseDown={e => { e.preventDefault(); applyFormat(fmt) }}
                title={`${fmt.label} ${fmt.shortcut}`}
              >
                <Icon size={15} />
              </button>
            )
          })}
        </div>
      ))}
    </div>
  )
}
