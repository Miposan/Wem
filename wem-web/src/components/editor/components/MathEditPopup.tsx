import { useState, useRef, useEffect, useCallback } from 'react'
import katex from 'katex'
import { domToMarkdown, renderMathSpan, findScrollParent } from '../core/InlineParser'

interface MathEditPopupProps {
  onContentChange: (blockId: string, content: string) => void
}

interface PopupData {
  element: HTMLElement
  originalSource: string
}

export function MathEditPopup({ onContentChange }: MathEditPopupProps) {
  const [popup, setPopup] = useState<PopupData | null>(null)
  const [source, setSource] = useState('')
  const rootRef = useRef<HTMLElement | null>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const previewRef = useRef<HTMLDivElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)

  const liveRef = useRef({ popup: null as PopupData | null, source: '' })
  liveRef.current = { popup, source }

  useEffect(() => {
    rootRef.current = document.querySelector('.wem-editor-root')
  }, [])

  const doSave = useCallback(() => {
    const { popup: data, source: currentSource } = liveRef.current
    if (!data) return
    const { element: el, originalSource } = data
    if (!el.parentElement || currentSource === originalSource) return

    const editable = el.closest('[contenteditable="true"]') as HTMLElement | null
    const blockEl = el.closest('[data-block-id]')
    const blockId = blockEl?.getAttribute('data-block-id')

    if (currentSource) {
      renderMathSpan(el, currentSource)
    } else {
      renderMathSpan(el, '')
    }

    if (blockId && editable) {
      onContentChange(blockId, domToMarkdown(editable))
    }
  }, [onContentChange])

  // Position popup directly via DOM (immediate, no React render)
  const syncPosition = useCallback((mathEl: HTMLElement) => {
    const el = containerRef.current
    if (!el) return
    const rect = mathEl.getBoundingClientRect()
    el.style.top = `${rect.bottom + 6}px`
    el.style.left = `${rect.left + rect.width / 2}px`
  }, [])

  // Detect clicks on rendered inline-math → open popup
  useEffect(() => {
    const handleMouseDown = (e: MouseEvent) => {
      const target = e.target as HTMLElement
      const mathEl = target.closest('.inline-math[data-render="true"]') as HTMLElement | null
      if (!mathEl || !rootRef.current?.contains(mathEl)) return

      e.preventDefault()
      doSave()

      const latex = mathEl.getAttribute('data-content') || ''

      setPopup({ element: mathEl, originalSource: latex })
      setSource(latex)

      // Position after React renders the element
      requestAnimationFrame(() => {
        syncPosition(mathEl)
        textareaRef.current?.focus()
        textareaRef.current?.select()
      })
    }

    document.addEventListener('mousedown', handleMouseDown, true)
    return () => document.removeEventListener('mousedown', handleMouseDown, true)
  }, [doSave, syncPosition])

  // Scroll: direct DOM reposition; hide if element leaves viewport, restore on return
  useEffect(() => {
    if (!popup) return
    const el = popup.element
    const scrollParent = findScrollParent(el)

    const handleScroll = () => {
      const popupEl = containerRef.current
      if (!popupEl) return
      if (!el.parentElement) { doSave(); setPopup(null); return }
      const rect = el.getBoundingClientRect()
      const parentRect = scrollParent?.getBoundingClientRect()
      if (parentRect && (rect.bottom < parentRect.top || rect.top > parentRect.bottom)) {
        popupEl.style.display = 'none'
        return
      }
      popupEl.style.display = ''
      syncPosition(el)
    }

    scrollParent?.addEventListener('scroll', handleScroll, { passive: true })
    return () => scrollParent?.removeEventListener('scroll', handleScroll)
  }, [popup, doSave, syncPosition])

  // Click outside popup → save and close
  useEffect(() => {
    if (!popup) return

    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current?.contains(e.target as Node)) return
      const target = e.target as HTMLElement
      if (target.closest('.inline-math[data-render="true"]')) return
      doSave()
      setPopup(null)
    }

    document.addEventListener('mousedown', handleClickOutside, true)
    return () => document.removeEventListener('mousedown', handleClickOutside, true)
  }, [popup, doSave])

  // Render KaTeX preview
  useEffect(() => {
    if (!popup || !previewRef.current) return
    try {
      katex.render(source || '\\ ', previewRef.current, { throwOnError: false })
    } catch {
      previewRef.current.textContent = source || '(empty)'
    }
  }, [popup, source])

  const close = useCallback(() => {
    doSave()
    setPopup(null)
  }, [doSave])

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    e.stopPropagation()
    if (e.key === 'Escape') {
      e.preventDefault()
      close()
    }
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault()
      close()
    }
  }, [close])

  if (!popup) return null

  return (
    <div
      ref={containerRef}
      className="wem-math-popup"
      style={{ position: 'fixed' }}
    >
      <div className="wem-math-popup-header">
        <span>LaTeX</span>
        <button
          className="wem-math-popup-close"
          onMouseDown={(e) => { e.preventDefault(); e.stopPropagation(); close() }}
        >
          ×
        </button>
      </div>
      <textarea
        ref={textareaRef}
        className="wem-math-popup-editor"
        value={source}
        onChange={(e) => setSource(e.target.value)}
        onKeyDown={handleKeyDown}
        rows={2}
      />
      <div ref={previewRef} className="wem-math-popup-preview" />
    </div>
  )
}
