/**
 * MathBlock — 公式块（KaTeX 渲染）
 *
 * content 存储 LaTeX 源码，点击进入编辑模式，失焦时渲染为公式。
 */

import { useState, useRef, useEffect, useCallback } from 'react'
import katex from 'katex'

interface MathBlockProps {
  block: {
    id: string
    content: string | null
  }
  readonly: boolean
  onContentChange: (blockId: string, content: string) => void
}

export function MathBlock({ block, readonly, onContentChange }: MathBlockProps) {
  const [editing, setEditing] = useState(false)
  const [source, setSource] = useState(block.content ?? '')
  const renderRef = useRef<HTMLDivElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  // 渲染 KaTeX
  useEffect(() => {
    if (editing || !renderRef.current) return
    try {
      katex.render(source || '\\text{空公式}', renderRef.current, {
        displayMode: true,
        throwOnError: false,
      })
    } catch {
      renderRef.current.textContent = source
    }
  }, [editing, source])

  // 同步外部 content 变更
  useEffect(() => {
    if (!editing) setSource(block.content ?? '')
  }, [block.content, editing])

  const handleFinishEdit = useCallback(() => {
    setEditing(false)
    if (source !== (block.content ?? '')) {
      onContentChange(block.id, source)
    }
  }, [block.id, block.content, source, onContentChange])

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      handleFinishEdit()
    }
    // Ctrl/Cmd+Enter 也退出编辑
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault()
      handleFinishEdit()
    }
  }, [handleFinishEdit])

  if (editing && !readonly) {
    return (
      <div className="wem-mathblock wem-mathblock-editing">
        <textarea
          ref={textareaRef}
          className="wem-mathblock-editor"
          value={source}
          onChange={(e) => setSource(e.target.value)}
          onBlur={handleFinishEdit}
          onKeyDown={handleKeyDown}
          placeholder="输入 LaTeX 公式…"
          rows={3}
          autoFocus
        />
      </div>
    )
  }

  return (
    <div
      className="wem-mathblock"
      onClick={() => { if (!readonly) setEditing(true) }}
      role={readonly ? undefined : 'button'}
      tabIndex={-1}
    >
      <div ref={renderRef} className="wem-mathblock-render" />
      {!source && !readonly && (
        <span className="wem-mathblock-placeholder">点击输入公式…</span>
      )}
    </div>
  )
}
