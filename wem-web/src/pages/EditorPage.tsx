import { useEffect, useRef, useState, useCallback } from 'react'
import { getDocument, updateBlock } from '@/api/client'
import type { BlockNode } from '@/types/api'
import { WemEditor } from '@/components/editor'
import { EmojiPicker } from '@/components/editor/components/EmojiPicker'
import { Breadcrumb, extractTocItems, type TocItem } from '@/components/layout'
import { useTabStore } from '@/stores/tabStore'
import '@/components/editor/editor.css'

interface Props {
  documentId: string | null
  onTocItemsChange?: (items: TocItem[]) => void
  onNavigate?: (id: string, title: string, icon: string) => void
}

// ─── 文档加载状态 ───
type DocData =
  | { status: 'idle' }
  | { status: 'loading'; docId: string }
  | { status: 'loaded'; docId: string; title: string; icon: string | undefined; tree: BlockNode[] }
  | { status: 'error'; docId: string; error: unknown }

export default function EditorPage({ documentId, onTocItemsChange, onNavigate }: Props) {
  const [doc, setDoc] = useState<DocData>({ status: 'idle' })
  const { updateTab } = useTabStore()
  const tocTimerRef = useRef<ReturnType<typeof setTimeout>>()

  const handleTitleBlur = useCallback(
    (e: React.FocusEvent<HTMLHeadingElement>) => {
      const newTitle = e.currentTarget.textContent || ''
      if (documentId) {
        setDoc((prev) => prev.status === 'loaded' ? { ...prev, title: newTitle } : prev)
        updateBlock(documentId, {
          properties: { title: newTitle },
          properties_mode: 'merge',
        }).catch((err) => console.error('标题保存失败:', err))
        updateTab(documentId, { title: newTitle })
        window.dispatchEvent(new CustomEvent('wem:doc-title-change', { detail: { id: documentId, title: newTitle } }))
      }
    },
    [documentId, updateTab],
  )

  /** Emoji 图标变更 */
  const handleIconChange = useCallback(
    (newIcon: string | undefined) => {
      setDoc((prev) =>
        prev.status === 'loaded' ? { ...prev, icon: newIcon } : prev,
      )
      if (documentId) {
        updateBlock(documentId, {
          properties: { ...(newIcon ? { icon: newIcon } : {}) },
          properties_mode: 'merge',
        }).catch((err) => console.error('图标保存失败:', err))
        updateTab(documentId, { icon: newIcon || '📄' })
      }
    },
    [documentId, updateTab],
  )

  /** 编辑器树变更 → 防抖更新 TOC */
  const handleTreeChange = useCallback((tree: BlockNode[]) => {
    if (tocTimerRef.current) clearTimeout(tocTimerRef.current)
    tocTimerRef.current = setTimeout(() => {
      onTocItemsChange?.(extractTocItems(tree))
    }, 200)
  }, [onTocItemsChange])

  // 加载文档
  useEffect(() => {
    if (!documentId) return
    let cancelled = false
    getDocument(documentId)
      .then((res) => {
        if (cancelled) return
        const blocks = res.blocks
        const title = (res.document.properties?.title as string) || ''
        const icon = res.document.properties?.icon as string | undefined
        setDoc({ status: 'loaded', docId: documentId, title, icon, tree: blocks })
        onTocItemsChange?.(extractTocItems(blocks))
      })
      .catch((err) => {
        if (!cancelled) setDoc({ status: 'error', docId: documentId, error: err })
      })
    return () => { cancelled = true }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [documentId])

  if (!documentId) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <div className="text-center">
          <p className="text-6xl mb-4">📝</p>
          <p className="text-lg">选择或创建一个文档开始编辑</p>
        </div>
      </div>
    )
  }

  if (doc.status === 'loading' || (doc.status === 'idle' && documentId)) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <p>加载中…</p>
      </div>
    )
  }

  if (doc.status === 'error') {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <p>加载失败</p>
      </div>
    )
  }

  if (doc.status !== 'loaded' || doc.docId !== documentId) {
    return (
      <div className="flex-1 flex items-center justify-center text-muted-foreground">
        <p>加载中…</p>
      </div>
    )
  }

  const { title, icon, tree } = doc

  return (
    <div className="flex-1 flex flex-col overflow-hidden bg-background rounded-tl-md shadow-[inset_1px_1px_3px_rgba(0,0,0,0.03)]">
      {onNavigate && documentId && (
        <Breadcrumb documentId={documentId} onNavigate={onNavigate} currentTitle={title} />
      )}

      <main className="flex-1 overflow-y-auto">
        <div className="max-w-3xl mx-auto px-8 py-12">
          <div className="mb-4 relative">
            <EmojiPicker value={icon} onChange={handleIconChange}>
              {({ onClick, ref }) => (
                <div
                  ref={ref}
                  onClick={onClick}
                  className="w-16 h-16 flex items-center justify-center text-4xl rounded-lg hover:bg-accent/50 transition-colors cursor-pointer"
                  title="点击更换图标"
                >
                  {icon || '📄'}
                </div>
              )}
            </EmojiPicker>
          </div>

          <h1
            className="text-3xl font-bold mb-8 outline-none"
            contentEditable
            suppressContentEditableWarning
            onBlur={handleTitleBlur}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                const firstBlock = document.querySelector('[data-block-id] [contenteditable]') as HTMLElement | null
                firstBlock?.focus()
              }
            }}
          >
            {title}
          </h1>

          <WemEditor
            blocks={tree}
            documentId={documentId!}
            placeholder="输入内容，或输入 / 插入块…"
            onTreeChange={handleTreeChange}
          />
        </div>
      </main>
    </div>
  )
}
